//! Effects follow the moving effect clock through a frame hold unless Hold Filters is on.
//! Ordinary 32x18 generated media keeps these command/render contracts independent of files.

use filmcraft_engine::Session;
use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{ClipId, Interpolation, Param, ParamValue, TrackItem};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use serde_json::{Value, json};

fn exec(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, params).unwrap()
}

fn clip(s: &Session, c: ClipId) -> TrackItem {
    s.active_sequence().unwrap().find_item(c).unwrap().1.clone()
}

fn fixture(reverse: bool) -> (Session, ClipId) {
    let mut s = Session::default();
    exec(&mut s, "file.newSequence", json!({"width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0}));
    let item = exec(&mut s, "file.newColorMatte", json!({"color": "#ffffff", "width": 32, "height": 18, "seconds": 8}))["item"].as_u64().unwrap();
    let placed =
        exec(&mut s, "timeline.place", json!({"item": item, "track": "V1", "time": 0, "sourceIn": TICKS_PER_SECOND, "duration": 4 * TICKS_PER_SECOND}));
    let c = ClipId(placed["clips"][0].as_u64().unwrap());
    exec(&mut s, "timeline.select", json!({"clips": [c.0]}));
    // Resolve the generated clip's automatic Motion points for exact history comparisons.
    for name in ["position", "anchor"] {
        exec(&mut s, "effects.setParam", json!({"clip": c.0, "effect": "motion", "param": name, "value": [16, 9]}));
    }
    if reverse {
        exec(&mut s, "clip.speedDuration", json!({"clips": [c.0], "speed": 200, "reverse": true}));
    }
    (s, c)
}

fn times(s: &Session, c: ClipId) -> (Tick, Tick) {
    let item = clip(s, c);
    (item.start + item.duration.mul_ratio(1, 4), item.start + item.duration.mul_ratio(3, 4))
}

fn hold(s: &mut Session, c: ClipId, filters: bool) {
    exec(s, "clip.frameHoldOptions", json!({"clips": [c.0], "holdOn": "in", "holdFilters": filters}));
}

fn animate(s: &mut Session, c: ClipId, effect: &str, param: &str, mask: Option<usize>, a: Value, b: Value) {
    let (early, late) = times(s, c);
    let mut p = json!({"clip": c.0, "effect": effect, "param": param});
    if let Some(mask) = mask {
        p["mask"] = json!(mask);
    }
    s.set_playhead(early);
    p["value"] = a;
    exec(s, "effects.setParam", p.clone());
    exec(s, "effects.toggleAnimation", p.clone());
    s.set_playhead(late);
    exec(s, "effects.addKeyframe", p.clone());
    p["value"] = b;
    exec(s, "effects.setParam", p);
}

fn param(s: &Session, c: ClipId, effect: &str, name: &str) -> Param {
    clip(s, c).effect(effect).unwrap().param(name).unwrap().clone()
}

fn red(s: &Session, x: usize, y: usize) -> f32 {
    s.render_program_working(1.0).unwrap().get(x, y)[0]
}

fn assert_undo_redo(s: &mut Session, before: &std::sync::Arc<filmcraft_project::Project>, after: &std::sync::Arc<filmcraft_project::Project>) {
    exec(s, "edit.undo", json!({}));
    assert_eq!(&s.project, before);
    exec(s, "edit.redo", json!({}));
    assert_eq!(&s.project, after);
}

#[test]
fn held_opacity_edits_target_the_rendered_keyframe_and_undo_in_both_directions() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let (mut s, c) = fixture(reverse);
            animate(&mut s, c, "opacity", "opacity", None, json!(50.0), json!(100.0));
            hold(&mut s, c, filters);
            let (early, late) = times(&s, c);
            let clock = clip(&s, c).effect_time_at(late);
            let before_param = param(&s, c, "opacity", "opacity");
            s.set_playhead(late);
            let before_image = red(&s, 16, 9);
            let before = s.project.clone();
            let history = s.history.undo.len();
            // Explicit timeline time must use the same effect clock as the default playhead.
            s.set_playhead(early);
            exec(&mut s, "effects.setParam", json!({"clip": c.0, "effect": "opacity", "param": "opacity", "time": late.0, "value": 25.0}));
            s.set_playhead(late);
            let after_param = param(&s, c, "opacity", "opacity");
            assert_eq!(after_param.keyframes.iter().find(|k| k.time == clock).unwrap().value, ParamValue::Float(25.0));
            for old in &before_param.keyframes {
                if old.time != clock {
                    assert_eq!(after_param.keyframes.iter().find(|k| k.time == old.time), Some(old));
                }
            }
            assert_eq!(after_param.keyframes.len(), before_param.keyframes.len() + usize::from(filters));
            assert!(before_image > 0.45);
            assert!((red(&s, 16, 9) - 0.25).abs() < 1e-5, "Program uses the edited effect key, reverse={reverse}, filters={filters}");
            assert_eq!(s.history.undo.len(), history + 1);
            let after = s.project.clone();
            assert_undo_redo(&mut s, &before, &after);
            assert!((red(&s, 16, 9) - 0.25).abs() < 1e-5);
        }
    }
}

