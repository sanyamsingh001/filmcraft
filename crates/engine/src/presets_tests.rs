use serde_json::json;

use crate::Session;
use crate::presets::{KeyframeMode, retime};
use filmcraft_project::{ClipId, ParamValue, TrackItem};
use filmcraft_time::{TICKS_PER_SECOND, Tick};

fn demo() -> (Session, Vec<ClipId>) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let clips: Vec<ClipId> = q.video_tracks[0].items.iter().map(|i| i.id).collect();
    (s, clips)
}

fn item(s: &Session, c: ClipId) -> TrackItem {
    s.active_sequence().unwrap().find_item(c).unwrap().1.clone()
}

fn secs(x: f64) -> Tick {
    Tick((x * TICKS_PER_SECOND as f64).round() as i64)
}

fn media_len(it: &TrackItem) -> Tick {
    Tick((it.duration.0 as f64 * it.speed.abs()).round() as i64)
}

/// Save a keyframed blur from clip A, apply with each keyframe mode to clip B (another length).
#[test]
fn keyframe_modes_retime_onto_the_target() {
    let (mut s, clips) = demo();
    let (a, b) = (clips[0], clips[2]);
    let ia = item(&s, a);
    s.state.selection = vec![a];
    s.execute("effects.apply", json!({"effect": "gaussian_blur"})).unwrap();
    // keyframes 0.5 s after the in point (20) and 1 s before the out point (0)
    let k1 = ia.start + secs(0.5);
    let k2 = ia.end() - secs(1.0);
    s.set_playhead(k1);
    let k1 = s.playhead();
    s.execute("effects.toggleAnimation", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "time": k1.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 20.0, "time": k1.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 0.0, "time": k2.0})).unwrap();
    let ib = item(&s, b);
    assert_ne!(media_len(&ia), media_len(&ib), "the demo clips differ in length");
    let la = media_len(&ia);
    let lb = media_len(&ib);
    for (mode, key) in [(KeyframeMode::AnchorToIn, "anchorIn"), (KeyframeMode::AnchorToOut, "anchorOut"), (KeyframeMode::Scale, "scale")] {
        let name = format!("blur-{key}");
        let bi = item(&s, a).effects.iter().position(|e| e.effect == "gaussian_blur").unwrap();
        s.execute("presets.save", json!({"clip": a.0, "name": name, "keyframes": key, "effects": [bi]})).unwrap();
        let before = item(&s, b).effects.len();
        s.execute("presets.apply", json!({"preset": name, "clips": [b.0]})).unwrap();
        let it = item(&s, b);
        assert_eq!(it.effects.len(), before + 1);
        let e = it.effects.iter().rfind(|e| e.effect == "gaussian_blur").unwrap();
        // standard effects go before the intrinsic ones
        assert!(it.effects.iter().position(|x| x.effect == "gaussian_blur").unwrap() < it.effects.iter().position(|x| x.effect == "motion").unwrap());
        let p = e.param("blurriness").unwrap();
        assert_eq!(
            p.keyframes.len(),
            2,
            "{:?} src {:?}",
            p.keyframes.iter().map(|k| (k.time, k.value.clone())).collect::<Vec<_>>(),
            item(&s, a)
                .effects
                .iter()
                .find(|e| e.effect == "gaussian_blur")
                .unwrap()
                .param("blurriness")
                .unwrap()
                .keyframes
                .iter()
                .map(|k| k.time)
                .collect::<Vec<_>>()
        );
        let rel1 = ia.source_time_at(k1) - ia.source_in;
        let rel2 = ia.source_time_at(k2) - ia.source_in;
        let (e1, e2) = match mode {
            KeyframeMode::AnchorToIn => (ib.source_in + rel1, ib.source_in + rel2),
            KeyframeMode::AnchorToOut => (ib.source_in + lb - (la - rel1), ib.source_in + lb - (la - rel2)),
            KeyframeMode::Scale => (
                ib.source_in + Tick((rel1.0 as i128 * lb.0 as i128 / la.0 as i128) as i64),
                ib.source_in + Tick((rel2.0 as i128 * lb.0 as i128 / la.0 as i128) as i64),
            ),
        };
        assert_eq!(p.keyframes[0].time, e1, "{key}");
        assert_eq!(p.keyframes[1].time, e2, "{key}");
        assert_eq!(p.keyframes[0].value, ParamValue::Float(20.0));
        assert_eq!(retime(rel1, mode, la, &ib), e1);
        s.undo();
        assert_eq!(item(&s, b).effects.len(), before, "apply is one undo step");
    }
}

