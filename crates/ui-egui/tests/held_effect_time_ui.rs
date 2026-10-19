//! Frame holds leave effect controls and mask geometry on the renderer's clock.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::{ClipId, ParamValue, TrackItem};
use filmcraft_time::TICKS_PER_SECOND;
use filmcraft_ui_egui::{FilmcraftApp, control::ControlRequest};
use serde_json::{Value, json};

fn exec(s: &mut Session, command: &str, p: Value) -> Value {
    s.execute(command, p).unwrap()
}

fn item(s: &Session, c: ClipId) -> TrackItem {
    s.active_sequence().unwrap().find_item(c).unwrap().1.clone()
}

fn fixture(reverse: bool) -> (Session, ClipId) {
    let mut s = Session::default();
    exec(&mut s, "file.newSequence", json!({"width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0}));
    let source = exec(&mut s, "file.newColorMatte", json!({"color": "#ffffff", "width": 32, "height": 18, "seconds": 8}))["item"].as_u64().unwrap();
    let c = ClipId(exec(&mut s, "timeline.place", json!({"item": source, "track": "V1", "time": 0, "sourceIn": TICKS_PER_SECOND, "duration": 4 * TICKS_PER_SECOND}))["clips"][0].as_u64().unwrap());
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

fn animate(s: &mut Session, c: ClipId, effect: &str, name: &str, mask: Option<usize>, values: [Value; 2]) {
    let i = item(s, c);
    let mut p = json!({"clip": c.0, "effect": effect, "param": name});
    if let Some(mask) = mask {
        p["mask"] = json!(mask);
    }
    for (index, value) in values.into_iter().enumerate() {
        s.set_playhead(i.start + i.duration.mul_ratio(if index == 0 { 1 } else { 3 }, 4));
        p["value"] = value;
        if index == 0 {
            exec(s, "effects.setParam", p.clone());
            exec(s, "effects.toggleAnimation", p.clone());
        } else {
            exec(s, "effects.addKeyframe", p.clone());
            exec(s, "effects.setParam", p.clone());
        }
    }
}

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new(session: Session) -> Self {
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(10_000).build_eframe(move |_| app);
        let mut d = Self { harness, tx };
        d.frames(4);
        d.call("ui.set", json!({"workspace": "Editing", "focused": "Program Monitor"}));
        d.call("ui.panel.show", json!({"panel": "Program Monitor"}));
        d.call("ui.menu.invoke", json!({"id": "window.maximizeFrame"}));
        d
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            let ctx = self.harness.ctx.clone();
            let mut input = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut input);
            *self.harness.input_mut() = input;
            self.harness.step();
        }
    }

    fn call(&mut self, method: &str, p: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, p.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(r) = reply.try_recv() {
                assert_eq!(r["ok"], true, "{method} {p}: {r}");
                return r["result"].clone();
            }
        }
        panic!("No control reply for {method}")
    }

    fn command(&mut self, command: &str, p: Value) -> Value {
        self.call("engine.execute", json!({"command": command, "params": p}))
    }

    fn rect(&mut self, id: &str) -> [f64; 4] {
        let all = self.call("ui.elements", json!({"prefix": id}));
        let r = all.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("Missing {id}: {all}"))["rect"].as_array().unwrap();
        std::array::from_fn(|i| r[i].as_f64().unwrap())
    }
}

#[test]
fn held_mask_overlay_matches_unheld_effect_geometry_and_vertex_drag_undo() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let (mut s, c) = fixture(reverse);
            exec(&mut s, "masks.add", json!({"clip": c.0, "effect": "opacity", "shape": "polygon", "center": [5, 9], "size": [6, 6]}));
            exec(&mut s, "masks.set", json!({"clip": c.0, "effect": "opacity", "mask": 0, "feather": 0}));
            animate(&mut s, c, "opacity", "path", Some(0), [json!([[2, 6], [8, 6], [8, 12], [2, 12]]), json!([[12, 6], [18, 6], [18, 12], [12, 12]])]);
            animate(&mut s, c, "motion", "position", None, [json!([16, 9]), json!([20, 9])]);
            let i = item(&s, c);
            let (early, late) = (i.start + i.duration.mul_ratio(1, 4), i.start + i.duration.mul_ratio(3, 4));
            // Hold on source In is before every source key. The unheld reference is the first
            // source key: early in forward playback, late in reversed playback.
            let shown = if reverse && !filters { early } else { late };
            let reference = if filters && !reverse { early } else { shown };
            s.set_playhead(reference);
            let mut d = Driver::new(s);
            let ordinary = d.rect("program.mask.vertex.0");
            d.command("playhead.set", json!({"time": shown.0}));
            d.command("clip.frameHoldOptions", json!({"clips": [c.0], "holdOn": "in", "holdFilters": filters}));
            let held = d.rect("program.mask.vertex.0");
            assert!(ordinary.iter().zip(&held).all(|(a, b)| (a - b).abs() < 0.05), "effect geometry stays on its reference clock: {ordinary:?}→{held:?}");
            let before = d.harness.state().session.project.clone();
            let i = item(&d.harness.state().session, c);
            let clock = i.effect_time_at(shown);
            let old = i.effect("opacity").unwrap().masks[0].path_at(clock);
            let history = d.harness.state().session.history.undo.len();
            let (x, y) = (held[0] + held[2] / 2.0, held[1] + held[3] / 2.0);
            d.call("ui.drag", json!({"from": {"x": x, "y": y}, "to": {"x": x + 18.0, "y": y}, "steps": 6}));
            let changed = item(&d.harness.state().session, c).effect("opacity").unwrap().masks[0].clone();
            assert!(changed.path_at(clock).vertices[0].p.x > old.vertices[0].p.x, "drag edits the path rendered at this clock");
            assert_eq!(d.harness.state().session.history.undo.len(), history + 1);
            let after = d.harness.state().session.project.clone();
            d.command("edit.undo", json!({}));
            assert_eq!(d.harness.state().session.project, before);
            d.command("edit.redo", json!({}));
            assert_eq!(d.harness.state().session.project, after);
        }
    }
}