#[test]
fn held_stopwatch_add_remove_and_default_interpolation_follow_the_effect_clock() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let (mut s, c) = fixture(reverse);
            hold(&mut s, c, filters);
            let (early, late) = times(&s, c);
            let first = clip(&s, c).effect_time_at(early);
            let second = clip(&s, c).effect_time_at(late);
            let p = json!({"clip": c.0, "effect": "motion", "param": "position"});
            s.set_playhead(early);
            exec(&mut s, "effects.toggleAnimation", p.clone());
            assert_eq!(param(&s, c, "motion", "position").keyframes[0].time, first);
            let before = s.project.clone();
            s.set_playhead(late);
            exec(&mut s, "effects.addKeyframe", p.clone());
            let got = param(&s, c, "motion", "position");
            assert_eq!(got.keyframes.len(), if filters { 0 } else { 2 });
            if !filters {
                assert!(got.keyframes.iter().any(|k| k.time == second));
                exec(&mut s, "effects.setInterpolation", json!({"clip": c.0, "effect": "motion", "param": "position", "interpolation": "bezier"}));
                assert_eq!(param(&s, c, "motion", "position").keyframes.iter().find(|k| k.time == second).unwrap().interp, Interpolation::Bezier);
                exec(&mut s, "edit.undo", json!({}));
            }
            let after = s.project.clone();
            assert_undo_redo(&mut s, &before, &after);
        }
    }
}

#[test]
fn held_animated_mask_edits_and_listing_agree_with_the_rendered_path() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let (mut s, c) = fixture(reverse);
            exec(&mut s, "masks.add", json!({"clip": c.0, "effect": "opacity", "shape": "polygon", "center": [5, 9], "size": [8, 16]}));
            exec(&mut s, "masks.set", json!({"clip": c.0, "effect": "opacity", "mask": 0, "feather": 0}));
            let path = clip(&s, c).effect("opacity").unwrap().masks[0].path.value.clone();
            let ParamValue::Path(original) = path else { panic!("mask path") };
            let json_path = serde_json::to_value(&original).unwrap();
            animate(&mut s, c, "opacity", "path", Some(0), json_path.clone(), json_path);
            animate(&mut s, c, "opacity", "feather", Some(0), json!(0.0), json!(0.0));
            hold(&mut s, c, filters);
            let (early, late) = times(&s, c);
            let clock = clip(&s, c).effect_time_at(late);
            s.set_playhead(late);
            assert!(red(&s, 5, 9) > 0.99 && red(&s, 13, 9) < 0.01);
            let before = s.project.clone();
            let history = s.history.undo.len();
            exec(&mut s, "masks.translate", json!({"clip": c.0, "effect": "opacity", "mask": 0, "delta": [8, 0]}));
            let after = s.project.clone();
            let mask = clip(&s, c).effect("opacity").unwrap().masks[0].clone();
            let shifted = original.transformed(&Affine::translate(8.0, 0.0));
            assert_eq!(mask.path_at(clock), shifted);
            assert!(red(&s, 5, 9) < 0.01 && red(&s, 13, 9) > 0.99, "mask moves on the rendered held frame");
            let list = exec(&mut s, "masks.list", json!({"clip": c.0}));
            assert_eq!(list["masks"][0]["path"]["vertices"][0]["p"], json!([9.0, 1.0]));
            assert_eq!(s.history.undo.len(), history + 1);
            assert_undo_redo(&mut s, &before, &after);
            s.set_playhead(early);
            assert_eq!(red(&s, 5, 9) > 0.99, !filters, "hold filters freezes mask animation too");
            s.set_playhead(late);
            let before = s.project.clone();
            exec(&mut s, "masks.set", json!({"clip": c.0, "effect": "opacity", "mask": 0, "feather": 2.0}));
            let changed = clip(&s, c).effect("opacity").unwrap().masks[0].clone();
            assert_eq!(changed.feather.keyframes.iter().find(|k| k.time == clock).unwrap().value, ParamValue::Float(2.0));
            assert_eq!(exec(&mut s, "masks.list", json!({"clip": c.0}))["masks"][0]["feather"], json!(2.0));
            let after = s.project.clone();
            assert_undo_redo(&mut s, &before, &after);
        }
    }
}

