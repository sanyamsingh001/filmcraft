//! Linux idle GOP retention through the production MP4/MKV source path.
#![cfg(target_os = "linux")]

mod common;

use common::{ffmpeg_frames, fixture, planes, raw_planes};
use filmcraft_media::{FrameRequest, SharedSource};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn open(path: &std::path::Path) -> SharedSource {
    let reader = Arc::new(filmcraft_media::reader::FileReader::open(path).unwrap());
    filmcraft_media::reader::open_reader(path.file_name().unwrap().to_str().unwrap(), reader, &filmcraft_codecs::reader_openers(), &filmcraft_codecs::openers())
        .unwrap()
}

#[test]
fn idle_sources_release_excess_decoders_without_new_frames_and_keep_cached_pictures() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let mp4 = fixture(
        &ff,
        "linux_idle_64x64.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x64:r=25:d=1",
            "-an",
            "-c:v",
            "libx264",
            "-threads",
            "1",
            "-bf",
            "0",
            "-refs",
            "1",
            "-g",
            "25",
            "-pix_fmt",
            "yuv420p",
        ],
    )
    .expect("small normal H.264 fixture is required");
    let mkv = fixture(&ff, "linux_idle_64x64.mkv", &["-i", mp4.to_str().unwrap(), "-c", "copy"]).expect("tiny MKV remux is required");
    let raw = ffmpeg_frames(&ff, &mp4, "yuv420p");
    let frame_bytes = 64 * 64 * 3 / 2;
    assert_eq!(raw.len(), 25 * frame_bytes);
    let oracle = |frame| raw_planes(&raw[frame * frame_bytes..(frame + 1) * frame_bytes], 64, 64, 32, 32, 1);
    for path in [&mp4, &mkv] {
        let sources: Vec<_> = (0..6).map(|_| open(path)).collect();
        let baseline = filmcraft_codecs::live_decoders();
        assert_eq!(baseline, 0, "fresh fixture begins with no decoders");
        let started = Instant::now();
        for source in &sources {
            let picture = source.video_frame(FrameRequest::full(source.info().frame_rate().tick_of(10))).unwrap();
            assert_eq!(planes(&picture), oracle(10));
        }
        assert!(started.elapsed() < Duration::from_secs(3), "six tiny source reads fit within the idle grace");
        assert_eq!(filmcraft_codecs::live_decoders(), baseline + 6, "real sources must share the production global pool");
        let decoded = filmcraft_codecs::gop_stats().decoded;
        let until = Instant::now() + Duration::from_secs(6);
        while filmcraft_codecs::live_decoders() > baseline + 4 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(filmcraft_codecs::live_decoders(), baseline + 4, "idle maintenance works without another source request");
        assert_eq!(filmcraft_codecs::gop_stats().decoded, decoded, "maintenance never decodes media");
        let picture = sources[0].video_frame(FrameRequest::full(sources[0].info().frame_rate().tick_of(10))).unwrap();
        assert_eq!(planes(&picture), oracle(10));
        assert_eq!(filmcraft_codecs::gop_stats().decoded, decoded, "cached request does not re-decode");
        assert_eq!(filmcraft_codecs::live_decoders(), baseline + 4, "cached requests retain the useful picture without restoring a decoder");
        let picture = sources[0].video_frame(FrameRequest::full(sources[0].info().frame_rate().tick_of(20))).unwrap();
        assert_eq!(planes(&picture), oracle(20));
        assert_eq!(filmcraft_codecs::live_decoders(), baseline + 4, "restarted source keeps the existing decoder cap");
        let weak: Vec<_> = sources.iter().map(Arc::downgrade).collect();
        drop(sources);
        assert!(weak.iter().all(|source| source.upgrade().is_none()), "maintenance cannot retain media source owners");
        let until = Instant::now() + Duration::from_secs(2);
        while filmcraft_codecs::live_decoders() != baseline && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(filmcraft_codecs::live_decoders(), baseline);
        println!(
            "{}: six native 64x64 sources shrink6→4 while idle; cached pictures and restarted frame20 remain ffmpeg-exact",
            path.extension().unwrap().to_str().unwrap()
        );
    }
}