#[test]
fn save_without_keyframes_keeps_the_value_at_the_playhead() {
    let (mut s, clips) = demo();
    let a = clips[0];
    let ia = item(&s, a);
    s.state.selection = vec![a];
    s.execute("effects.apply", json!({"effect": "gaussian_blur"})).unwrap();
    s.set_playhead(ia.start);
    s.execute("effects.toggleAnimation", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "time": ia.start.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 40.0, "time": ia.start.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "gaussian_blur", "param": "blurriness", "value": 0.0, "time": (ia.start + secs(2.0)).0}))
        .unwrap();
    s.set_playhead(ia.start + secs(1.0));
    let bi = item(&s, a).effects.iter().position(|e| e.effect == "gaussian_blur").unwrap();
    s.execute("presets.save", json!({"name": "half blur", "keyframes": "none", "effects": [bi]})).unwrap();
    let l = s.execute("presets.list", json!({})).unwrap();
    let p = l["presets"].as_array().unwrap().iter().find(|p| p["name"] == "half blur").unwrap().clone();
    assert_eq!(p["animated"], false);
    s.execute("presets.apply", json!({"preset": "half blur", "clips": [clips[1].0]})).unwrap();
    let e = item(&s, clips[1]).effects.into_iter().find(|e| e.effect == "gaussian_blur").unwrap();
    let v = e.param("blurriness").unwrap();
    assert!(!v.is_animated());
    let ParamValue::Float(x) = v.value else { panic!() };
    assert!((x - 20.0).abs() < 1.0, "value at the playhead: {x}");
}

