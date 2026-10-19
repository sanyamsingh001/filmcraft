//! #318: reverse Frame Sampling keeps a trimmed source range In-inclusive/Out-exclusive.
//! External ffmpeg generates original grayscale frames and supplies a literal trim/reverse oracle.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use filmcraft_engine::Session;
use filmcraft_media::FrameRequest;
use filmcraft_project::{ClipId, ItemId};
use filmcraft_render::RenderOptions;
use filmcraft_render::plan::{FramePlan, execute_cpu, plan_frame};
use filmcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

const WIDTH: usize = 32;
const HEIGHT: usize = 18;
const RGB_FRAME: usize = WIDTH * HEIGHT * 3;

fn exec(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, params).unwrap()
}

fn fixture(ff: &Path, fps: i64, ext: &str, coarse: bool) -> PathBuf {
    let suffix = if coarse { "_ts1000" } else { "" };
    let path = filmcraft_testkit::fixtures_dir("reverse318-supported-qp8-v1").join(format!("id12_{fps}{suffix}.{ext}"));
    let rgb: Vec<u8> = (0u8..12).flat_map(|n| std::iter::repeat_n(16 + 18 * n, RGB_FRAME)).collect();
    filmcraft_testkit::fixtures::generate(&path, |tmp| {
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
        if coarse {
            command.args(["-video_track_timescale", "1000"]);
        }
        let mut child = command.arg(tmp).stdin(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(&rgb).unwrap();
        child.wait().unwrap().success()
    })
    .expect("generate reverse-playback fixture")
}

fn oracle(ff: &Path, file: &Path, reverse: bool) -> Vec<u8> {
    let mut command = Command::new(ff);
    command.args(["-v", "error", "-i"]).arg(file).args(["-map", "0:v:0"]);
    if reverse {
        command.args(["-vf", "trim=start_frame=3:end_frame=9,reverse,setpts=PTS-STARTPTS"]);
    }
    let out = command.args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]).stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "ffmpeg oracle failed: {}", String::from_utf8_lossy(&out.stderr));
    let frames = if reverse { 6 } else { 12 };
    assert_eq!(out.stdout.len(), frames * RGB_FRAME, "oracle must preserve every frame");
    out.stdout
}

fn assert_pixels(rgba: &[u8], rgb: &[u8], context: &str) {
    assert_eq!(rgba.len(), WIDTH * HEIGHT * 4, "{context}: dimensions");
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
    assert!(worst <= 1, "{context}: max RGB difference {worst}; got {}, expected {}", rgba[0], rgb[0]);
}

fn check(fps: i64, ext: &str, coarse: bool, timeline_multiplier: i64) {
    let (reverse, forward): (&[usize], &[usize]) = if timeline_multiplier == 1 {
        (&[8, 7, 6, 5, 4, 3], &[3, 4, 5, 6, 7, 8])
    } else {
        (&[8, 8, 7, 7, 6, 6, 5, 5, 4, 4, 3, 3], &[3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8])
    };
    check_range(fps, ext, coarse, fps * timeline_multiplier, 3, 9, 1, false, reverse, forward);
}

