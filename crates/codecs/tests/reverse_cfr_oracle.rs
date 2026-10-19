//! #318 source-level oracle for validated CFR reverse lookup and unchanged fallback.
//! FFmpeg supplies encoded pictures and independent literal trim/reverse pixels. Our writers
//! retain that exact AVC configuration while stamping every sample at known millisecond PTS.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use filmcraft_frame::VideoFrame;
use filmcraft_isobmff::{Brand, CodecConfig, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
use filmcraft_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
use filmcraft_media::{FrameRequest, MediaSource};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};

const WIDTH: usize = 32;
const HEIGHT: usize = 18;
const RGB_FRAME: usize = WIDTH * HEIGHT * 3;
const VFR: [i64; 13] = [0, 40, 100, 120, 180, 220, 260, 310, 335, 375, 430, 480, 505];

#[derive(Clone, Copy, Debug)]
enum Stamp {
    Nearest,
    Floor,
    Vfr,
}

fn pts(fps: i64, stamp: Stamp) -> Vec<i64> {
    match stamp {
        Stamp::Nearest => (0..=12).map(|i| (2 * i * 1000 + fps) / (2 * fps)).collect(),
        Stamp::Floor => (0..=12).map(|i| i * 1000 / fps).collect(),
        Stamp::Vfr => VFR.to_vec(),
    }
}

fn encoded(ff: &Path, fps: i64, floor_encoder_clock: bool) -> PathBuf {
    let kind = if floor_encoder_clock { "floor_encoder_clock" } else { "normal_clock" };
    let path = filmcraft_testkit::fixtures_dir("reverse318-source-supported-qp8-v2").join(format!("id12_{fps}_{kind}.mp4"));
    let rgb: Vec<u8> = (0u8..12).flat_map(|n| std::iter::repeat_n(16 + 18 * n, RGB_FRAME)).collect();
    let made = filmcraft_testkit::fixtures::generate(&path, |tmp| {
        let mut command = Command::new(ff);
        command.args([
            "-y",
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgb24",
            "-video_size",
            "32x18",
            "-framerate",
            &fps.to_string(),
            "-i",
            "pipe:0",
            "-frames:v",
            "12",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-qp",
            "8",
            "-g",
            "12",
            "-bf",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ]);
        if floor_encoder_clock {
            command.args([
                "-vf",
                &format!("settb=1/1000,setpts=floor(N*1000/{fps})"),
                "-enc_time_base",
                "1:1000",
                "-fps_mode",
                "passthrough",
                "-video_track_timescale",
                "1000",
            ]);
        }
        let mut child = command.arg(tmp).stdin(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(&rgb).unwrap();
        child.wait().unwrap().success()
    })
    .expect("generate bounded source oracle fixture");
    assert_hint(&made, if floor_encoder_clock { 1000 } else { fps });
    made
}

fn assert_hint(path: &Path, expected_fps: i64) {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() <= 1 << 20, "bounded AVC fixture input");
    let file = filmcraft_isobmff::open(&bytes).unwrap();
    let track = file.tracks.iter().find(|track| track.kind == filmcraft_isobmff::TrackKind::Video).unwrap();
    let CodecConfig::Avc(config) = &track.entries.first().unwrap().codec else { panic!("AVC fixture") };
    assert!(!config.sps.is_empty());
    for nal in &config.sps {
        let rbsp = filmcraft_bitstream::unescape_rbsp(nal.get(1..).unwrap());
        let sps = filmcraft_h264::params::Sps::parse(&rbsp).unwrap();
        assert!(sps.frame_mbs_only, "progressive fixture");
        let (units, scale, _) = sps.vui.as_ref().and_then(|vui| vui.timing).expect("fixture has declarative timing");
        assert_eq!(
            FrameRate::new(i64::from(scale), i64::from(units) * 2),
            FrameRate::new(expected_fps, 1),
            "fixture must preserve its independently asserted AVC hint"
        );
    }
}