#[test]
fn static_preset_saves_the_effect_value_shown_through_the_hold() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let (mut s, c) = fixture(reverse);
            animate(&mut s, c, "opacity", "opacity", None, json!(20.0), json!(80.0));
            hold(&mut s, c, filters);
            let (_, late) = times(&s, c);
            s.set_playhead(late);
            let item = clip(&s, c);
            let expected = item.effect("opacity").unwrap().param("opacity").unwrap().value_at(item.effect_time_at(late));
            let index = item.effects.iter().position(|e| e.effect == "opacity").unwrap();
            exec(&mut s, "presets.save", json!({"clip": c.0, "name": "Held opacity", "effects": [index], "keyframes": "none"}));
            let saved = s.presets.find("Held opacity").unwrap();
            assert_eq!(saved.effects[0].params["opacity"].value, expected);
            assert!(saved.effects[0].params["opacity"].keyframes.is_empty());
            // Saving animated presets retains the established relative source-key offsets.
            exec(&mut s, "presets.save", json!({"clip": c.0, "name": "Held keys", "effects": [index], "keyframes": "anchorIn"}));
            let keys = s.presets.find("Held keys").unwrap();
            for (saved, original) in keys.effects[0].params["opacity"].keyframes.iter().zip(&item.effects[index].params["opacity"].keyframes) {
                assert_eq!(saved.time, original.time - item.source_in);
                assert_eq!(saved.value, original.value);
            }
        }
    }
}

#[test]
fn held_direct_graphic_position_uses_the_rendered_layer_clock() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let mut s = Session::default();
            exec(&mut s, "file.newSequence", json!({"width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0}));
            let c = ClipId(
                exec(&mut s, "graphics.newShape", json!({"shape": "rectangle", "position": [8, 9], "size": [6, 6], "seconds": 4, "time": 0}))["clip"]
                    .as_u64()
                    .unwrap(),
            );
            if reverse {
                exec(&mut s, "clip.speedDuration", json!({"clips": [c.0], "speed": 200, "reverse": true}));
            }
            let index = clip(&s, c).effects.iter().position(filmcraft_project::graphic::is_layer).unwrap();
            let effect = clip(&s, c).effects[index].effect.clone();
            animate(&mut s, c, &effect, "position", None, json!([8, 9]), json!([8, 9]));
            hold(&mut s, c, filters);
            let (_, late) = times(&s, c);
            s.set_playhead(late);
            let clock = clip(&s, c).effect_time_at(late);
            let before = s.project.clone();
            assert!(red(&s, 8, 9) > 0.05 && red(&s, 24, 9) < 0.01);
            exec(&mut s, "graphics.set", json!({"clip": c.0, "layer": 0, "props": {"position": [24, 9]}}));
            assert_eq!(clip(&s, c).effects[index].params["position"].value_at(clock), ParamValue::Vec2(Vec2::new(24.0, 9.0)));
            assert!(red(&s, 8, 9) < 0.01 && red(&s, 24, 9) > 0.05);
            let list = exec(&mut s, "graphics.list", json!({"clip": c.0}));
            assert_eq!(list["layers"][0]["position"], json!([24.0, 9.0]), "layer inspection reflects the rendered position");
            let after = s.project.clone();
            assert_undo_redo(&mut s, &before, &after);
            // Explicit layer presets use the same clock as direct Graphic rendering.
            exec(&mut s, "presets.save", json!({"clip": c.0, "name": "Held layer", "effects": [index], "keyframes": "none"}));
            assert_eq!(s.presets.find("Held layer").unwrap().effects[0].params["position"].value, ParamValue::Vec2(Vec2::new(24.0, 9.0)));
        }
    }
}