// IDs are literal independently decoded fixture frames. A saved project may retain exact
// fractional source ranges even when the public placement command rounds its timeline duration.
#[allow(clippy::too_many_arguments)]
fn check_range(
    fps: i64,
    ext: &str,
    coarse: bool,
    timeline_fps: i64,
    in_numerator: i64,
    out_numerator: i64,
    range_denominator: i64,
    seed_exact_saved_range: bool,
    reverse_ids: &[usize],
    forward_ids: &[usize],
) {
    let Some(ff) = filmcraft_testkit::ffmpeg_or_skip("reverse_frame_sampling") else { return };
    let path = fixture(&ff, fps, ext, coarse);
    let all = oracle(&ff, &path, false);
    let reversed = oracle(&ff, &path, true);
    for (n, frame) in reversed.as_chunks::<RGB_FRAME>().0.iter().enumerate() {
        assert_eq!(frame, &all[(8 - n) * RGB_FRAME..(9 - n) * RGB_FRAME], "literal oracle includes In and excludes Out");
    }
    assert_eq!(reverse_ids.len(), forward_ids.len());
    let rate = FrameRate::new(fps, 1);
    let timeline_rate = FrameRate::new(timeline_fps, 1);
    let mut s = Session::default();
    exec(&mut s, "file.newSequence", json!({"width": WIDTH, "height": HEIGHT, "fps": timeline_fps, "video": 1, "audio": 0}));
    let imported = exec(&mut s, "file.import", json!({"paths": [path]}));
    assert_eq!(imported["errors"], json!([]));
    let item = ItemId(imported["items"][0].as_u64().unwrap());
    let src = s.source(item).unwrap();
    for n in 0..12 {
        let expected = &all[n * RGB_FRAME..(n + 1) * RGB_FRAME];
        for time in [rate.tick_of(n as i64), (rate.tick_of(n as i64) - Tick(1)).max(Tick::ZERO), rate.tick_of(n as i64) + Tick(rate.frame_duration().0 / 2)] {
            let frame = src.video_frame(FrameRequest::full(time)).unwrap();
            assert_pixels(&frame.to_rgba8(), expected, &format!("{fps} {ext} coarse={coarse} ordinary frame {n} time {}", time.0));
        }
    }
    let quarter_rate = FrameRate::new(fps * range_denominator, 1);
    let source_in = quarter_rate.tick_of(in_numerator);
    let requested_out = quarter_rate.tick_of(out_numerator);
    // Canonical endpoints matter at 31 fps: tick_of(out) - tick_of(in), not tick_of(out-in).
    let requested_duration = requested_out - source_in;
    let placed = exec(&mut s, "timeline.place", json!({"item": item.0, "track": "V1", "time": 0, "sourceIn": source_in.0, "duration": requested_duration.0}));
    let clip = ClipId(placed["clips"][0].as_u64().unwrap());
    let sequence = s.state.active_sequence.unwrap();
    assert_eq!(
        s.active_sequence().unwrap().find_item(clip).unwrap().1.duration,
        timeline_rate.snap_nearest(requested_duration).max(timeline_rate.frame_duration()),
        "public placement rounds only the timeline duration"
    );
    if seed_exact_saved_range {
        let project = std::sync::Arc::make_mut(&mut s.project);
        project.sequence_mut(sequence).unwrap().find_item_mut(clip).unwrap().1.duration = requested_duration;
    }
    for name in ["position", "anchor"] {
        exec(&mut s, "effects.setParam", json!({"clip": clip.0, "effect": "motion", "param": name, "value": [16, 9]}));
    }
    let before = s.project.clone();
    let original = s.active_sequence().unwrap().find_item(clip).unwrap().1.clone();
    let duration = original.duration;
    let source_out = original.source_out();
    assert_eq!(original.source_in, source_in);
    if seed_exact_saved_range {
        assert_eq!(source_out, requested_out);
    }
    assert!(timeline_rate.tick_of(reverse_ids.len() as i64 - 1) < duration, "last literal expectation is inside the clip");
    assert!(timeline_rate.tick_of(reverse_ids.len() as i64) >= duration, "literal expectations cover all visible frames");
    exec(&mut s, "clip.speedDuration", json!({"clips": [clip.0], "speed": 100, "reverse": true, "ripple": false, "interpolation": "frameSampling"}));
    let after = s.project.clone();
    let track_item = s.active_sequence().unwrap().find_item(clip).unwrap().1.clone();
    assert_eq!(track_item.source_in, source_in);
    assert_eq!(track_item.duration, duration);
    assert_eq!(track_item.source_out(), source_out);
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    for (frame, expected) in reverse_ids.iter().copied().enumerate() {
        let time = timeline_rate.tick_of(frame as i64);
        let want = &all[expected * RGB_FRAME..(expected + 1) * RGB_FRAME];
        let context = format!("{fps} {ext} coarse={coarse} timeline={timeline_fps} reverse frame {frame}");
        let clock = track_item.source_time_at(time);
        assert_eq!(clock, source_out - time - Tick(1), "continuous source clock");
        assert_eq!(track_item.effect_time_at(time), clock, "continuous effect clock");
        assert_pixels(&s.render_program_at(1.0, time).unwrap().to_rgba8(), want, &format!("{context} CPU"));
        let plan = plan_frame(&s.project, sequence, time, RenderOptions::default(), &provider);
        let FramePlan::Layers { layers, .. } = &plan else { panic!("{context}: expected GPU transport") };
        assert_eq!(layers.len(), 1);
        assert_pixels(&layers[0].frame.to_rgba8(), want, &format!("{context} GPU request transport"));
        assert_pixels(&execute_cpu(&plan).to_rgba8(), want, &format!("{context} GPU plan CPU reference"));
        if frame == 0 || frame == reverse_ids.len() - 1 {
            let export = filmcraft_testkit::fixtures_dir("reverse318-supported-qp8-v1").join(format!(
                "export_{fps}_{ext}_{coarse}_{timeline_fps}_{in_numerator}_{out_numerator}_{range_denominator}_{seed_exact_saved_range}_{frame}.png"
            ));
            exec(&mut s, "file.exportFrame", json!({"sequence": sequence.0, "time": time.0, "path": export, "format": "png", "import": false}));
            let png = image::open(&export).unwrap().to_rgba8();
            assert_eq!(png.dimensions(), (WIDTH as u32, HEIGHT as u32));
            assert_pixels(png.as_raw(), want, &format!("{context} Program PNG export"));
        }
    }
    exec(&mut s, "edit.undo", json!({}));
    assert_eq!(s.project, before, "reverse undo preserves the source range");
    for (frame, expected) in forward_ids.iter().copied().enumerate() {
        assert_pixels(
            &s.render_program_at(1.0, timeline_rate.tick_of(frame as i64)).unwrap().to_rgba8(),
            &all[expected * RGB_FRAME..(expected + 1) * RGB_FRAME],
            "forward after undo",
        );
    }
    exec(&mut s, "edit.redo", json!({}));
    assert_eq!(s.project, after, "reverse redo preserves the source range");
    assert_pixels(
        &s.render_program_at(1.0, Tick::ZERO).unwrap().to_rgba8(),
        &all[reverse_ids[0] * RGB_FRAME..(reverse_ids[0] + 1) * RGB_FRAME],
        "reverse after redo",
    );
    // An exact hold in a saved project keeps ordinary frame lookup even on a reversed clip.
    let project = std::sync::Arc::make_mut(&mut s.project);
    let held = project.sequence_mut(sequence).unwrap().find_item_mut(clip).unwrap().1;
    held.frame_hold = Some(rate.tick_of(5));
    held.hold_filters = true;
    for time in [Tick::ZERO, timeline_rate.tick_of(reverse_ids.len() as i64 - 1)] {
        assert_pixels(&s.render_program_at(1.0, time).unwrap().to_rgba8(), &all[5 * RGB_FRAME..6 * RGB_FRAME], "reverse held frame uses ordinary lookup");
    }
}