fn remux(base: &Path, fps: i64, stamp: Stamp, ext: &str) -> PathBuf {
    let input = std::fs::read(base).unwrap();
    assert!(input.len() <= 1 << 20, "bounded fixture input");
    let file = filmcraft_isobmff::open(&input).unwrap();
    let ti = file.tracks.iter().position(|track| track.kind == filmcraft_isobmff::TrackKind::Video).unwrap();
    let track = &file.tracks[ti];
    assert_eq!(track.samples.len(), 12);
    let entry = track.entries.first().unwrap().clone();
    let stamps = pts(fps, stamp);
    let path = filmcraft_testkit::fixtures_dir("reverse318-source-supported-qp8-v2").join(format!("id12_{fps}_{stamp:?}.{ext}"));
    filmcraft_testkit::fixtures::generate(&path, |tmp| {
        let data = if ext == "mp4" {
            let mut writer = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
            let target = writer.add_track(TrackConfig::new(entry.clone(), 1000)).unwrap();
            for i in 0..12 {
                let sample = file.read_sample(&input, ti, i).unwrap();
                writer
                    .write_sample(
                        target,
                        WriteSample { data: &sample, duration: (stamps[i + 1] - stamps[i]) as u32, composition_offset: 0, is_sync: track.samples[i].is_sync },
                    )
                    .unwrap();
            }
            writer.finish().unwrap().into_inner()
        } else {
            let CodecConfig::Avc(config) = &entry.codec else { panic!("AVC fixture") };
            let mut spec = TrackSpec::new(TrackKind::Video, "V_MPEG4/ISO/AVC");
            spec.codec_private = config.to_bytes();
            spec.video_size = Some((WIDTH as u32, HEIGHT as u32));
            spec.default_duration_ns = Some((1_000_000_000 / fps) as u64);
            let mut writer = MkvWriter::new(Cursor::new(Vec::new()), vec![spec], MuxOptions::default()).unwrap();
            for i in 0..12 {
                let sample = file.read_sample(&input, ti, i).unwrap();
                writer.write_frame(0, stamps[i] * 1_000_000, track.samples[i].is_sync, &sample, None).unwrap();
            }
            writer.finish().unwrap().into_inner()
        };
        std::fs::write(tmp, data).is_ok()
    })
    .expect("remux bounded source oracle fixture")
}

fn oracle(ff: &Path, path: &Path, reverse: bool) -> Vec<u8> {
    let mut command = Command::new(ff);
    command.args(["-v", "error", "-i"]).arg(path).args(["-map", "0:v:0"]);
    if reverse {
        command.args(["-vf", "trim=start_frame=3:end_frame=9,reverse,setpts=PTS-STARTPTS"]);
    }
    let out = command.args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]).stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "ffmpeg oracle: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.stdout.len(), if reverse { 6 } else { 12 } * RGB_FRAME);
    out.stdout
}

fn check_stamps(probe: &Path, path: &Path, expected: &[i64]) {
    let out = Command::new(probe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_packets", "-show_streams", "-show_entries", "packet=pts:stream=time_base", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffprobe: {}", String::from_utf8_lossy(&out.stderr));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["streams"][0]["time_base"].as_str(), Some("1/1000"), "oracle millisecond timestamp unit");
    let actual: Vec<i64> = value["packets"].as_array().unwrap().iter().map(|packet| packet["pts"].as_i64().unwrap()).collect();
    assert_eq!(actual, expected, "oracle must attest actual encoded millisecond PTS");
}

fn assert_pixels(frame: &VideoFrame, rgb: &[u8], context: &str) {
    assert_eq!((frame.width, frame.height), (WIDTH as u32, HEIGHT as u32), "{context}: dimensions");
    let rgba = frame.to_rgba8();
    assert_eq!(rgba.len(), WIDTH * HEIGHT * 4);
    assert_eq!(rgb.len(), RGB_FRAME);
    let worst = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(rgb.as_chunks::<3>().0.iter())
        .map(|(a, b)| {
            assert_eq!(a[3], 255, "{context}: opaque frame");
            a[..3].iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap()
        })
        .max()
        .unwrap();
    assert!(worst <= 1, "{context}: worst RGB difference {worst}; got {}, expected {}", rgba[0], rgb[0]);
}

fn open(path: &Path) -> filmcraft_media::SharedSource {
    filmcraft_codecs::open_bytes(path.to_str().unwrap(), Arc::from(std::fs::read(path).unwrap())).unwrap()
}