fn lumetri_text(app: &mut FilmcraftApp) -> Vec<String> {
    let ctx = egui::Context::default();
    let output =
        ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 900.0))), ..Default::default() }, |ui| {
            let rect = ui.max_rect();
            filmcraft_ui_egui::panels::lumetri::show(app, ui, rect);
        });
    output
        .shapes
        .into_iter()
        .filter_map(|s| match s.shape {
            egui::epaint::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn lumetri_values_follow_hold_filters_and_reversed_effect_time() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let (mut s, c) = fixture(reverse);
            exec(&mut s, "effects.apply", json!({"clips": [c.0], "effect": "lumetri"}));
            animate(&mut s, c, "lumetri", "exposure", None, [json!(-1.3), json!(1.7)]);
            let i = item(&s, c);
            let time = i.start + i.duration.mul_ratio(if reverse { 1 } else { 3 }, 4);
            s.set_playhead(time);
            exec(&mut s, "clip.frameHoldOptions", json!({"clips": [c.0], "holdOn": "in", "holdFilters": filters}));
            let i = item(&s, c);
            let ParamValue::Float(value) = i.effect("lumetri").unwrap().param("exposure").unwrap().value_at(i.effect_time_at(time)) else { panic!("exposure") };
            let mut app = FilmcraftApp::new(s);
            let texts = lumetri_text(&mut app);
            let expected = format!("{value:.1}");
            assert!(texts.contains(&expected), "Lumetri displays its evaluated exposure {expected}: {texts:?}");
            let wrong = if expected == "1.7" { "-1.3" } else { "1.7" };
            assert!(!texts.iter().any(|s| s == wrong), "the other timeline key is not displayed");
        }
    }
}

#[test]
fn responsive_graphic_hit_geometry_uses_layer_time_and_raw_motion_time_through_a_hold() {
    for reverse in [false, true] {
        for filters in [false, true] {
            let mut s = Session::default();
            exec(&mut s, "file.newSequence", json!({"width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0}));
            let c = ClipId(
                exec(&mut s, "graphics.newShape", json!({"shape": "rectangle", "position": [8, 9], "size": [6, 6], "seconds": 4, "time": 0}))["clip"]
                    .as_u64()
                    .unwrap(),
            );
            let effect = item(&s, c).effects.iter().find(|e| filmcraft_project::graphic::is_layer(e)).unwrap().effect.clone();
            animate(&mut s, c, &effect, "position", None, [json!([8, 9]), json!([20, 9])]);
            animate(&mut s, c, "motion", "position", None, [json!([16, 9]), json!([20, 9])]);
            exec(&mut s, "graphics.setResponsiveTime", json!({"clip": c.0, "introSeconds": 1, "outroSeconds": 1}));
            exec(&mut s, "timeline.trim", json!({"clip": c.0, "edge": "out", "deltaFrames": 96}));
            exec(&mut s, "timeline.trim", json!({"clip": c.0, "edge": "in", "deltaFrames": 24}));
            if reverse {
                exec(&mut s, "clip.speedDuration", json!({"clips": [c.0], "speed": 200, "reverse": true}));
            }
            let i = item(&s, c);
            s.set_playhead(i.start + i.duration.mul_ratio(3, 4));
            exec(&mut s, "clip.frameHoldOptions", json!({"clips": [c.0], "holdOn": "in", "holdFilters": filters}));
            let app = FilmcraftApp::new(s);
            let picture = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 180.0));
            let shown = filmcraft_ui_egui::panels::graphics::visible_layers(&app, picture, (32, 18));
            assert_eq!(shown.len(), 1);
            // The designed middle is 2s stretched over 5s after the head trim. Motion
            // remains un-stretched: its 4px movement is evaluated on the raw effect clock.
            let (layer_x, screen_x) = if filters && reverse {
                // Holding the reversed clip's In frame reads the protected outro and
                // Motion's final 4px displacement, rather than the source's first frame.
                (20.0, 240.0)
            } else if filters {
                (8.0, 80.0)
            } else if reverse {
                (9.8, 133.0)
            } else {
                (18.2, 222.0)
            };
            assert!((shown[0].spec.transform.position.x - layer_x).abs() < 1e-6);
            let quad = shown[0].quad();
            let center_x = quad.iter().map(|p| p.x).sum::<f32>() / 4.0;
            let center_y = quad.iter().map(|p| p.y).sum::<f32>() / 4.0;
            assert!(
                (center_x as f64 - screen_x).abs() < 0.001 && (center_y - 90.0).abs() < 0.001,
                "hit geometry follows the rendered graphic and its Motion: {quad:?}"
            );
            let rendered = app.session.render_program_working(1.0).unwrap();
            assert!(rendered.get((screen_x / 10.0).round() as usize, 9)[0] > 0.05, "the hit box covers the rendered shape");
        }
    }
}
