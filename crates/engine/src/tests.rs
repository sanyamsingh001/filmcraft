use super::*;

#[test]
fn invalid_preview_scales_are_rejected_without_allocating() {
    let mut s = Session::default();
    s.execute("file.newSequence", serde_json::json!({"width":160,"height":90})).unwrap();
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX] {
        assert!(s.render_program(scale).is_none());
        assert!(s.render_program_working(scale).is_none());
    }
    assert!(s.try_render_program_at(f32::NAN, Tick::ZERO).unwrap_err().to_string().contains("finite"), "the reason is reported");
    assert_eq!(s.render_program(0.5).unwrap().w, 80);
}
use serde_json::json;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn clip_ids(s: &Session, track: usize) -> Vec<u64> {
    s.active_sequence().unwrap().video_tracks[track].items.iter().map(|i| i.id.0).collect()
}

#[test]
fn demo_project_is_valid_and_renders() {
    let s = demo();
    let q = s.active_sequence().unwrap();
    q.check().unwrap();
    assert_eq!(q.video_tracks[0].items.len(), 6);
    assert!(q.duration() > Tick::ZERO);
    let img = s.render_program(0.125).unwrap();
    assert_eq!((img.w, img.h), (240, 135));
    assert!(img.px.chunks(4).any(|p| p[3] > 0.9));
}

/// #301: a rate inferred from imported media need not divide the tick timescale exactly.
#[test]
fn frame_steps_in_new_custom_rate_sequences_move_in_the_requested_direction() {
    for fps in [31.0, 23.98, 24.999] {
        let mut s = Session::default();
        s.execute("file.newSequence", json!({"fps": fps})).unwrap();
        let rate = s.sequence_rate();
        s.execute("playhead.set", json!({"frame": 10})).unwrap();
        assert_eq!(rate.frame_at(s.playhead()), 10);
        for frame in 11..=42 {
            s.execute("playhead.stepForward", json!({})).unwrap();
            assert_eq!(rate.frame_at(s.playhead()), frame);
        }
        for frame in (10..42).rev() {
            s.execute("playhead.stepBack", json!({})).unwrap();
            assert_eq!(rate.frame_at(s.playhead()), frame);
        }
        s.execute("prefs.set", json!({"key": "playback.stepManyFrames", "value": 12})).unwrap();
        s.execute("playhead.stepForward5", json!({})).unwrap();
        assert_eq!(rate.frame_at(s.playhead()), 22);
        s.execute("playhead.stepBack5", json!({})).unwrap();
        assert_eq!(rate.frame_at(s.playhead()), 10);
        s.execute("playhead.step", json!({"frames": -20})).unwrap();
        s.execute("playhead.stepBack", json!({})).unwrap();
        assert_eq!(s.playhead(), Tick::ZERO);
    }
}

#[test]
fn overflowing_frame_steps_are_errors_and_leave_the_playhead_unchanged() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"fps": 31.0})).unwrap();
    for start in [0, 10] {
        s.execute("playhead.set", json!({"frame": start})).unwrap();
        let before = s.playhead();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute("playhead.step", json!({"frames": i64::MAX}))));
        assert!(result.is_ok(), "an extreme frame offset must not panic");
        assert!(result.unwrap().is_err());
        assert_eq!(s.playhead(), before);
    }
    s.execute("playhead.step", json!({"frames": i64::MIN})).unwrap();
    assert_eq!(s.playhead(), Tick::ZERO);
}