macro_rules! case {
    ($name:ident, $fps:literal, $ext:literal, $coarse:literal) => {
        #[test]
        fn $name() {
            check($fps, $ext, $coarse, 1);
            check($fps, $ext, $coarse, 2);
        }
    };
}

case!(reverse_trimmed_mp4_fps24, 24, "mp4", false);
case!(reverse_trimmed_mp4_fps24_ts1000, 24, "mp4", true);
case!(reverse_trimmed_mkv_fps24, 24, "mkv", false);
case!(reverse_trimmed_mp4_fps30, 30, "mp4", false);
case!(reverse_trimmed_mp4_fps30_ts1000, 30, "mp4", true);
case!(reverse_trimmed_mkv_fps30, 30, "mkv", false);
case!(reverse_trimmed_mp4_fps31, 31, "mp4", false);
case!(reverse_trimmed_mp4_fps31_ts1000, 31, "mp4", true);
case!(reverse_trimmed_mkv_fps31, 31, "mkv", false);

#[test]
fn reverse_fps20_on_timeline24_preserves_an_exact_saved_source_range() {
    for (ext, coarse) in [("mp4", false), ("mp4", true), ("mkv", false)] {
        check_range(20, ext, coarse, 24, 3, 9, 1, true, &[8, 8, 7, 6, 5, 4, 3, 3], &[3, 3, 4, 5, 6, 7, 8, 8]);
    }
}

#[test]
fn reverse_fps20_on_timeline24_keeps_the_public_placement_source_out() {
    for (ext, coarse) in [("mp4", false), ("mp4", true), ("mkv", false)] {
        check_range(20, ext, coarse, 24, 3, 9, 1, false, &[8, 7, 7, 6, 5, 4, 3], &[3, 3, 4, 5, 6, 7, 8]);
    }
}

#[test]
fn reverse_fractional_saved_source_range_keeps_partial_endpoint_frames() {
    for (ext, coarse) in [("mp4", false), ("mp4", true), ("mkv", false)] {
        check_range(31, ext, coarse, 62, 13, 35, 4, true, &[8, 8, 7, 7, 6, 6, 5, 5, 4, 4, 3], &[3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8]);
    }
}

#[test]
fn reverse_source_range_ending_at_the_last_coded_frame_reaches_in() {
    for (ext, coarse) in [("mp4", false), ("mp4", true), ("mkv", false)] {
        check_range(24, ext, coarse, 24, 3, 12, 1, false, &[11, 10, 9, 8, 7, 6, 5, 4, 3], &[3, 4, 5, 6, 7, 8, 9, 10, 11]);
    }
}