#[test]
fn stretched_graphic_layer_edits_and_presets_keep_the_protected_clock_separate_from_intrinsics() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let mut s = Session::default();
            exec(&mut s, "file.newSequence", json!({"width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0}));
            let c = ClipId(
                exec(&mut s, "graphics.newShape", json!({"shape": "rectangle", "position": [8, 9], "size": [6, 6], "seconds": 4, "time": 0}))["clip"]
                    .as_u64()
                    .unwrap(),
            );
            let layer = clip(&s, c).effects.iter().position(filmcraft_project::graphic::is_layer).unwrap();
            exec(&mut s, "graphics.set", json!({"clip": c.0, "layer": 0, "props": {"fillColor": "#ffffff"}}));
            // The designed graphic has stationary protected ends and an animated middle.
            s.edit_sequence("Fixture keys", |q, _, _| {
                let (_, it) = q.find_item_mut(c).unwrap();
                let position = it.effects[layer].params.get_mut("position").unwrap();
                position.toggle_animation(Tick::ZERO);
                for (seconds, x) in [(0, 8.0), (1, 8.0), (3, 20.0), (4, 20.0)] {
                    position.set_at(Tick(seconds * TICKS_PER_SECOND), ParamValue::Vec2(Vec2::new(x, 9.0)));
                }
                let opacity = it.effect_mut("opacity").unwrap().params.get_mut("opacity").unwrap();
                opacity.toggle_animation(Tick::ZERO);
                for (seconds, value) in [(0, 20.0), (1, 40.0), (3, 80.0), (4, 100.0)] {
                    opacity.set_at(Tick(seconds * TICKS_PER_SECOND), ParamValue::Float(value));
                }
                Ok(())
            })
            .unwrap();
            exec(&mut s, "graphics.setResponsiveTime", json!({"clip": c.0, "introSeconds": 1, "outroSeconds": 1}));
            exec(&mut s, "timeline.trim", json!({"clip": c.0, "edge": "out", "deltaFrames": 96}));
            // Rendered protected positions survive extension: 0.5 seconds is still at the
            // designed intro; 0.5 seconds before the new end is still at the designed outro.
            for (time, x, empty) in [(Tick(TICKS_PER_SECOND / 2), 8, 20), (Tick(15 * TICKS_PER_SECOND / 2), 20, 8)] {
                s.set_playhead(time);
                assert!(red(&s, x, 9) > 0.1 && red(&s, empty, 9) < 0.01);
                assert_eq!(exec(&mut s, "graphics.list", json!({"clip": c.0}))["layers"][0]["position"], json!([x as f64, 9.0]));
            }
            // A head trim restarts the designed intro, so even a held In frame has different
            // layer and intrinsic clocks. Reverse 200% preserves the same consumed media span.
            exec(&mut s, "timeline.trim", json!({"clip": c.0, "edge": "in", "deltaFrames": 24}));
            if reverse {
                exec(&mut s, "clip.speedDuration", json!({"clips": [c.0], "speed": 200, "reverse": true}));
            }
            let (_, late) = times(&s, c);
            s.set_playhead(late);
            hold(&mut s, c, filters);
            let item = clip(&s, c);
            let raw = item.effect_time_at(late);
            let mapped = filmcraft_render::graphic_clip::parameter_time(&item, &item.effects[layer], raw);
            assert_ne!(raw, mapped, "a stretched layer must not use the intrinsic clock");
            if filters {
                // In is the clip's first displayed frame. Reverse 200% starts at the
                // last source frame (191 at 24fps), which maps into the protected outro.
                let (held_raw, held_layer) =
                    if reverse { (Tick(191 * TICKS_PER_SECOND / 24), Tick(95 * TICKS_PER_SECOND / 24)) } else { (Tick(TICKS_PER_SECOND), Tick::ZERO) };
                assert_eq!(raw, held_raw);
                assert_eq!(mapped, held_layer);
            }
            let old_keys = item.effects[layer].params["position"].keyframes.clone();
            let before = s.project.clone();
            let history = s.history.undo.len();
            // Generic Effect Controls writes must use the same mapped clock as graphics.set.
            exec(&mut s, "effects.setParam", json!({"clip": c.0, "effect": layer, "param": "position", "value": [24, 9]}));
            let item = clip(&s, c);
            let position = &item.effects[layer].params["position"];
            assert_eq!(position.keyframes.iter().find(|k| k.time == mapped).unwrap().value, ParamValue::Vec2(Vec2::new(24.0, 9.0)));
            for old in old_keys.iter().filter(|k| k.time != mapped) {
                assert_eq!(position.keyframes.iter().find(|k| k.time == old.time), Some(old));
            }
            assert!(red(&s, 24, 9) > 0.1 && red(&s, 8, 9) < 0.01 && red(&s, 20, 9) < 0.01);
            assert_eq!(exec(&mut s, "graphics.list", json!({"clip": c.0}))["layers"][0]["position"], json!([24.0, 9.0]));
            assert_eq!(s.history.undo.len(), history + 1);
            let after = s.project.clone();
            assert_undo_redo(&mut s, &before, &after);
            exec(&mut s, "edit.undo", json!({}));
            exec(&mut s, "graphics.set", json!({"clip": c.0, "layer": 0, "props": {"position": [24, 9]}}));
            assert_eq!(s.project, after, "direct Graphic editing produces the same rendered key as Effect Controls");
            assert_undo_redo(&mut s, &before, &after);
            // Intrinsic effects still use the raw effect clock. One static preset may contain
            // both families, and each must capture what Program actually displays.
            let opacity = item.effects.iter().position(|e| e.effect == "opacity").unwrap();
            let expected_opacity = item.effects[opacity].params["opacity"].value_at(raw);
            assert_ne!(expected_opacity, item.effects[opacity].params["opacity"].value_at(mapped));
            exec(&mut s, "presets.save", json!({"clip": c.0, "name": "Responsive held mix", "effects": [layer, opacity], "keyframes": "none"}));
            let saved = s.presets.find("Responsive held mix").unwrap();
            assert_eq!(saved.effects[0].params["position"].value, ParamValue::Vec2(Vec2::new(24.0, 9.0)));
            assert_eq!(saved.effects[1].params["opacity"].value, expected_opacity);
            assert!(saved.effects.iter().flat_map(|e| e.params.values()).all(|p| p.keyframes.is_empty()));
            let before = s.project.clone();
            let intensity = red(&s, 24, 9);
            exec(&mut s, "effects.setParam", json!({"clip": c.0, "effect": "opacity", "param": "opacity", "value": 25.0}));
            assert_eq!(param(&s, c, "opacity", "opacity").keyframes.iter().find(|k| k.time == raw).unwrap().value, ParamValue::Float(25.0));
            assert!((red(&s, 24, 9) - 0.25).abs() < 1e-5 && (intensity - 0.25).abs() > 0.05);
            let after = s.project.clone();
            assert_undo_redo(&mut s, &before, &after);
        }
    }
}