fn check_forward(src: &dyn MediaSource, fps: i64, all: &[u8]) {
    let rate = FrameRate::new(fps, 1);
    for id in 0..12 {
        for offset in [Tick::ZERO, Tick(1), Tick(rate.frame_duration().0 / 2)] {
            assert_pixels(
                &src.video_frame(FrameRequest::full(rate.tick_of(id) + offset)).unwrap(),
                &all[id as usize * RGB_FRAME..(id as usize + 1) * RGB_FRAME],
                &format!("forward {fps}fps frame {id} offset {offset:?}"),
            );
        }
    }
}

fn check_reverse(src: &dyn MediaSource, fps: i64, all: &[u8]) {
    let rate = FrameRate::new(fps, 1);
    for multiplier in [1, 2] {
        let timeline = FrameRate::new(fps * multiplier, 1);
        let literal: &[usize] = if multiplier == 1 { &[8, 7, 6, 5, 4, 3] } else { &[8, 8, 7, 7, 6, 6, 5, 5, 4, 4, 3, 3] };
        for (slot, &id) in literal.iter().enumerate() {
            let req = FrameRequest::full(rate.tick_of(9) - timeline.tick_of(slot as i64) - Tick(1));
            let ordinary = src.video_frame(req).unwrap().to_rgba8();
            assert_pixels(
                &src.video_frame_reverse(req).unwrap(),
                &all[id * RGB_FRAME..(id + 1) * RGB_FRAME],
                &format!("reverse {fps}fps x{multiplier} slot {slot}"),
            );
            assert_eq!(src.video_frame(req).unwrap().to_rgba8(), ordinary, "same-time ordinary/reverse cache entries must not alias");
        }
    }
    assert_pixels(
        &src.video_frame_reverse(FrameRequest::full(rate.tick_of(12) - Tick(1))).unwrap(),
        &all[11 * RGB_FRAME..12 * RGB_FRAME],
        "full coded Out stays on last frame",
    );
    assert_pixels(&src.video_frame_reverse(FrameRequest::full(Tick(-1))).unwrap(), &all[..RGB_FRAME], "negative source time clamps");
}

#[test]
fn exact_hint_nearest_and_floor_cfr_mp4_and_mkv_match_literal_reverse_pixels() {
    let (ff, probe) = filmcraft_testkit::require_oracles!();
    for fps in [24, 30, 31] {
        let base = encoded(&ff, fps, false);
        let original = oracle(&ff, &base, false);
        for id in 1..12 {
            assert_ne!(
                &original[(id - 1) * RGB_FRAME..id * RGB_FRAME],
                &original[id * RGB_FRAME..(id + 1) * RGB_FRAME],
                "all coded fixture pictures are distinct"
            );
        }
        for ext in ["mp4", "mkv"] {
            for stamp in [Stamp::Nearest, Stamp::Floor] {
                let path = remux(&base, fps, stamp, ext);
                check_stamps(&probe, &path, &pts(fps, stamp)[..12]);
                let all = oracle(&ff, &path, false);
                assert_eq!(all, original, "remux preserves all 12 distinct pictures");
                let literal = oracle(&ff, &path, true);
                for (slot, frame) in literal.as_chunks::<RGB_FRAME>().0.iter().enumerate() {
                    assert_eq!(frame, &all[(8 - slot) * RGB_FRAME..(9 - slot) * RGB_FRAME], "external reverse includes In and excludes Out");
                }
                let src = open(&path);
                check_forward(src.as_ref(), fps, &all);
                check_reverse(src.as_ref(), fps, &all);
                check_forward(src.as_ref(), fps, &all);
            }
        }
    }
}