#[test]
fn presets_with_masks_and_intrinsics_round_trip_through_files() {
    let (mut s, clips) = demo();
    let a = clips[0];
    s.state.selection = vec![a];
    s.execute("effects.apply", json!({"effect": "black_white"})).unwrap();
    s.execute("masks.add", json!({"effect": "black_white", "shape": "ellipse", "center": [960, 540], "size": [800, 400]})).unwrap();
    s.execute("effects.setParam", json!({"clip": a.0, "effect": "opacity", "param": "opacity", "value": 50.0})).unwrap();
    let ia = item(&s, a);
    let bw = ia.effects.iter().position(|e| e.effect == "black_white").unwrap();
    let op = ia.effects.iter().position(|e| e.effect == "opacity").unwrap();
    s.execute("presets.save", json!({"name": "masked bw", "effects": [bw, op], "description": "test"})).unwrap();
    let dir = std::env::temp_dir().join(format!("filmcraft-presets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.json").to_string_lossy().to_string();
    let r = s.execute("presets.export", json!({"path": path, "names": ["masked bw", "Fade In"]})).unwrap();
    assert_eq!(r["count"], 2);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("filmcraft.effect-presets"));
    // a fresh session imports and applies it
    let (mut s2, clips2) = demo();
    s2.execute("presets.import", json!({"path": path})).unwrap();
    let names: Vec<String> =
        s2.execute("presets.list", json!({})).unwrap()["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"masked bw".to_string()));
    s2.execute("presets.apply", json!({"preset": "masked bw", "clips": [clips2[1].0]})).unwrap();
    let it = item(&s2, clips2[1]);
    let e = it.effects.iter().find(|e| e.effect == "black_white").unwrap();
    assert_eq!(e.masks.len(), 1);
    assert_eq!(it.effects.iter().filter(|e| e.effect == "opacity").count(), 1, "intrinsic replaced, not duplicated");
    assert_eq!(it.effect("opacity").unwrap().param("opacity").unwrap().value, ParamValue::Float(50.0));
    // garbage is rejected
    std::fs::write(dir.join("bad.json"), b"{\"format\":\"something\",\"version\":1,\"presets\":[]}").unwrap();
    assert!(s2.execute("presets.import", json!({"path": dir.join("bad.json").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn builtins_apply_and_user_presets_persist() {
    let (mut s, clips) = demo();
    let l = s.execute("presets.list", json!({})).unwrap();
    let all = l["presets"].as_array().unwrap();
    assert!(all.len() >= 8);
    assert!(all.iter().all(|p| p["builtin"] == true));
    for p in all {
        let name = p["name"].as_str().unwrap();
        s.execute("presets.apply", json!({"preset": name, "clips": [clips[0].0]})).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    // Fade Out anchors to the out point
    let it = item(&s, clips[0]);
    let op = it.effect("opacity").unwrap().param("opacity").unwrap();
    assert_eq!(op.keyframes.last().unwrap().time, it.source_in + media_len(&it));
    assert!(s.execute("presets.delete", json!({"name": "Fade In"})).is_err(), "built-ins stay");
    // persistence through the data directory
    let dir = std::env::temp_dir().join(format!("filmcraft-presets-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.presets.set_dir(&dir);
    s.state.selection = vec![clips[1]];
    s.execute("effects.apply", json!({"effect": "invert"})).unwrap();
    s.execute("presets.save", json!({"name": "Mine"})).unwrap();
    s.execute("presets.rename", json!({"name": "Mine", "to": "Mine 2"})).unwrap();
    let mut lib = crate::presets::PresetLibrary::default();
    lib.set_dir(&dir);
    assert_eq!(lib.user.len(), 1);
    assert_eq!(lib.user[0].name, "Mine 2");
    s.execute("presets.delete", json!({"name": "Mine 2"})).unwrap();
    let mut lib = crate::presets::PresetLibrary::default();
    lib.set_dir(&dir);
    assert!(lib.user.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mask_geometry_scales_to_the_target_frame_size() {
    let (s, clips) = demo();
    let it = item(&s, clips[0]);
    let p = crate::presets::builtin_presets().into_iter().find(|p| p.name == "Spotlight").unwrap();
    let fx = crate::presets::instantiate(&p, &it, (960, 540));
    let m = &fx[0].masks[0];
    let path = m.path_at(it.source_in);
    let c = path.centroid();
    assert!((c.x - 480.0).abs() < 1e-6 && (c.y - 270.0).abs() < 1e-6, "{c:?}");
    assert_eq!(m.feather.value, ParamValue::Float(80.0));
}

/// Public built-in preset commands should preserve both ends of one opacity curve.
/// Imported/user presets retain the replacement contract tested above.
fn check_builtin_fade_pair(seconds: i64, first: &str, second: &str) {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Both fades", "width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0})).unwrap();
    let media = s
        .execute("file.newColorMatte", json!({"name": "White fade target", "width": 32, "height": 18, "seconds": seconds, "color": "#ffffff"}))
        .unwrap()["item"]
        .as_u64()
        .unwrap();
    let end = Tick(seconds * TICKS_PER_SECOND);
    let clip =
        s.execute("timeline.place", json!({"item": media, "track": "V1", "time": 0, "sourceIn": 0, "duration": end.0, "insert": false})).unwrap()["clips"][0]
            .as_u64()
            .unwrap();
    let clip = ClipId(clip);
    let listed = s.execute("presets.list", json!({})).unwrap();
    for name in [first, second] {
        let row = listed["presets"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap();
        assert_eq!(row["builtin"], true, "these cases must apply the actual built-ins");
    }
    let samples = [Tick::ZERO, Tick(TICKS_PER_SECOND / 2), Tick(end.0 / 2), end - Tick(TICKS_PER_SECOND / 2), end - Tick(TICKS_PER_SECOND / 24)];
    let baseline: Vec<_> = samples.iter().map(|&clock| s.render_program_at(1.0, clock).unwrap()).collect();
    assert!(baseline.iter().all(|image| image.px.as_chunks::<4>().0.iter().map(|pixel| pixel[0] + pixel[1] + pixel[2]).sum::<f32>() > 1.0));
    let count = |s: &mut Session| s.execute("history.list", json!({})).unwrap()["undo"].as_array().unwrap().len();
    let before = count(&mut s);
    assert_eq!(s.execute("presets.apply", json!({"preset": first, "clips": [clip.0]})).unwrap()["applied"], 1);
    assert_eq!(count(&mut s), before + 1);
    let first_item = item(&s, clip);
    let first_pixels: Vec<_> = samples.iter().map(|&clock| s.render_program_at(1.0, clock).unwrap()).collect();
    assert_eq!(s.execute("presets.apply", json!({"preset": second, "clips": [clip.0]})).unwrap()["applied"], 1);
    assert_eq!(count(&mut s), before + 2, "each preset apply is one undo step");
    let current = item(&s, clip);
    assert_eq!(current.effects.iter().filter(|effect| effect.effect == "opacity").count(), 1, "both fades share one intrinsic opacity");
    let opacity = current.effect("opacity").unwrap().param("opacity").unwrap();
    let expected = if seconds == 2 {
        vec![(Tick::ZERO, ParamValue::Float(0.0)), (Tick(TICKS_PER_SECOND), ParamValue::Float(100.0)), (end, ParamValue::Float(0.0))]
    } else {
        vec![
            (Tick::ZERO, ParamValue::Float(0.0)),
            (Tick(TICKS_PER_SECOND), ParamValue::Float(100.0)),
            (end - Tick(TICKS_PER_SECOND), ParamValue::Float(100.0)),
            (end, ParamValue::Float(0.0)),
        ]
    };
    assert_eq!(
        opacity.keyframes.iter().map(|key| (key.time, key.value.clone())).collect::<Vec<_>>(),
        expected,
        "{seconds}s: {first} then {second} must retain both boundaries"
    );
    for (clock, expected_value) in samples.iter().zip([0.0, 50.0, 100.0, 50.0, 100.0 / 24.0]) {
        let actual = opacity.value_at(*clock).as_f64().unwrap();
        assert!((actual - expected_value).abs() < 1e-6, "{first} then {second} at {clock:?}: {actual}");
    }
    let rendered: Vec<_> = samples.iter().map(|&clock| s.render_program_at(1.0, clock).unwrap()).collect();
    let energy = |image: &filmcraft_render::Image| image.px.as_chunks::<4>().0.iter().map(|pixel| pixel[0] + pixel[1] + pixel[2]).sum::<f32>();
    assert_eq!(energy(&rendered[0]), 0.0, "first video frame fades from black");
    assert_eq!(rendered[2], baseline[2], "the middle stays fully visible");
    for index in [1, 3, 4] {
        assert!(energy(&rendered[index]) > 0.0 && energy(&rendered[index]) < energy(&baseline[index]), "frame {index} must show an actual rendered fade");
    }
    assert_eq!(s.execute("edit.undo", json!({})).unwrap()["undone"], json!(format!("Apply Preset {second}")));
    assert_eq!(count(&mut s), before + 1);
    assert_eq!(item(&s, clip), first_item, "one undo restores the exact first-preset clip");
    let restored: Vec<_> = samples.iter().map(|&clock| s.render_program_at(1.0, clock).unwrap()).collect();
    assert_eq!(restored, first_pixels, "undo restores actual first-preset rendering");
}

#[test]
fn builtin_fade_in_then_out_keeps_both_ends_on_two_second_clip() {
    check_builtin_fade_pair(2, "Fade In", "Fade Out");
}

#[test]
fn builtin_fade_out_then_in_keeps_both_ends_on_two_second_clip() {
    check_builtin_fade_pair(2, "Fade Out", "Fade In");
}

#[test]
fn builtin_fade_in_then_out_keeps_both_ends_on_eight_second_clip() {
    check_builtin_fade_pair(8, "Fade In", "Fade Out");
}

#[test]
fn builtin_fade_out_then_in_keeps_both_ends_on_eight_second_clip() {
    check_builtin_fade_pair(8, "Fade Out", "Fade In");
}

fn generated_fade_target(duration: Tick, source_in: Tick) -> (Session, ClipId) {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Fade source clock", "width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0})).unwrap();
    let media = s.execute("file.newColorMatte", json!({"name": "White", "color": "#ffffff", "seconds": 8})).unwrap()["item"].as_u64().unwrap();
    let clip = s.execute("timeline.place", json!({"item": media, "track": "V1", "time": 0, "sourceIn": source_in.0, "duration": duration.0})).unwrap()["clips"]
        [0]
    .as_u64()
    .unwrap();
    (s, ClipId(clip))
}

#[test]
fn builtin_fades_share_short_clips_and_reapplication_stays_bounded() {
    for duration in [secs(0.5), secs(1.5)] {
        for (first, second) in [("Fade In", "Fade Out"), ("Fade Out", "Fade In")] {
            let (mut s, clip) = generated_fade_target(duration, secs(1.0));
            let middle = Tick(duration.0 / 2);
            let baseline = s.render_program_at(1.0, middle).unwrap();
            s.execute("presets.apply", json!({"preset": first, "clips": [clip.0]})).unwrap();
            let first_item = item(&s, clip);
            s.execute("presets.apply", json!({"preset": second, "clips": [clip.0]})).unwrap();
            let composed = item(&s, clip);
            let param = composed.effect("opacity").unwrap().param("opacity").unwrap();
            assert_eq!(param.keyframes.len(), 3, "the two ramps meet at a full-opacity midpoint");
            assert_eq!(param.value_at(composed.source_in), ParamValue::Float(0.0));
            assert_eq!(param.value_at(composed.source_in + middle), ParamValue::Float(100.0));
            assert_eq!(param.value_at(composed.source_in + duration), ParamValue::Float(0.0));
            assert_eq!(s.render_program_at(1.0, middle).unwrap(), baseline, "short clips remain visible between fades");
            assert_eq!(s.execute("edit.undo", json!({})).unwrap()["undone"], json!(format!("Apply Preset {second}")));
            assert_eq!(item(&s, clip), first_item);
            for _ in 0..8 {
                for preset in [first, second] {
                    s.execute("presets.apply", json!({"preset": preset, "clips": [clip.0]})).unwrap();
                    let current = item(&s, clip);
                    assert!(current.effect("opacity").unwrap().param("opacity").unwrap().keyframes.len() <= 4);
                }
            }
            assert_eq!(item(&s, clip), composed, "repeated fades must not multiply keys or change the curve");
        }
    }
}

#[test]
fn builtin_fade_pairs_use_trimmed_media_clock_at_changed_speed() {
    for speed in [50, 200] {
        let (mut s, clip) = generated_fade_target(secs(2.0), secs(1.0));
        s.execute("clip.speedDuration", json!({"clips": [clip.0], "speed": speed})).unwrap();
        let target = item(&s, clip);
        assert_eq!(target.source_in, secs(1.0));
        assert_eq!(media_len(&target), secs(2.0));
        let middle = target.start + Tick(target.duration.0 / 2);
        let baseline = s.render_program_at(1.0, middle).unwrap();
        for preset in ["Fade In", "Fade Out"] {
            s.execute("presets.apply", json!({"preset": preset, "clips": [clip.0]})).unwrap();
        }
        let current = item(&s, clip);
        let param = current.effect("opacity").unwrap().param("opacity").unwrap();
        assert_eq!(
            param.keyframes.iter().map(|key| (key.time, key.value.clone())).collect::<Vec<_>>(),
            vec![(secs(1.0), ParamValue::Float(0.0)), (secs(2.0), ParamValue::Float(100.0)), (secs(3.0), ParamValue::Float(0.0))]
        );
        assert_eq!(s.render_program_at(1.0, middle).unwrap(), baseline, "the media midpoint remains visible at either speed");
        assert_eq!(param.value_at(current.effect_time_at(current.start)), ParamValue::Float(0.0));
    }
}

#[test]
fn same_name_user_fade_keeps_intrinsic_replacement_and_masks() {
    let (mut s, clip) = generated_fade_target(secs(8.0), Tick::ZERO);
    s.state.selection = vec![clip];
    s.execute("effects.setParam", json!({"clip": clip.0, "effect": "opacity", "param": "opacity", "value": 50.0})).unwrap();
    s.execute("masks.add", json!({"effect": "opacity", "shape": "ellipse", "center": [16, 9], "size": [12, 8]})).unwrap();
    let opacity = item(&s, clip).effects.iter().position(|effect| effect.effect == "opacity").unwrap();
    s.execute("presets.save", json!({"clip": clip.0, "name": "Fade In", "effects": [opacity]})).unwrap();
    let listing = s.execute("presets.list", json!({})).unwrap();
    assert_eq!(listing["presets"].as_array().unwrap().iter().find(|preset| preset["name"] == "Fade In").unwrap()["builtin"], false);
    s.execute("presets.apply", json!({"preset": "Fade Out", "clips": [clip.0]})).unwrap();
    assert!(item(&s, clip).effect("opacity").unwrap().param("opacity").unwrap().is_animated());
    s.execute("presets.apply", json!({"preset": "Fade In", "clips": [clip.0]})).unwrap();
    let result = item(&s, clip);
    assert_eq!(result.effects.iter().filter(|effect| effect.effect == "opacity").count(), 1);
    let effect = result.effect("opacity").unwrap();
    assert_eq!(effect.param("opacity").unwrap().value, ParamValue::Float(50.0));
    assert!(effect.param("opacity").unwrap().keyframes.is_empty(), "same-name user preset replaces existing animation");
    assert_eq!(effect.masks.len(), 1);
}

#[test]
fn imported_builtin_named_fade_keeps_user_replacement_semantics() {
    let (mut s, clip) = generated_fade_target(secs(8.0), Tick::ZERO);
    let directory = std::path::Path::new("target/fixtures/engine/presets");
    std::fs::create_dir_all(directory).unwrap();
    let path = directory.join(format!("fade-shadow-{}.json", std::process::id()));
    s.execute("presets.export", json!({"path": path.to_string_lossy(), "names": ["Fade In"]})).unwrap();
    s.execute("presets.import", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(!s.presets.find("Fade In").unwrap().builtin, "imported presets have user identity");
    s.execute("presets.apply", json!({"preset": "Fade Out", "clips": [clip.0]})).unwrap();
    s.execute("presets.apply", json!({"preset": "Fade In", "clips": [clip.0]})).unwrap();
    let result = item(&s, clip);
    assert_eq!(result.effects.iter().filter(|effect| effect.effect == "opacity").count(), 1);
    let param = result.effect("opacity").unwrap().param("opacity").unwrap();
    assert_eq!(
        param.keyframes.iter().map(|key| (key.time, key.value.clone())).collect::<Vec<_>>(),
        vec![(Tick::ZERO, ParamValue::Float(0.0)), (secs(1.0), ParamValue::Float(100.0))],
        "importing a builtin name must still replace, rather than combine, the prior Fade Out"
    );
    std::fs::remove_file(path).unwrap();
}