#[test]
fn add_edit_undo_redo() {
    let mut s = demo();
    let before = clip_ids(&s, 0).len();
    s.execute("playhead.set", json!({"seconds": 2.0})).unwrap();
    s.execute("sequence.addEditAllTracks", json!({})).unwrap();
    assert_eq!(clip_ids(&s, 0).len(), before + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(clip_ids(&s, 0).len(), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(clip_ids(&s, 0).len(), before + 1);
    assert!(s.execute("edit.redo", json!({})).is_err(), "disabled when nothing to redo");
}

#[test]
fn source_insert_three_point() {
    let mut s = demo();
    let item = s
        .project
        .root
        .children
        .iter()
        .find_map(|c| {
            if let filmcraft_project::BinEntry::Bin(b) = c {
                b.children.first().and_then(|e| if let filmcraft_project::BinEntry::Item(i) = e { Some(*i) } else { None })
            } else {
                None
            }
        })
        .unwrap();
    s.execute("source.open", json!({"item": item.0})).unwrap();
    let r = s.sequence_rate();
    s.execute("project.setMarks", json!({"item": item.0, "in": r.tick_of(24).0, "out": r.tick_of(47).0})).unwrap();
    s.execute("playhead.set", json!({"frame": 0})).unwrap();
    let dur0 = s.active_sequence().unwrap().duration();
    s.execute("source.insert", json!({})).unwrap();
    let dur1 = s.active_sequence().unwrap().duration();
    assert_eq!(dur1 - dur0, r.tick_of(24), "one second inserted, sequence rippled");
    assert_eq!(s.playhead(), r.tick_of(24), "playhead moves to end of edit");
    s.active_sequence().unwrap().check().unwrap();
}

#[test]
fn trim_linked_and_ripple_delete() {
    let mut s = demo();
    let first = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
    s.execute("timeline.trim", json!({"clip": first.id.0, "edge": "out", "mode": "regular", "deltaFrames": -12})).unwrap();
    let q = s.active_sequence().unwrap();
    assert_eq!(q.video_tracks[0].items[0].duration, first.duration - s.sequence_rate().tick_of(12));
    let a = q.audio_tracks[0].items.iter().find(|i| i.link == first.link).unwrap();
    assert_eq!(a.duration, q.video_tracks[0].items[0].duration, "linked audio trimmed too");
    s.execute("timeline.select", json!({"clips": [first.id.0]})).unwrap();
    assert_eq!(s.state.selection.len(), 2, "linked selection adds audio");
    assert!(s.execute("edit.rippleDelete", json!({})).is_err(), "music on sync-locked A2 blocks the ripple");
    s.execute("timeline.setTrack", json!({"track": "A2", "syncLock": false})).unwrap();
    s.execute("edit.rippleDelete", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    assert_eq!(q.video_tracks[0].items[0].start, s.sequence_rate().tick_of(12), "the trim gap remains");
}

#[test]
fn slide_moves_linked_audio_with_the_video() {
    let mut s = demo();
    let v = s.active_sequence().unwrap().video_tracks[0].items[1].clone();
    s.execute("timeline.slide", json!({"clip": v.id.0, "deltaFrames": 5})).unwrap();
    let q = s.active_sequence().unwrap();
    let (_, nv) = q.find_item(v.id).unwrap();
    let a = q.audio_tracks[0].items.iter().find(|i| i.link == v.link).unwrap();
    assert_eq!(nv.start, v.start + s.sequence_rate().tick_of(5));
    assert_eq!((a.start, a.source_in), (nv.start, nv.source_in), "linked audio slides too and stays in sync");
}

#[test]
fn sequence_parameters_are_bounded_and_failed_changes_are_atomic() {
    let mut s = demo();
    let before = (*s.project).clone();
    let history = s.history.undo.len();
    for command in ["file.newSequence", "sequence.settings"] {
        for params in [
            json!({"width":0}),
            json!({"height":0}),
            json!({"sampleRate":0}),
            json!({"width":4_294_967_360_u64}),
            json!({"height":u64::MAX}),
            json!({"sampleRate":u64::MAX}),
            json!({"width":-1}),
            json!({"width":1.5}),
            json!({"width":32768,"height":16384}),
            json!({"width":1920.25}),
            json!({"fps":1001}),
            json!({"sampleRate":384001}),
        ] {
            assert!(s.execute(command, params.clone()).is_err(), "{command} {params}");
            assert_eq!(*s.project, before);
            assert_eq!(s.history.undo.len(), history);
        }
    }
    for params in [json!({"video":u64::MAX}), json!({"audio":u64::MAX}), json!({"video":257}), json!({"audio":-1})] {
        assert!(s.execute("file.newSequence", params.clone()).is_err(), "{params}");
        assert_eq!(*s.project, before);
    }
    s.execute("file.newSequence", json!({"width":7680,"height":4320,"video":0,"audio":0})).unwrap();
    assert_eq!(s.active_sequence().unwrap().settings.width, 7680, "standard 8K remains supported");
    s.execute("file.newSequence", json!({"width":15360.0,"height":8640.0,"sampleRate":48000.0,"video":1.0,"audio":0})).unwrap();
    assert_eq!(s.active_sequence().unwrap().settings.width, 15360, "16K, and integer-valued floats, are accepted");
    s.execute("sequence.settings", json!({"width":16384,"height":8192})).unwrap();
    assert_eq!(s.active_sequence().unwrap().settings.height, 8192, "a 16384x8192 panorama is accepted");
}

#[test]
fn zero_fps_is_refused() {
    let mut s = demo();
    let rate = s.sequence_rate();
    assert!(s.execute("file.newSequence", json!({"name": "z", "fps": 0})).is_err());
    assert!(s.execute("sequence.settings", json!({"fps": 0})).is_err());
    assert!(s.execute("sequence.settings", json!({"fps": -24})).is_err());
    assert_eq!(s.sequence_rate(), rate, "the active sequence keeps its rate");
    s.execute("markers.markIn", json!({"time": 0})).unwrap();
}

#[test]
fn effects_and_keyframes() {
    let mut s = demo();
    let c = s.active_sequence().unwrap().video_tracks[0].items[1].id.0;
    s.execute("effects.apply", json!({"clips": [c], "effect": "Gaussian Blur"})).unwrap();
    s.execute("effects.setParam", json!({"clip": c, "effect": "gaussian_blur", "param": "blurriness", "value": 25.0})).unwrap();
    let q = s.active_sequence().unwrap();
    let it = q.find_item(filmcraft_project::ClipId(c)).unwrap().1;
    assert_eq!(it.effects[0].effect, "gaussian_blur", "standard effects render before intrinsics");
    assert_eq!(it.effects[0].params["blurriness"].value, filmcraft_project::ParamValue::Float(25.0));
    s.execute("effects.toggleAnimation", json!({"clip": c, "effect": "motion", "param": "scale"})).unwrap();
    let it = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(c)).unwrap().1.clone();
    assert!(it.effect("motion").unwrap().params["scale"].is_animated());
}

#[test]
fn transitions_and_markers() {
    let mut s = demo();
    let q = s.active_sequence().unwrap();
    let cut = q.video_tracks[0].items[2].start;
    s.execute("playhead.set", json!({"time": cut.0})).unwrap();
    s.execute("sequence.applyVideoTransition", json!({"effect": "wipe"})).unwrap();
    let q = s.active_sequence().unwrap();
    assert!(q.video_tracks[0].transitions.iter().any(|t| t.effect.effect == "wipe"));
    let n = q.markers.len();
    s.execute("markers.add", json!({"name": "Test"})).unwrap();
    assert_eq!(s.active_sequence().unwrap().markers.len(), n + 1);
}

#[test]
fn every_video_transition_applies_via_command() {
    let mut s = demo();
    let cut = s.active_sequence().unwrap().video_tracks[0].items[2].start;
    s.execute("playhead.set", json!({"time": cut.0})).unwrap();
    let defs: Vec<_> =
        filmcraft_project::effect_defs().iter().filter(|d| d.kind == filmcraft_project::EffectKind::VideoTransition && d.category[0] != "Obsolete").collect();
    assert_eq!(defs.len(), 84 + 21);
    for (i, d) in defs.iter().enumerate() {
        // alternate id and display-name lookups
        let name = if i % 2 == 0 { d.id } else { d.name };
        let r = s.execute("sequence.applyVideoTransition", json!({"effect": name})).unwrap_or_else(|e| panic!("{}: {e}", d.id));
        let q = s.active_sequence().unwrap();
        let tr = q.video_tracks[0].transitions.iter().find(|t| t.id.0 == r["transition"].as_u64().unwrap()).unwrap();
        assert_eq!(tr.effect.effect, d.id);
        // rendering the middle of the transition works through the engine
        let mid = tr.start + tr.duration.mul_ratio(1, 2);
        s.execute("playhead.set", json!({"time": mid.0})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active_sequence().unwrap().video_tracks[0].transitions.iter().all(|t| t.id.0 != r["transition"].as_u64().unwrap()));
        s.execute("playhead.set", json!({"time": cut.0})).unwrap();
    }
    // names shared with video effects resolve to the transition here
    let r = s.execute("sequence.applyVideoTransition", json!({"effect": "Mosaic"})).unwrap();
    let q = s.active_sequence().unwrap();
    assert!(q.video_tracks[0].transitions.iter().any(|t| t.id.0 == r["transition"].as_u64().unwrap() && t.effect.effect == "mosaic_transition"));
    assert!(s.execute("sequence.applyVideoTransition", json!({"effect": "Constant Power"})).is_err());
}

#[test]
fn transition_params_and_reverse_via_commands() {
    let mut s = demo();
    let cut = s.active_sequence().unwrap().video_tracks[0].items[2].start;
    s.execute("playhead.set", json!({"time": cut.0})).unwrap();
    let r = s
        .execute(
            "sequence.applyVideoTransition",
            json!({"effect": "Iris Round", "params": {"border_width": 12.0, "border_color": "#ff0000", "antialias": "High"}, "reverse": true}),
        )
        .unwrap();
    let id = r["transition"].as_u64().unwrap();
    let find = |s: &Session| s.active_sequence().unwrap().video_tracks[0].transitions.iter().find(|t| t.id.0 == id).cloned().unwrap();
    let tr = find(&s);
    assert!(tr.reverse);
    assert_eq!(tr.effect.params["border_width"].value, filmcraft_project::ParamValue::Float(12.0));
    assert_eq!(tr.effect.params["antialias"].value, filmcraft_project::ParamValue::Choice(3));
    s.execute("sequence.setTransition", json!({"transition": id, "params": {"center": [100.0, 50.0], "border_width": 9999.0}, "reverse": false})).unwrap();
    let tr = find(&s);
    assert!(!tr.reverse);
    assert_eq!(tr.effect.params["border_width"].value, filmcraft_project::ParamValue::Float(200.0), "clamped to the range");
    assert!(s.execute("sequence.setTransition", json!({"transition": id, "params": {"nope": 1}})).is_err());
    assert!(s.execute("sequence.setTransition", json!({"transition": id, "params": {"antialias": "Ultra"}})).is_err());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(find(&s).reverse, "setTransition is one undo step");
    s.execute("sequence.setTransition", json!({"transition": id, "reset": true})).unwrap();
    assert_eq!(find(&s).effect.params["border_width"].value, filmcraft_project::ParamValue::Float(0.0));
    let seq = s.execute("sequence.inspect", json!({})).unwrap();
    let js = seq["video"][0]["transitions"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap().clone();
    assert_eq!(js["effect"], "iris_round");
    assert!(js["params"]["border_width"].is_object() || js["params"]["border_width"].is_number(), "{js}");
}

#[test]
fn effects_list_reports_transition_folders() {
    let s = demo();
    let mut s = s;
    let all = s.execute("effects.list", json!({"kind": "VideoTransition"})).unwrap();
    let all = all.as_array().unwrap();
    assert!(all.iter().all(|e| e["kind"] == "VideoTransition" && e["folder"].is_string()));
    let wipes = s.execute("effects.list", json!({"folder": "Video Transitions/Wipe", "detail": true})).unwrap();
    let wipes = wipes.as_array().unwrap();
    assert_eq!(wipes.len(), 9);
    let lw = wipes.iter().find(|e| e["id"] == "linear_wipe").unwrap();
    assert_eq!(lw["folder"], "Wipe");
    assert_eq!(lw["path"], "Video Transitions/Wipe");
    let aa = lw["paramInfo"].as_array().unwrap().iter().find(|p| p["id"] == "antialias").unwrap();
    assert_eq!(aa["type"], "choice");
    assert_eq!(aa["options"].as_array().unwrap().len(), 4);
    let legacy = s.execute("effects.list", json!({"folder": "Legacy/Video Transitions"})).unwrap();
    assert_eq!(legacy.as_array().unwrap().len(), 21);
    // the default transition accepts names shared with video effects
    s.execute("effects.setDefaultTransition", json!({"effect": "Mosaic"})).unwrap();
    assert_eq!(s.state.default_video_transition, "mosaic_transition");
}

#[test]
fn commands_are_unique_and_described() {
    let mut ids = std::collections::HashSet::new();
    for c in command_specs() {
        assert!(ids.insert(c.id), "duplicate {}", c.id);
    }
    assert!(command_specs().len() > 90, "{}", command_specs().len());
    let s = demo();
    let list = commands::inspect_project(&s);
    assert!(list["root"]["children"].as_array().unwrap().len() >= 4);
}

#[test]
fn save_and_open_roundtrip() {
    let dir = std::env::temp_dir().join(format!("fc-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("demo.fcproj").to_string_lossy().to_string();
    let mut s = demo();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    assert!(!s.is_dirty());
    let mut t = Session::default();
    t.execute("file.open", json!({"path": path})).unwrap();
    assert_eq!(*t.project, *s.project);
    // media re-created from generator refs
    assert!(t.render_program(0.1).is_some());
    std::fs::remove_dir_all(dir).ok();
}

/// Demo project with sync lock off on every track except V1/A1 (the A2 score spans every cut).
fn demo_unlocked() -> Session {
    let mut s = demo();
    for t in ["V2", "V3", "A2", "A3"] {
        let _ = s.execute("timeline.setTrack", json!({"track": t, "syncLock": false}));
    }
    s
}

fn v1(s: &Session) -> Vec<(u64, Tick, Tick)> {
    s.active_sequence().unwrap().video_tracks[0].items.iter().map(|i| (i.id.0, i.start, i.end())).collect()
}

#[test]
fn trim_mode_roll_ripple_and_toggle() {
    let mut s = demo_unlocked();
    let rate = s.sequence_rate();
    let clips = v1(&s);
    // park near the first V1 cut and pick it as a roll
    let cut = clips[0].2;
    s.execute("playhead.set", json!({"time": (cut + rate.tick_of(2)).0})).unwrap();
    let r = s.execute("trim.selectNearest", json!({"kind": "roll"})).unwrap();
    assert!(!r["editPoints"].as_array().unwrap().is_empty());
    let dur_before = s.active_sequence().unwrap().duration();
    s.execute("trim.forwardMany", json!({})).unwrap();
    let after = v1(&s);
    let left = after.iter().find(|c| c.0 == clips[0].0).unwrap();
    assert_eq!(left.2, cut + rate.tick_of(5), "roll moved the cut 5 frames later");
    assert_eq!(s.active_sequence().unwrap().duration(), dur_before, "roll keeps the duration");
    // toggle Roll -> Trim -> Ripple; ripple backward shortens the sequence
    s.execute("trim.toggleType", json!({})).unwrap();
    s.execute("trim.toggleType", json!({})).unwrap();
    assert_eq!(s.state.edit_points[0].kind, crate::trim::TrimKind::Ripple);
    let d0 = s.active_sequence().unwrap().duration();
    s.execute("trim.backward", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().duration() <= d0, "ripple trim back must not lengthen");
    s.execute("trim.clear", json!({})).unwrap();
    assert!(s.execute("trim.forward", json!({})).is_err(), "disabled without edit points");
}

#[test]
fn ripple_trim_to_playhead_q_w() {
    let mut s = demo_unlocked();
    let rate = s.sequence_rate();
    let clips = v1(&s);
    let (id, start, end) = clips[1];
    let d0 = s.active_sequence().unwrap().duration();
    // W: ripple trim the end of the clip under the playhead to the playhead
    let ph = start + rate.tick_of(10);
    s.execute("playhead.set", json!({"time": ph.0})).unwrap();
    s.execute("trim.rippleNext", json!({})).unwrap();
    let c = v1(&s).into_iter().find(|c| c.0 == id).unwrap();
    assert_eq!(c.2, ph, "clip now ends at the playhead");
    assert_eq!(s.active_sequence().unwrap().duration(), d0 - (end - ph), "later material rippled left");
    // Q: ripple trim the start of the (new) clip under the playhead to the playhead
    let (_, start2, end2) = v1(&s)[2];
    let d1 = s.active_sequence().unwrap().duration();
    let ph2 = start2 + rate.tick_of(6);
    s.execute("playhead.set", json!({"time": ph2.0})).unwrap();
    s.execute("trim.ripplePrevious", json!({})).unwrap();
    let c2 = v1(&s).into_iter().find(|c| c.1 == start2).expect("a clip still starts at the old edit");
    assert_eq!(c2.2, end2 - rate.tick_of(6), "its head (6 frames) was removed");
    assert_eq!(s.active_sequence().unwrap().duration(), d1 - rate.tick_of(6));
    assert_eq!(s.playhead(), start2, "playhead parks on the new edit");
}

#[test]
fn interchange_roundtrip_through_files() {
    let dir = std::env::temp_dir().join(format!("fc-ix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = demo();
    let n0 = clip_ids(&s, 0).len();
    for (cmd, ext) in [("file.exportFcp7Xml", "xml"), ("file.exportOtio", "otio"), ("file.exportFcpxml", "fcpxml"), ("file.exportEdl", "edl")] {
        let path = dir.join(format!("demo.{ext}")).to_string_lossy().to_string();
        s.execute(cmd, json!({"path": path})).unwrap();
        let r = s.execute("file.import", json!({"paths": [path]})).unwrap();
        let seq = r["sequences"][0].as_u64().unwrap_or_else(|| panic!("{ext}: no sequence in {r}"));
        assert_eq!(s.state.active_sequence, Some(ItemId(seq)), "{ext}: imported sequence is opened");
        // FCP7 XML and OTIO carry our generator (synthetic) media; FCPXML/EDL only file media.
        if matches!(ext, "xml" | "otio") {
            assert_eq!(clip_ids(&s, 0).len(), n0, "{ext}: V1 clip count survives the round trip");
        }
        s.execute("sequence.open", json!({"item": s.state.open_sequences[0].0})).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `file.exportInterchange {format: "<unknown>"}` used to write FCP7 XML without a word.
#[test]
fn export_interchange_refuses_an_unknown_format() {
    let d = std::env::temp_dir().join(format!("fc-ix-format-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let path = |n: &str| d.join(n).to_string_lossy().to_string();
    let mut s = demo();
    // unknown names, values that are not names, and spellings that are not the documented ones
    for bad in [json!("premiere"), json!("mp4"), json!(""), json!(7), json!(["xml"]), json!(".xml"), json!("fcpxmld"), json!("omfi"), json!("xml ")] {
        let e = s.execute("file.exportInterchange", json!({"format": bad, "path": path("bad.out")})).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { cmd, .. } if cmd == "file.exportInterchange"), "{bad}: {e:?}");
        let e = e.to_string();
        assert!(e.contains("unknown format") && e.ends_with("use one of edl, xml, fcpxml, otio, aaf, omf"), "the error lists exactly the accepted names: {e}");
        assert!(!d.join("bad.out").exists(), "nothing is written for {bad}");
    }
    // the documented formats still export (in either case), and no format still means FCP7 XML
    let text = |n: &str| String::from_utf8_lossy(&std::fs::read(d.join(n)).unwrap()).into_owned();
    s.execute("file.exportInterchange", json!({"path": path("default.xml")})).unwrap();
    s.execute("file.exportInterchange", json!({"format": "xml", "path": path("a.xml")})).unwrap();
    assert!(text("default.xml").contains("<xmeml") && text("a.xml").contains("<xmeml"));
    s.execute("file.exportInterchange", json!({"format": "fcpxml", "path": path("a.fcpxml")})).unwrap();
    assert!(text("a.fcpxml").contains("<fcpxml"));
    s.execute("file.exportInterchange", json!({"format": "otio", "path": path("a.otio")})).unwrap();
    assert!(text("a.otio").contains("OTIO_SCHEMA"));
    s.execute("file.exportInterchange", json!({"format": "EDL", "path": path("a.edl")})).unwrap();
    assert!(text("a.edl").contains("TITLE:"));
    for (format, magic) in [("aaf", "aaf"), ("omf", "omf")] {
        let r = s.execute("file.exportInterchange", json!({"format": format, "path": path(&format!("a.{magic}"))})).unwrap();
        assert!(r["bytes"].as_u64().unwrap() > 0, "{format}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn move_items_to_bin_and_back_with_undo() {
    let mut s = demo();
    let bin = s.execute("file.newBin", json!({"name": "Selects"})).unwrap()["bin"].as_u64().unwrap();
    let mut items = Vec::new();
    s.project.root.all_items(&mut items);
    let item = items[0];
    let home = s.project.root.parent_of(item);
    let r = s.execute("project.moveToBin", json!({"items": [item.0], "bin": bin})).unwrap();
    assert_eq!(r["moved"], 1);
    assert_eq!(s.project.root.parent_of(item), Some(filmcraft_project::BinId(bin)));
    // the item is in the tree exactly once
    let mut all = Vec::new();
    s.project.root.all_items(&mut all);
    assert_eq!(all.iter().filter(|i| **i == item).count(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.root.parent_of(item), home);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project.root.parent_of(item), Some(filmcraft_project::BinId(bin)));
    // back to the root with `bin: null`
    s.execute("project.moveToBin", json!({"items": [item.0], "bin": null})).unwrap();
    assert_eq!(s.project.root.parent_of(item), Some(s.project.root.id));
    // an unknown bin is an error, and so is no items and no selection
    assert!(s.execute("project.moveToBin", json!({"items": [item.0], "bin": 999_999})).is_err());
    s.state.project_selection.clear();
    assert!(s.execute("project.moveToBin", json!({})).is_err());
}

#[test]
fn fps_validation_keeps_real_rates_and_refuses_degenerate_ones() {
    let mut s = demo();
    // real rates still work, through both commands
    for fps in [23.976, 25.0, 29.97, 59.94, 120.0] {
        s.execute("file.newSequence", json!({"name": format!("r{fps}"), "fps": fps})).unwrap_or_else(|e| panic!("{fps}: {e}"));
        let r = s.sequence_rate();
        assert!((r.as_f64() - fps).abs() < 0.01, "{fps} -> {}", r.as_f64());
        s.execute("sequence.settings", json!({"fps": 24})).unwrap();
        assert!((s.sequence_rate().as_f64() - 24.0).abs() < 1e-9);
    }
    // a rate that rounds to nothing is refused, and nothing changes
    let before = s.sequence_rate();
    let n = s.execute("project.inspect", json!({})).unwrap().to_string().len();
    assert!(s.execute("file.newSequence", json!({"name": "tiny", "fps": 1e-12})).is_err());
    assert!(s.execute("sequence.settings", json!({"fps": -0.0})).is_err());
    assert_eq!(s.sequence_rate(), before);
    assert_eq!(s.execute("project.inspect", json!({})).unwrap().to_string().len(), n, "no sequence was created");
}

/// #164: with a clip selected, Q/W trim only its tracks (and its linked sound), not every
/// targeted track under the playhead (the music on A2 used to be cut too).
#[test]
fn ripple_trim_to_playhead_follows_the_selection() {
    let mut s = demo_unlocked();
    let rate = s.sequence_rate();
    let a2 = |s: &Session| s.active_sequence().unwrap().audio_tracks[1].items.iter().map(|i| (i.start, i.duration)).collect::<Vec<_>>();
    let music = a2(&s);
    assert!(!music.is_empty(), "the demo has music on A2");
    let (id, start, _) = v1(&s)[1];
    s.execute("timeline.select", json!({"clips": [id]})).unwrap();
    let ph = start + rate.tick_of(10);
    s.execute("playhead.set", json!({"time": ph.0})).unwrap();
    s.execute("trim.rippleNext", json!({})).unwrap();
    assert_eq!(v1(&s).into_iter().find(|c| c.0 == id).unwrap().2, ph, "the selected clip ends at the playhead");
    assert_eq!(a2(&s), music, "the music on A2 is untouched");
    let linked = s.active_sequence().unwrap().audio_tracks[0].items.iter().find(|i| i.start == start).map(|i| i.end());
    assert_eq!(linked, Some(ph), "its linked sound on A1 is trimmed with it");
}

#[test]
fn empty_items_do_not_panic_keyframe_commands() {
    let mut s = demo();
    let seq = s.state.active_sequence.unwrap();
    let clip = {
        let item = &mut std::sync::Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap().video_tracks[0].items[0];
        item.duration = Tick::ZERO;
        item.id
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        s.execute("effects.toggleAnimation", json!({"clip":clip.0,"effect":"motion","param":"position"}))
    }));
    assert!(result.is_ok(), "an empty clip must not supply reversed clamp bounds");
    let error = result.unwrap().unwrap_err();
    assert!(error.to_string().contains("non-positive duration"), "expected the ordinary invariant error, got: {error}");
}

/// #201: a drag of an effect parameter (`merge`, starting with `begin`) is one undo step, and the
/// next drag is another.
#[test]
fn dragging_an_effect_parameter_is_one_undo_step() {
    let mut s = demo_unlocked();
    let id = v1(&s)[0].0;
    let opacity = |s: &Session| {
        let q = s.active_sequence().unwrap();
        let it = q.find_item(ClipId(id)).unwrap().1;
        let e = it.effects.iter().find(|e| e.effect == "opacity").unwrap();
        e.params["opacity"].value.as_f64().unwrap()
    };
    let start = opacity(&s);
    let set = |s: &mut Session, v: f64, begin: bool| {
        s.execute("effects.setParam", json!({"clip": id, "effect": "opacity", "param": "opacity", "value": v, "merge": true, "begin": begin})).unwrap();
    };
    set(&mut s, 90.0, true);
    set(&mut s, 70.0, false);
    set(&mut s, 30.0, false);
    set(&mut s, 50.0, true); // a second drag
    set(&mut s, 60.0, false);
    assert_eq!(opacity(&s), 60.0);
    s.undo();
    assert_eq!(opacity(&s), 30.0, "undo takes back the whole second drag");
    s.undo();
    assert_eq!(opacity(&s), start, "and then the whole first one");
}

fn source_endpoint_fixture(rate: FrameRate, end: Tick, audio_only: bool) -> (Session, filmcraft_project::ItemId) {
    let mut session = Session::default();
    session.execute("file.newSequence", json!({"width":32,"height":18,"fps":24,"video":1,"audio":0})).unwrap();
    session.execute("playhead.set", json!({"frame":13})).unwrap();
    let pool = session.media.clone();
    let item = crate::demo::add_generator(
        Arc::make_mut(&mut session.project),
        &pool,
        filmcraft_media::generators::GeneratorSource::new(filmcraft_media::Generator::BarsAndTone, 32, 18, rate, end),
        "Source endpoint",
        filmcraft_project::Label::Iris,
        None,
    );
    if audio_only {
        let filmcraft_project::ItemKind::Media(m) = &mut Arc::make_mut(&mut session.project).item_mut(item).unwrap().kind else { unreachable!() };
        m.info.video = None;
        m.info.kind = filmcraft_media::MediaKind::AudioOnly;
    }
    session.execute("source.open", json!({"item":item.0})).unwrap();
    (session, item)
}

#[test]
fn source_seek_keeps_the_last_frame_of_exact_coarse_fractional_and_short_spans() {
    let r31 = FrameRate::new(31, 1);
    let audio_rate = FrameRate::default();
    for (rate, end, last, audio) in [
        (r31, r31.tick_of(41), 40, false),
        // Real generated41-frame31fps MKV duration, independently decoded by FFmpeg/native before.
        (r31, Tick(335_809_152_000), 40, false),
        (r31, r31.tick_of(3) + Tick(1), 3, false),
        (r31, Tick(1), 0, false),
        (audio_rate, audio_rate.tick_of(3) + Tick(1), 3, true),
    ] {
        let (mut s, item) = source_endpoint_fixture(rate, end, audio);
        let project = s.project.clone();
        let history = s.history.undo.len();
        let program = s.playhead();
        s.execute("source.setPlayhead", json!({"frame":last})).unwrap();
        assert_eq!(s.state.source_playhead, rate.tick_of(last));
        s.execute("source.setPlayhead", json!({"time":i64::MAX})).unwrap();
        assert_eq!(s.state.source_playhead, rate.tick_of(last), "last partial frame must remain reachable");
        assert!(s.state.source_playhead < end);
        s.execute("source.setPlayhead", json!({"time":-1})).unwrap();
        assert_eq!(s.state.source_playhead, Tick::ZERO);
        assert_eq!(s.playhead(), program);
        assert_eq!(s.project, project);
        assert_eq!(s.history.undo.len(), history);
        assert_eq!(crate::clip_ops::source_view(&s, item).unwrap().end, end);
    }
}

#[test]
fn fractional_subclip_source_bounds_and_derived_out_keep_the_final_partial_frame() {
    let rate = FrameRate::new(31, 1);
    let end = Tick(335_809_152_000);
    let start = rate.tick_of(10) + Tick(1);
    for restrict in [true, false] {
        let (mut s, parent) = source_endpoint_fixture(rate, end, false);
        let item =
            s.execute("clip.makeSubclip", json!({"item":parent.0,"start":start.0,"end":end.0,"restrictTrims":restrict})).unwrap()["item"].as_u64().unwrap();
        s.execute("source.open", json!({"item":item})).unwrap();
        let view = crate::clip_ops::source_view(&s, filmcraft_project::ItemId(item)).unwrap();
        assert_eq!(view.subclip, Some((filmcraft_time::TimeRange::from_bounds(start, end), restrict)));
        assert_eq!(view.mark_out, if restrict { None } else { Some(rate.tick_of(40)) });
        let project = s.project.clone();
        let program = s.playhead();
        s.execute("source.setPlayhead", json!({"frame":40})).unwrap();
        assert_eq!(s.state.source_playhead, rate.tick_of(40));
        s.execute("source.setPlayhead", json!({"time":i64::MIN})).unwrap();
        assert_eq!(s.state.source_playhead, if restrict { start } else { Tick::ZERO });
        assert_eq!(s.project, project);
        assert_eq!(s.playhead(), program);
    }
}

#[test]
fn source_marker_accepts_the_last_partial_frame_and_preserves_duration_bounds_and_undo() {
    let rate = FrameRate::new(31, 1);
    let (mut s, item) = source_endpoint_fixture(rate, Tick(335_809_152_000), false);
    s.execute("source.setPlayhead", json!({"frame":40})).unwrap();
    let before = s.project.clone();
    let history = s.history.undo.len();
    let program = s.playhead();
    s.execute("source.addMarker", json!({"name":"Last decoded frame"})).unwrap();
    let filmcraft_project::ItemKind::Media(m) = &s.project.item(item).unwrap().kind else { unreachable!() };
    assert_eq!(m.markers.len(), 1);
    assert_eq!(m.markers[0].start, rate.tick_of(40));
    assert_eq!(m.markers[0].duration, Tick::ZERO);
    assert_eq!(s.history.undo.len(), history + 1);
    let marked = s.project.clone();
    for params in [json!({"frame":41}), json!({"frame":40,"durationFrames":1})] {
        assert!(s.execute("source.addMarker", params).is_err());
        assert_eq!(s.project, marked);
        assert_eq!(s.history.undo.len(), history + 1);
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project, marked);
    assert_eq!(s.playhead(), program);
}