#[test]
fn validated_reverse_preserves_mixed_rate_and_fractional_source_clocks() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    for fps in [20, 31] {
        let base = encoded(&ff, fps, false);
        for ext in ["mp4", "mkv"] {
            for stamp in [Stamp::Nearest, Stamp::Floor] {
                let path = remux(&base, fps, stamp, ext);
                let all = oracle(&ff, &path, false);
                let src = open(&path);
                let (out, timeline, literal): (Tick, FrameRate, &[usize]) = if fps == 20 {
                    (FrameRate::new(20, 1).tick_of(9), FrameRate::FPS_24, &[8, 8, 7, 6, 5, 4, 3, 3])
                } else {
                    (FrameRate::new(124, 1).tick_of(35), FrameRate::new(62, 1), &[8, 8, 7, 7, 6, 6, 5, 5, 4, 4, 3])
                };
                for (slot, &id) in literal.iter().enumerate() {
                    let req = FrameRequest::full(out - timeline.tick_of(slot as i64) - Tick(1));
                    assert_pixels(
                        &src.video_frame_reverse(req).unwrap(),
                        &all[id * RGB_FRAME..(id + 1) * RGB_FRAME],
                        &format!("mixed/fractional {fps}fps slot {slot}"),
                    );
                }
                if fps == 20 {
                    let rate = FrameRate::new(20, 1);
                    // Public placement snaps a six-source-frame span to seven 24fps frames.
                    let out = rate.tick_of(3) + FrameRate::FPS_24.tick_of(7);
                    for (slot, id) in [8usize, 7, 7, 6, 5, 4, 3].into_iter().enumerate() {
                        let req = FrameRequest::full(out - FrameRate::FPS_24.tick_of(slot as i64) - Tick(1));
                        assert_pixels(
                            &src.video_frame_reverse(req).unwrap(),
                            &all[id * RGB_FRAME..(id + 1) * RGB_FRAME],
                            "20-to-24 placement clock positive control",
                        );
                    }
                } else {
                    let rate = FrameRate::new(31, 1);
                    // The exact saved duration is tick(Out)-tick(In), not tick(Out-In).
                    let source_in = rate.tick_of(1);
                    let duration = rate.tick_of(2) - source_in;
                    assert_pixels(
                        &src.video_frame_reverse(FrameRequest::full(source_in + duration - Tick(1))).unwrap(),
                        &all[RGB_FRAME..2 * RGB_FRAME],
                        "canonical 31fps [1,2) exclusive boundary",
                    );
                }
                check_forward(src.as_ref(), fps, &all);
            }
        }
    }
}

#[test]
fn irregular_pts_with_exact_avc_hint_retain_ordinary_lookup() {
    let (ff, probe) = filmcraft_testkit::require_oracles!();
    for fps in [24, 30, 31] {
        let base = encoded(&ff, fps, false);
        for ext in ["mp4", "mkv"] {
            let path = remux(&base, fps, Stamp::Vfr, ext);
            check_stamps(&probe, &path, &VFR[..12]);
            let all = oracle(&ff, &path, false);
            let src = open(&path);
            for (id, &pts_ms) in VFR[..12].iter().enumerate() {
                let time = Tick(pts_ms * (TICKS_PER_SECOND / 1000));
                assert_pixels(
                    &src.video_frame(FrameRequest::full(time)).unwrap(),
                    &all[id * RGB_FRAME..(id + 1) * RGB_FRAME],
                    "VFR source-frame positive control",
                );
            }
            let rate = FrameRate::new(fps, 1);
            for slot in 0..12 {
                let req = FrameRequest::full(rate.tick_of(9) - rate.tick_of(slot) - Tick(1));
                let ordinary = src.video_frame(req).unwrap().to_rgba8();
                assert_eq!(src.video_frame_reverse(req).unwrap().to_rgba8(), ordinary, "VFR rejects logical cadence and preserves ordinary source pixels");
            }
        }
    }
}

#[test]
fn floor_stamped_mp4_without_matching_exact_hint_retains_ordinary_lookup() {
    let (ff, probe) = filmcraft_testkit::require_oracles!();
    for fps in [24, 30, 31] {
        let path = encoded(&ff, fps, true);
        check_stamps(&probe, &path, &pts(fps, Stamp::Floor)[..12]);
        let all = oracle(&ff, &path, false);
        let src = open(&path);
        check_forward(src.as_ref(), fps, &all);
        let rate = FrameRate::new(fps, 1);
        for slot in 0..6 {
            let req = FrameRequest::full(rate.tick_of(9) - rate.tick_of(slot) - Tick(1));
            let ordinary = src.video_frame(req).unwrap().to_rgba8();
            assert_eq!(src.video_frame_reverse(req).unwrap().to_rgba8(), ordinary, "unverified timing must retain ordinary source pixels");
        }
    }
}
