//! Point graph edits through the real UI: each coordinate keeps its counterpart and the shared
//! temporal easing, every gesture is undoable, and scalar/mask graph controls keep their ids.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::{ClipId, ItemKind, Label, MediaClip, MediaRef, Param, ParamValue, Project, SequenceSettings};
use filmcraft_time::{FrameRate, Tick};
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    clip: u64,
    times: [i64; 2],
}

impl Driver {
    fn new() -> Self {
        let rate = FrameRate::FPS_24;
        let source = GeneratorSource::new(Generator::BlackVideo, 32, 18, rate, Tick::from_seconds_f64(8.0));
        let mut project = Project::new("Point graphs");
        let item = project.add_item(
            "Clip",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(Generator::BlackVideo),
                info: source.info().clone(),
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            None,
        );
        let seq = project.new_sequence("Cut", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 1, None);
        let mut s = Session::default();
        s.project = Arc::new(project);
        s.state.active_sequence = Some(seq);
        s.state.open_sequences = vec![seq];
        s.execute("timeline.place", json!({"item": item.0, "track": "V1", "time": 0, "insert": false})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        s.execute("timeline.select", json!({"clips": [clip]})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { harness, tx, clip, times: [0, 0] };
        d.frames(4);
        d.ok("ui.set", json!({"workspace": "Effects", "focused": "Effect Controls"}));
        d.ok("ui.panel.show", json!({"panel": "Effect Controls"}));
        d.ok("ui.menu.invoke", json!({"id": "window.maximizeFrame"}));
        d.animate("motion", "position", None, json!([800.0, 400.0]), json!([1200.0, 700.0]));
        d.times = d.param("motion", "position", None).keyframes.iter().map(|k| k.time.0).collect::<Vec<_>>().try_into().unwrap();
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], true, "{method} {params}: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn animate(&mut self, effect: &str, param: &str, mask: Option<usize>, a: Value, b: Value) {
        let item = self.harness.state().session.active_sequence().unwrap().find_item(ClipId(self.clip)).unwrap().1.clone();
        let mut p = json!({"clip": self.clip, "effect": effect, "param": param});
        if let Some(mask) = mask {
            p["mask"] = json!(mask);
        }
        self.exec("playhead.set", json!({"time": item.start.0 + item.duration.0 / 4}));
        p["value"] = a;
        self.exec("effects.setParam", p.clone());
        self.exec("effects.toggleAnimation", p.clone());
        self.exec("playhead.set", json!({"time": item.start.0 + item.duration.0 * 3 / 4}));
        self.exec("effects.addKeyframe", p.clone());
        p["value"] = b;
        self.exec("effects.setParam", p);
    }

    fn param(&self, effect: &str, param: &str, mask: Option<usize>) -> Param {
        let s = &self.harness.state().session;
        let effect = s.active_sequence().unwrap().find_item(ClipId(self.clip)).unwrap().1.effect(effect).unwrap();
        match mask {
            Some(mask) => effect.masks[mask].param(param).unwrap().clone(),
            None => effect.params[param].clone(),
        }
    }

    fn rect(&mut self, id: &str) -> [f64; 4] {
        // Engine replies can precede the paint recorded by previous-frame telemetry.
        self.frames(2);
        let v = self.ok("ui.elements", json!({"prefix": id}));
        let e = v.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| {
            let app = self.harness.state();
            let item = app.session.active_sequence().unwrap().find_item(ClipId(self.clip)).unwrap().1;
            let controls: Vec<_> = app.auto.previous.iter().filter(|e| e.id.starts_with("effectControls.")).collect();
            panic!(
                "no element {id}: {v}; selection={:?}; expanded={:?}; collapsed={:?}; clock=start {:?}, source {:?}, duration {:?}, speed {}, reverse {}, hold {:?}, hold_filters {}; position={:?}; controls={controls:?}; status={}",
                app.session.state.selection,
                app.ui.expanded_fx,
                app.ui.collapsed_fx,
                item.start,
                item.source_in,
                item.duration,
                item.speed,
                item.reverse,
                item.frame_hold,
                item.hold_filters,
                item.effects.iter().filter_map(|e| e.param("position").map(|p| (&e.effect, p))).collect::<Vec<_>>(),
                app.ui.status,
            )
        });
        let r = e["rect"].as_array().unwrap();
        std::array::from_fn(|i| r[i].as_f64().unwrap())
    }

    fn visible_rect(&mut self, id: &str) -> [f64; 4] {
        let panel = self.rect("panel.EffectControls");
        // The scroll area begins after the clip tabs and Video heading and ends above the footer.
        let (top, bottom) = (panel[1] + 50.0, panel[1] + panel[3] - 32.0);
        // The first effect header rests exactly on the scroll body's top at maximum scroll.
        // It is fully clickable there; graph controls still need the existing 24px margin.
        let margin = if id.starts_with("effectControls.effect.") { 0.0 } else { 24.0 };
        for _ in 0..12 {
            let r = self.rect(id);
            if r[1] >= top + margin && r[1] + r[3] <= bottom - margin {
                return r;
            }
            let dy = if r[1] < top + margin { 420.0 } else { -420.0 };
            self.ok("ui.scroll", json!({"x": panel[0] + 200.0, "y": (top + bottom) / 2.0, "dy": dy}));
            self.frames(8);
        }
        panic!("control {id} must be visible before its pointer action: {:?}, body={top}..{bottom}", self.rect(id));
    }

    fn open(&mut self, id: &str) {
        self.visible_rect(id);
        let before = self.harness.state().ui.expanded_fx.clone();
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
        if id.ends_with(".graphs") {
            assert_ne!(self.harness.state().ui.expanded_fx, before, "visible graph control {id} must toggle its expansion");
        }
    }

    fn drag(&mut self, id: &str, dx: f64, dy: f64) {
        let r = self.visible_rect(id);
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        self.ok("ui.drag", json!({"from": {"x": x, "y": y}, "to": {"x": x + dx, "y": y + dy}, "steps": 6}));
        self.frames(3);
    }

    fn undo_count(&mut self) -> usize {
        self.exec("history.list", json!({}))["undo"].as_array().unwrap().len()
    }

    fn bezier(&mut self) {
        for time in self.times {
            self.exec(
                "effects.setInterpolation",
                json!({"clip": self.clip, "effect": "motion", "param": "position", "mediaTime": time, "interpolation": "bezier"}),
            );
            self.exec(
                "effects.setKeyframe",
                json!({"clip": self.clip, "effect": "motion", "param": "position", "mediaTime": time, "inInfluence": 0.4, "outInfluence": 0.25}),
            );
        }
    }
}

#[test]
fn point_graphs_edit_each_coordinate_and_undo_one_gesture() {
    let mut d = Driver::new();
    d.bezier();
    d.open("effectControls.motion.position.graphs");
    for axis in ["x", "y"] {
        let value = d.rect(&format!("effectControls.position.graph.{axis}.value"));
        let velocity = d.rect(&format!("effectControls.position.graph.{axis}.velocity"));
        assert!(value.iter().chain(&velocity).all(|v| v.is_finite()));
        assert!(value[2] > 100.0 && value[3] > 90.0 && velocity[3] > 50.0, "{value:?} {velocity:?}");
    }
    for axis in ["x", "y"] {
        let before = d.param("motion", "position", None);
        let undo = d.undo_count();
        d.drag(&format!("effectControls.position.graph.{axis}.keyframe.{}", d.times[0]), 0.0, -10.0);
        let after = d.param("motion", "position", None);
        let a = before.keyframes[0].value.as_vec2().unwrap();
        let b = after.keyframes[0].value.as_vec2().unwrap();
        if axis == "x" {
            assert!(b.x > a.x, "X moved: {a:?} → {b:?}");
            assert_eq!(b.y, a.y);
        } else {
            assert!(b.y > a.y, "Y moved: {a:?} → {b:?}");
            assert_eq!(b.x, a.x);
        }
        assert_eq!(after.keyframes[0].time, before.keyframes[0].time);
        assert_eq!(after.keyframes[0].interp, before.keyframes[0].interp);
        assert_eq!(after.keyframes[0].in_influence, before.keyframes[0].in_influence);
        assert_eq!(after.keyframes[0].out_influence, before.keyframes[0].out_influence);
        assert_eq!(after.keyframes[1], before.keyframes[1]);
        assert_eq!(after.value, before.value);
        assert_eq!(d.undo_count(), undo + 1, "one undo per graph gesture");
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param("motion", "position", None), before);
        d.exec("edit.redo", json!({}));
        assert_eq!(d.param("motion", "position", None), after);
    }
}

#[test]
fn point_influence_handles_change_shared_easing_without_changing_values() {
    let mut d = Driver::new();
    d.bezier();
    d.open("effectControls.motion.position.graphs");
    let before = d.param("motion", "position", None);
    let undo = d.undo_count();
    let sample_time = before.keyframes[0].time + (before.keyframes[1].time - before.keyframes[0].time).mul_ratio(1, 3);
    d.drag(&format!("effectControls.position.graph.x.keyframe.{}.out", d.times[0]), 35.0, 0.0);
    let after = d.param("motion", "position", None);
    for (a, b) in before.keyframes.iter().zip(&after.keyframes) {
        assert_eq!(a.value, b.value);
        assert_eq!(a.interp, b.interp);
        assert_eq!(a.in_influence, b.in_influence);
        assert_eq!(a.time, b.time);
    }
    assert!(after.keyframes[0].out_influence > before.keyframes[0].out_influence);
    assert_eq!(after.keyframes[1], before.keyframes[1]);
    let (a, b) = (before.vec2_at(sample_time), after.vec2_at(sample_time));
    assert_ne!(a, b, "handle changes the evaluated curve");
    assert!(((b.x - 800.0) / 400.0 - (b.y - 400.0) / 300.0).abs() < 1e-12, "both axes share temporal easing");
    let x = d.rect(&format!("effectControls.position.graph.x.keyframe.{}.out", d.times[0]));
    let y = d.rect(&format!("effectControls.position.graph.y.keyframe.{}.out", d.times[0]));
    assert!((x[0] - y[0]).abs() < 0.01, "both axes show the updated influence: {x:?} {y:?}");
    assert_eq!(d.undo_count(), undo + 1);
    d.exec("edit.undo", json!({}));
    assert_eq!(d.param("motion", "position", None), before);
    d.exec("edit.redo", json!({}));
    assert_eq!(d.param("motion", "position", None), after);
}

#[test]
fn float_and_mask_graphs_keep_their_controls_and_undo() {
    let mut d = Driver::new();
    d.animate("motion", "scale", None, json!(70.0), json!(120.0));
    d.open("effectControls.motion.scale.graphs");
    d.rect("effectControls.scale.graph.value");
    let before = d.param("motion", "scale", None);
    d.drag(&format!("effectControls.scale.graph.keyframe.{}", before.keyframes[0].time.0), 0.0, -10.0);
    let after = d.param("motion", "scale", None);
    assert!(after.keyframes[0].value.as_f64().unwrap() > before.keyframes[0].value.as_f64().unwrap());
    d.exec("edit.undo", json!({}));
    assert_eq!(d.param("motion", "scale", None), before);
    d.open("effectControls.motion.scale.graphs"); // close to keep mask rows in view
    d.open("effectControls.effect.motion");
    d.exec("masks.add", json!({"clip": d.clip, "effect": "opacity", "shape": "ellipse"}));
    d.animate("opacity", "feather", Some(0), json!(20.0), json!(60.0));
    d.open("effectControls.opacity.mask0.feather.graphs");
    d.rect("effectControls.feather.graph.velocity");
    let before = d.param("opacity", "feather", Some(0));
    d.drag(&format!("effectControls.feather.graph.keyframe.{}", before.keyframes[0].time.0), 0.0, -10.0);
    let after = d.param("opacity", "feather", Some(0));
    assert!(matches!(after.keyframes[0].value, ParamValue::Float(_)));
    assert!(after.keyframes[0].value.as_f64().unwrap() > before.keyframes[0].value.as_f64().unwrap());
    assert_eq!(after.keyframes[1], before.keyframes[1]);
    d.exec("edit.undo", json!({}));
    assert_eq!(d.param("opacity", "feather", Some(0)), before);
}

#[test]
fn reverse_and_fast_point_keyframes_follow_sequence_positions_navigation_and_handles() {
    for reverse in [false, true] {
        let mut d = Driver::new();
        let clip = ClipId(d.clip);
        d.harness
            .state_mut()
            .session
            .edit_sequence("Fixture timing", |q, _, _| {
                let item = q.find_item_mut(clip).unwrap().1;
                item.source_in = Tick::from_seconds_f64(1.0);
                item.duration = Tick::from_seconds_f64(2.0);
                item.speed = 2.0;
                item.reverse = reverse;
                item.effect_mut("motion").unwrap().params.insert("position".into(), Param::new(ParamValue::Vec2(filmcraft_geom::Vec2::new(800.0, 400.0))));
                Ok(())
            })
            .unwrap();
        d.animate("motion", "position", None, json!([800.0, 400.0]), json!([1200.0, 700.0]));
        let before = d.param("motion", "position", None);
        d.times = before.keyframes.iter().map(|k| k.time.0).collect::<Vec<_>>().try_into().unwrap();
        d.bezier();
        d.open("effectControls.motion.position.graphs");
        let item = d.harness.state().session.active_sequence().unwrap().find_item(clip).unwrap().1.clone();
        let lane = d.rect("effectControls.lane");
        for fraction in [0.25, 0.75] {
            let time = item.start + Tick((item.duration.0 as f64 * fraction) as i64);
            let media = item.effect_time_at(time);
            for id in [
                format!("effectControls.motion.position.keyframe.{}", media.0),
                format!("effectControls.position.graph.x.keyframe.{}", media.0),
                format!("effectControls.position.graph.y.keyframe.{}", media.0),
            ] {
                let r = d.rect(&id);
                assert!((r[0] + r[2] / 2.0 - (lane[0] + lane[2] * fraction)).abs() < 0.05, "{id} follows sequence time, reverse={reverse}");
            }
        }
        d.exec("playhead.set", json!({"seconds": 1.0}));
        d.open("effectControls.motion.position.prevKeyframe");
        assert_eq!(d.harness.state().session.playhead(), Tick::from_seconds_f64(0.5));
        d.exec("playhead.set", json!({"seconds": 1.0}));
        d.open("effectControls.motion.position.nextKeyframe");
        assert_eq!(d.harness.state().session.playhead(), Tick::from_seconds_f64(1.5));
        let before = d.param("motion", "position", None);
        let earlier_media = item.effect_time_at(Tick::from_seconds_f64(0.5));
        let earlier_key = before.keyframes.iter().find(|k| k.time == earlier_media).unwrap().clone();
        let undo = d.undo_count();
        d.drag(&format!("effectControls.motion.position.keyframe.{}", earlier_media.0), lane[2] / 8.0, 0.0);
        let after = d.param("motion", "position", None);
        let moved_media = item.effect_time_at(Tick::from_seconds_f64(0.75));
        let moved = after.keyframes.iter().find(|k| k.time == moved_media).unwrap();
        assert_eq!(moved.value, earlier_key.value);
        assert_eq!(moved.interp, earlier_key.interp);
        assert_eq!(moved.in_influence, earlier_key.in_influence);
        assert_eq!(moved.out_influence, earlier_key.out_influence);
        assert_eq!(d.undo_count(), undo + 1);
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param("motion", "position", None), before);
        let key_id = format!("effectControls.position.graph.x.keyframe.{}", d.times[0]);
        let out_id = format!("{key_id}.out");
        let key = d.rect(&key_id);
        let out = d.rect(&out_id);
        assert!(if reverse { out[0] < key[0] } else { out[0] > key[0] }, "source-out influence points toward its sequence neighbor");
        d.drag(&out_id, if reverse { -25.0 } else { 25.0 }, 0.0);
        let after = d.param("motion", "position", None);
        assert!(after.keyframes[0].out_influence > before.keyframes[0].out_influence);
        assert_eq!(after.keyframes[0].in_influence, before.keyframes[0].in_influence);
        assert_eq!(after.keyframes[0].value, before.keyframes[0].value);
        assert_eq!(after.keyframes[0].interp, before.keyframes[0].interp);
        assert_eq!(after.keyframes[1], before.keyframes[1]);
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param("motion", "position", None), before);
        d.drag(&key_id, 0.0, -8.0);
        let after = d.param("motion", "position", None);
        let a = before.keyframes[0].value.as_vec2().unwrap();
        let b = after.keyframes[0].value.as_vec2().unwrap();
        assert!(b.x > a.x);
        assert_eq!(b.y, a.y);
        assert_eq!(after.keyframes[0].time, before.keyframes[0].time);
        assert_eq!(after.keyframes[0].interp, before.keyframes[0].interp);
        assert_eq!(after.keyframes[0].in_influence, before.keyframes[0].in_influence);
        assert_eq!(after.keyframes[0].out_influence, before.keyframes[0].out_influence);
        assert_eq!(after.keyframes[1], before.keyframes[1]);
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param("motion", "position", None), before);
    }
}

fn render_graphic(d: &Driver, timeline: Tick) -> filmcraft_render::Image {
    let session = &d.harness.state().session;
    let mut project = (*session.project).clone();
    let sequence = session.state.active_sequence.unwrap();
    // Isolate the graphic fixture but use the compositor: render_clip deliberately omits
    // intrinsic Opacity, which is applied only when its layer is blended into the sequence.
    for track in &mut project.sequence_mut(sequence).unwrap().video_tracks {
        track.items.retain(|item| item.id == ClipId(d.clip));
        track.transitions.clear();
    }
    filmcraft_render::render_sequence(&project, sequence, timeline, filmcraft_render::RenderOptions::default(), &|_| None)
}

fn image_center_x(image: &filmcraft_render::Image) -> f64 {
    let mut mass = 0.0;
    let mut position = 0.0;
    for y in 0..image.h {
        for x in 0..image.w {
            let alpha = f64::from(image.get(x, y)[3]);
            mass += alpha;
            position += (x as f64 + 0.5) * alpha;
        }
    }
    assert!(mass > 1.0);
    position / mass
}

#[test]
fn responsive_graphic_points_keep_protected_timing_in_navigation_drag_and_rendering() {
    use filmcraft_geom::Vec2;
    use filmcraft_project::{Interpolation, Keyframe};
    use filmcraft_render::graphic_clip::parameter_time;
    for reverse in [false, true] {
        let mut d = Driver::new();
        d.exec("playhead.set", json!({"time": 0}));
        d.clip =
            d.exec("graphics.newShape", json!({"shape": "rectangle", "position": [8, 9], "size": [6, 6], "seconds": 8, "time": 0}))["clip"].as_u64().unwrap();
        d.exec("graphics.setResponsiveTime", json!({"clip": d.clip, "introSeconds": 1, "outroSeconds": 1}));
        d.exec("timeline.trim", json!({"clip": d.clip, "edge": "out", "deltaFrames": 192}));
        d.exec("clip.speedDuration", json!({"clips": [d.clip], "speed": 200, "reverse": reverse}));
        let clip = ClipId(d.clip);
        let item = d.harness.state().session.active_sequence().unwrap().find_item(clip).unwrap().1.clone();
        assert_eq!(item.duration, Tick::from_seconds_f64(8.0));
        let layer = item.effects.iter().position(filmcraft_project::graphic::is_layer).unwrap();
        let effect = item.effects[layer].effect.clone();
        let early = item.start + Tick::from_seconds_f64(0.5);
        let late = item.start + Tick::from_seconds_f64(7.5);
        let clocks = [early, late].map(|t| parameter_time(&item, &item.effects[layer], item.effect_time_at(t)));
        let mut keys = vec![Keyframe::new(clocks[0], ParamValue::Vec2(Vec2::new(8.0, 9.0))), Keyframe::new(clocks[1], ParamValue::Vec2(Vec2::new(20.0, 9.0)))];
        keys.sort_by_key(|k| k.time);
        d.harness
            .state_mut()
            .session
            .edit_sequence("Designed graphic keys", |q, _, _| {
                let it = q.find_item_mut(clip).unwrap().1;
                it.effects[layer].params.insert("position".into(), Param { value: keys[0].value.clone(), keyframes: keys.clone() });
                Ok(())
            })
            .unwrap();
        // Keep the established effect layout; collapse the intrinsic rows so the layer's
        // Position row and graphs can be scrolled into view in an ordinary-height window.
        for (idx, e) in item.effects.iter().enumerate() {
            if e.def().unwrap().intrinsic {
                d.harness.state_mut().ui.collapsed_fx.push(format!("{}:{idx}", d.clip));
            }
        }
        d.exec("playhead.set", json!({"time": item.start.0 + item.duration.0 / 2}));
        d.frames(4);
        d.open(&format!("effectControls.{effect}.position.graphs"));
        d.ok("ui.scroll", json!({"x": 200, "y": 600, "dy": -420}));
        d.frames(8);
        let lane = d.rect("effectControls.lane");
        for (time, clock) in [(early, clocks[0]), (late, clocks[1])] {
            let fraction = (time - item.start).0 as f64 / item.duration.0 as f64;
            for id in [
                format!("effectControls.{effect}.position.keyframe.{}", clock.0),
                format!("effectControls.position.graph.x.keyframe.{}", clock.0),
                format!("effectControls.position.graph.y.keyframe.{}", clock.0),
            ] {
                let r = d.rect(&id);
                assert!((r[0] + r[2] / 2.0 - (lane[0] + lane[2] * fraction)).abs() < 0.05, "{id} retains protected sequence timing");
            }
        }
        d.open(&format!("effectControls.{effect}.position.prevKeyframe"));
        assert!((d.harness.state().session.playhead().0 - early.0).abs() <= 2);
        d.exec("playhead.set", json!({"time": item.start.0 + item.duration.0 / 2}));
        d.open(&format!("effectControls.{effect}.position.nextKeyframe"));
        assert!((d.harness.state().session.playhead().0 - late.0).abs() <= 2);
        // The tiny actual render agrees with the mapped layer value, including the stretched
        // middle; using the raw Motion clock would put this shape at a different position.
        let before = d.param(&effect, "position", None);
        let sample_time = item.start + Tick::from_seconds_f64(2.0);
        let raw = item.effect_time_at(sample_time);
        let clock = parameter_time(&item, &item.effects[layer], raw);
        let position = before.value_at(clock).as_vec2().unwrap();
        assert!((image_center_x(&render_graphic(&d, sample_time)) - position.x).abs() < 0.2);
        assert!((position.x - before.value_at(raw).as_vec2().unwrap().x).abs() > 1.0);
        let before_image = render_graphic(&d, early);
        let old_key = before.keyframes.iter().find(|k| k.time == clocks[0]).unwrap().clone();
        let undo = d.undo_count();
        d.drag(&format!("effectControls.{effect}.position.keyframe.{}", clocks[0].0), lane[2] / 8.0, 0.0);
        let moved_time = early + Tick::from_seconds_f64(1.0);
        let moved_clock = parameter_time(&item, &item.effects[layer], item.effect_time_at(moved_time));
        let after = d.param(&effect, "position", None);
        let moved = after.keyframes.iter().find(|k| k.time == moved_clock).unwrap();
        assert_eq!(moved.value, old_key.value);
        assert_eq!(moved.interp, old_key.interp);
        assert_eq!(moved.in_influence, old_key.in_influence);
        assert_eq!(moved.out_influence, old_key.out_influence);
        assert_eq!(d.undo_count(), undo + 1);
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param(&effect, "position", None), before);
        assert_eq!(render_graphic(&d, early), before_image);
        for time in clocks {
            d.exec("effects.setInterpolation", json!({"clip": d.clip, "effect": layer, "param": "position", "mediaTime": time.0, "interpolation": "bezier"}));
            d.exec(
                "effects.setKeyframe",
                json!({"clip": d.clip, "effect": layer, "param": "position", "mediaTime": time.0, "inInfluence": 0.4, "outInfluence": 0.25}),
            );
        }
        let before = d.param(&effect, "position", None);
        let source_first = before.keyframes[0].time;
        let key_id = format!("effectControls.position.graph.x.keyframe.{}", source_first.0);
        let key = d.rect(&key_id);
        let handle = d.rect(&format!("{key_id}.out"));
        assert!(if reverse { handle[0] < key[0] } else { handle[0] > key[0] });
        d.drag(&format!("{key_id}.out"), if reverse { -25.0 } else { 25.0 }, 0.0);
        let after = d.param(&effect, "position", None);
        assert!(after.keyframes[0].out_influence > before.keyframes[0].out_influence);
        assert_eq!(after.keyframes[0].in_influence, before.keyframes[0].in_influence);
        assert_eq!(after.keyframes[0].value, before.keyframes[0].value);
        assert_eq!(after.keyframes[0].interp, Interpolation::Bezier);
        assert_eq!(after.keyframes[1], before.keyframes[1]);
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param(&effect, "position", None), before);
        let before_image = render_graphic(&d, if reverse { late } else { early });
        let undo = d.undo_count();
        d.drag(&key_id, 0.0, -10.0);
        let after = d.param(&effect, "position", None);
        let a = before.keyframes[0].value.as_vec2().unwrap();
        let b = after.keyframes[0].value.as_vec2().unwrap();
        assert!(b.x > a.x);
        assert_eq!(b.y, a.y);
        assert_eq!(after.keyframes[0].time, before.keyframes[0].time);
        assert_eq!(after.keyframes[0].interp, before.keyframes[0].interp);
        assert_eq!(after.keyframes[0].in_influence, before.keyframes[0].in_influence);
        assert_eq!(after.keyframes[0].out_influence, before.keyframes[0].out_influence);
        assert_eq!(after.keyframes[1], before.keyframes[1]);
        assert_ne!(render_graphic(&d, if reverse { late } else { early }), before_image);
        assert_eq!(d.undo_count(), undo + 1);
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param(&effect, "position", None), before);
        assert_eq!(render_graphic(&d, if reverse { late } else { early }), before_image);
    }
}

#[test]
fn shortened_responsive_graphic_navigation_skips_unreachable_design_keys() {
    use filmcraft_geom::Vec2;
    use filmcraft_project::Keyframe;
    for reverse in [false, true] {
        let mut d = Driver::new();
        d.exec("playhead.set", json!({"time": 0}));
        d.clip =
            d.exec("graphics.newShape", json!({"shape": "rectangle", "position": [8, 9], "size": [6, 6], "seconds": 8, "time": 0}))["clip"].as_u64().unwrap();
        d.exec("graphics.setResponsiveTime", json!({"clip": d.clip, "introSeconds": 1, "outroSeconds": 1}));
        d.exec("timeline.trim", json!({"clip": d.clip, "edge": "out", "deltaFrames": -144}));
        d.exec("clip.speedDuration", json!({"clips": [d.clip], "speed": 200, "reverse": reverse}));
        let clip = ClipId(d.clip);
        let item = d.harness.state().session.active_sequence().unwrap().find_item(clip).unwrap().1.clone();
        assert_eq!(item.duration, Tick::from_seconds_f64(1.0));
        let layer = item.effects.iter().position(filmcraft_project::graphic::is_layer).unwrap();
        let effect = item.effects[layer].effect.clone();
        let times = [0.5, 2.0, 4.0, 7.5].map(Tick::from_seconds_f64);
        let keys: Vec<_> = times.into_iter().zip([8.0, 10.0, 12.0, 20.0]).map(|(t, x)| Keyframe::new(t, ParamValue::Vec2(Vec2::new(x, 9.0)))).collect();
        d.harness
            .state_mut()
            .session
            .edit_sequence("Designed shortened graphic keys", |q, _, _| {
                q.find_item_mut(clip).unwrap().1.effects[layer]
                    .params
                    .insert("position".into(), Param { value: keys[0].value.clone(), keyframes: keys.clone() });
                Ok(())
            })
            .unwrap();
        for (idx, e) in item.effects.iter().enumerate() {
            if e.def().unwrap().intrinsic {
                d.harness.state_mut().ui.collapsed_fx.push(format!("{}:{idx}", d.clip));
            }
        }
        d.frames(4);
        d.open(&format!("effectControls.{effect}.position.graphs"));
        d.ok("ui.scroll", json!({"x": 200, "y": 600, "dy": -420}));
        d.frames(8);
        let graph_keys = d.ok("ui.elements", json!({"prefix": "effectControls.position.graph.x.keyframe."}));
        for time in [times[0], times[3]] {
            let id = format!("effectControls.position.graph.x.keyframe.{}", time.0);
            assert!(
                graph_keys.as_array().unwrap().iter().any(|e| e["id"] == id),
                "reachable key {id} has a graph position, reverse={reverse}; graph_keys={graph_keys}; expanded={:?}; collapsed={:?}; item={item:?}",
                d.harness.state().ui.expanded_fx,
                d.harness.state().ui.collapsed_fx,
            );
        }
        for time in [times[1], times[2]] {
            let id = format!("effectControls.position.graph.x.keyframe.{}", time.0);
            assert!(!graph_keys.as_array().unwrap().iter().any(|e| e["id"] == id), "unreachable middle key has no fabricated graph position");
        }
        let project = d.harness.state().session.project.clone();
        let undo = d.undo_count();
        // The layer jumps between design1s and7s. On either side, one direction must skip
        // both middle design keys; reversing playback swaps their media search directions.
        for current in [0.375, 0.625] {
            for (direction, expected) in [("prevKeyframe", 0.25), ("nextKeyframe", 0.75)] {
                d.exec("playhead.set", json!({"seconds": current}));
                d.open(&format!("effectControls.{effect}.position.{direction}"));
                let expected = Tick::from_seconds_f64(expected);
                assert!(
                    (d.harness.state().session.playhead().0 - expected.0).abs() <= 2,
                    "{direction} skips unreachable keys, current={current}, reverse={reverse}"
                );
                assert!(Arc::ptr_eq(&project, &d.harness.state().session.project), "navigation only moves the playhead");
                assert_eq!(d.undo_count(), undo);
            }
        }
        let before = d.param(&effect, "position", None);
        let shown = if reverse { Tick::from_seconds_f64(0.75) } else { Tick::from_seconds_f64(0.25) };
        let before_image = render_graphic(&d, shown);
        d.drag(&format!("effectControls.position.graph.x.keyframe.{}", times[0].0), 0.0, -10.0);
        let after = d.param(&effect, "position", None);
        assert!(after.keyframes[0].value.as_vec2().unwrap().x > before.keyframes[0].value.as_vec2().unwrap().x);
        assert_eq!(after.keyframes[0].value.as_vec2().unwrap().y, before.keyframes[0].value.as_vec2().unwrap().y);
        assert_eq!(after.keyframes[0].time, before.keyframes[0].time);
        assert_eq!(after.keyframes[0].interp, before.keyframes[0].interp);
        assert_eq!(after.keyframes[0].in_influence, before.keyframes[0].in_influence);
        assert_eq!(after.keyframes[0].out_influence, before.keyframes[0].out_influence);
        assert_eq!(&after.keyframes[1..], &before.keyframes[1..], "unreachable design keys are preserved");
        assert_ne!(render_graphic(&d, shown), before_image);
        assert_eq!(d.undo_count(), undo + 1);
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param(&effect, "position", None), before);
        assert_eq!(render_graphic(&d, shown), before_image);
    }
}

#[test]
fn mixed_interpolation_incoming_handles_edit_render_and_undo_for_points_and_scalars() {
    for reverse in [false, true] {
        for point in [false, true] {
            let mut d = Driver::new();
            d.exec("playhead.set", json!({"time": 0}));
            d.clip = d.exec("graphics.newShape", json!({"shape": "rectangle", "position": [16, 9], "size": [6, 6], "seconds": 8, "time": 0}))["clip"]
                .as_u64()
                .unwrap();
            d.exec("clip.speedDuration", json!({"clips": [d.clip], "speed": 200, "reverse": reverse}));
            d.exec("timeline.select", json!({"clips": [d.clip]}));
            let item = d.harness.state().session.active_sequence().unwrap().find_item(ClipId(d.clip)).unwrap().1.clone();
            let layer = item.effects.iter().position(filmcraft_project::graphic::is_layer).unwrap();
            let effect = if point { item.effects[layer].effect.clone() } else { "opacity".to_string() };
            let parameter = if point { "position" } else { "opacity" };
            let values = if point { [json!([8, 9]), json!([20, 9])] } else { [json!(20), json!(80)] };
            d.animate(&effect, parameter, None, values[0].clone(), values[1].clone());
            let keys = d.param(&effect, parameter, None).keyframes;
            let (first, last) = (keys[0].time, keys[1].time);
            d.exec("effects.setInterpolation", json!({"clip": d.clip, "effect": effect, "param": parameter, "mediaTime": first.0, "interpolation": "bezier"}));
            d.exec("effects.setKeyframe", json!({"clip": d.clip, "effect": effect, "param": parameter, "mediaTime": first.0, "outInfluence": 0.25}));
            d.exec("effects.setKeyframe", json!({"clip": d.clip, "effect": effect, "param": parameter, "mediaTime": last.0, "inInfluence": 0.25}));
            for (index, fx) in item.effects.iter().enumerate() {
                if fx.effect != effect {
                    d.harness.state_mut().ui.collapsed_fx.push(format!("{}:{index}", d.clip));
                }
            }
            d.frames(4);
            d.ok("ui.scroll", json!({"x": 200, "y": 300, "dy": 10000}));
            d.open(&format!("effectControls.{effect}.{parameter}.graphs"));
            let graph = if point { "effectControls.position.graph.x" } else { "effectControls.opacity.graph" };
            let key_id = format!("{graph}.keyframe.{}", last.0);
            let incoming_id = format!("{key_id}.in");
            let key = d.rect(&key_id);
            let incoming = d.rect(&incoming_id);
            assert!(if reverse { incoming[0] > key[0] } else { incoming[0] < key[0] }, "incoming handle remains reachable toward the sequence neighbour");
            if point {
                d.rect(&format!("effectControls.position.graph.y.keyframe.{}.in", last.0));
            }
            let before = d.param(&effect, parameter, None);
            assert_eq!(before.keyframes[1].interp, filmcraft_project::Interpolation::Linear);
            let sample_time = item.start + item.duration.mul_ratio(1, 2);
            let sample_clock = item.effect_time_at(sample_time);
            let before_image = render_graphic(&d, sample_time);
            assert_eq!((before_image.w, before_image.h), (32, 18));
            assert!(before_image.px.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0.0), "the selected graphic must supply visible pixels before editing");
            let undo = d.undo_count();
            d.drag(&incoming_id, if reverse { 25.0 } else { -25.0 }, 0.0);
            let after = d.param(&effect, parameter, None);
            assert!(after.keyframes[1].in_influence > before.keyframes[1].in_influence);
            assert_eq!(after.keyframes[0], before.keyframes[0]);
            assert_eq!(after.value, before.value);
            assert_eq!(after.keyframes[1].time, before.keyframes[1].time);
            assert_eq!(after.keyframes[1].value, before.keyframes[1].value);
            assert_eq!(after.keyframes[1].interp, before.keyframes[1].interp);
            assert_eq!(after.keyframes[1].out_influence, before.keyframes[1].out_influence);
            assert_ne!(after.value_at(sample_clock), before.value_at(sample_clock), "handle changes the evaluated easing");
            let after_image = render_graphic(&d, sample_time);
            assert_ne!(after_image, before_image, "actual 32x18 rendered shape follows the incoming influence edit");
            if point {
                assert!((image_center_x(&after_image) - image_center_x(&before_image)).abs() > 0.05);
            }
            assert_eq!(d.undo_count(), undo + 1, "one undo per incoming handle gesture");
            d.exec("edit.undo", json!({}));
            assert_eq!(d.param(&effect, parameter, None), before);
            assert_eq!(render_graphic(&d, sample_time), before_image);
            d.exec("edit.redo", json!({}));
            assert_eq!(d.param(&effect, parameter, None), after);
            assert_eq!(render_graphic(&d, sample_time), after_image);
        }
    }
}

fn graphic_graph_fixture(d: &mut Driver) -> (String, Param) {
    d.exec("file.newSequence", json!({"width": 32, "height": 18, "fps": 24, "video": 1, "audio": 0}));
    d.clip = d.exec("graphics.newShape", json!({"shape": "rectangle", "position": [8, 9], "size": [6, 6], "seconds": 8, "time": 0}))["clip"].as_u64().unwrap();
    d.exec("timeline.select", json!({"clips": [d.clip]}));
    let item = d.harness.state().session.active_sequence().unwrap().find_item(ClipId(d.clip)).unwrap().1.clone();
    let layer = item.effects.iter().position(filmcraft_project::graphic::is_layer).unwrap();
    let effect = item.effects[layer].effect.clone();
    d.animate(&effect, "position", None, json!([8, 9]), json!([20, 9]));
    let before = d.param(&effect, "position", None);
    for key in &before.keyframes {
        d.exec("effects.setInterpolation", json!({"clip": d.clip, "effect": effect, "param": "position", "mediaTime": key.time.0, "interpolation": "bezier"}));
        d.exec(
            "effects.setKeyframe",
            json!({"clip": d.clip, "effect": effect, "param": "position", "mediaTime": key.time.0, "inInfluence": 0.25, "outInfluence": 0.25}),
        );
    }
    for (idx, fx) in item.effects.iter().enumerate() {
        if fx.effect != effect {
            d.harness.state_mut().ui.collapsed_fx.push(format!("{}:{idx}", d.clip));
        }
    }
    d.ok("ui.set", json!({"workspace": "Effects", "focused": "Effect Controls"}));
    d.ok("ui.panel.show", json!({"panel": "Effect Controls"}));
    if d.harness.state().ui.keys.maximized.is_none() {
        d.ok("ui.menu.invoke", json!({"id": "window.maximizeFrame"}));
    }
    d.frames(4);
    let graph_key = format!("graph:{}:{layer}:position", d.clip);
    if !d.harness.state().ui.expanded_fx.contains(&graph_key) {
        d.open(&format!("effectControls.{effect}.position.graphs"));
    }
    d.exec("playhead.set", json!({"seconds": 4}));
    (effect.clone(), d.param(&effect, "position", None))
}
fn pointer(d: &mut Driver, events: Vec<egui::Event>) {
    d.harness.input_mut().events.extend(events);
    d.frames(1);
}
fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

#[test]
fn hidden_release_and_new_project_discard_graph_time_value_and_influence_previews() {
    for replacement in [false, true] {
        for gesture in ["time", "value", "influence"] {
            let mut d = Driver::new();
            d.exec("file.newProject", json!({"name": "Original graph gesture"}));
            let (mut effect, before) = graphic_graph_fixture(&mut d);
            let original_clip = d.clip;
            let first = before.keyframes[0].time.0;
            let key_id = match gesture {
                "time" => format!("effectControls.{effect}.position.keyframe.{first}"),
                "value" => format!("effectControls.position.graph.x.keyframe.{first}"),
                _ => format!("effectControls.position.graph.x.keyframe.{first}.out"),
            };
            let initial = d.visible_rect(&key_id);
            let start = egui::pos2((initial[0] + initial[2] / 2.0) as f32, (initial[1] + initial[3] / 2.0) as f32);
            let offset = if gesture == "value" { egui::vec2(0.0, -20.0) } else { egui::vec2(20.0, 0.0) };
            pointer(&mut d, vec![egui::Event::PointerMoved(start), button(start, true)]);
            pointer(&mut d, vec![egui::Event::PointerMoved(start + offset)]);
            d.frames(2);
            let preview = d.rect(&key_id);
            let axis = usize::from(gesture == "value");
            assert!(
                (preview[axis] - initial[axis]).abs() > 10.0,
                "real pointer drag must display a provisional offset: {gesture}, replacement={replacement}, initial={initial:?}, preview={preview:?}, start={start:?}, offset={offset:?}, pointer={:?}, focused={:?}, maximized={:?}, expanded={:?}, collapsed={:?}, panels={:?}, status={}",
                d.harness.ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down())),
                d.harness.state().ui.focused,
                d.harness.state().ui.keys.maximized,
                d.harness.state().ui.expanded_fx,
                d.harness.state().ui.collapsed_fx,
                d.harness.state().auto.previous.iter().filter(|e| e.id.starts_with("panel.")).collect::<Vec<_>>(),
                d.harness.state().ui.status,
            );
            assert_eq!(d.param(&effect, "position", None), before, "preview must not commit before release");
            let undo = d.undo_count();
            if replacement {
                d.exec("file.newProject", json!({"name": "Replacement graph gesture"}));
            } else {
                d.ok("ui.menu.invoke", json!({"id": "view.programFullScreen"}));
            }
            // The graph is hidden for the release frame, so its Response cannot observe
            // drag_stopped. Resuming must discard that preview, including reused document ids.
            pointer(&mut d, vec![button(start + offset, false)]);
            d.frames(3);
            let expected = if replacement {
                let next = graphic_graph_fixture(&mut d);
                effect = next.0;
                assert_eq!(d.clip, original_clip, "replacement deliberately reuses the numeric clip id");
                assert_eq!(next.1, before, "new document has the same visible key fixture");
                next.1
            } else {
                d.ok("ui.key", json!({"key": "Escape"}));
                assert_eq!(d.undo_count(), undo);
                before.clone()
            };
            let restored = d.rect(&key_id);
            assert!(
                initial.iter().zip(&restored).all(|(a, b)| (a - b).abs() < 0.05),
                "hidden release must restore committed geometry: {gesture}, replacement={replacement}, initial={initial:?}, returned={restored:?}"
            );
            assert_eq!(d.param(&effect, "position", None), expected);
            let sample = Tick::from_seconds_f64(if gesture == "value" { 2.0 } else { 4.0 });
            let before_image = render_graphic(&d, sample);
            let undo = d.undo_count();
            let lane = d.rect("effectControls.lane");
            let (dx, dy) = match gesture {
                "time" => (lane[2] / 8.0, 0.0),
                "value" => (0.0, -8.0),
                _ => (25.0, 0.0),
            };
            d.drag(&key_id, dx, dy);
            let changed = d.param(&effect, "position", None);
            assert_ne!(changed, expected, "a fresh gesture remains editable after cancellation");
            assert_eq!(d.undo_count(), undo + 1);
            assert_ne!(render_graphic(&d, sample), before_image);
            d.exec("edit.undo", json!({}));
            assert_eq!(d.param(&effect, "position", None), expected);
            assert_eq!(render_graphic(&d, sample), before_image);
        }
    }
}

#[test]
fn graph_time_value_and_influence_previews_commit_once_on_visible_release() {
    for gesture in ["time", "value", "influence"] {
        let mut d = Driver::new();
        d.exec("file.newProject", json!({"name": "Visible graph release"}));
        let (effect, before) = graphic_graph_fixture(&mut d);
        // Resolve intrinsic auto points before exact project snapshots, preserving their center.
        for name in ["position", "anchor"] {
            d.exec("effects.setParam", json!({"clip": d.clip, "effect": "motion", "param": name, "value": [16, 9]}));
        }
        let first = before.keyframes[0].time.0;
        let key_id = match gesture {
            "time" => format!("effectControls.{effect}.position.keyframe.{first}"),
            "value" => format!("effectControls.position.graph.x.keyframe.{first}"),
            _ => format!("effectControls.position.graph.x.keyframe.{first}.out"),
        };
        let initial = d.visible_rect(&key_id);
        let lane = d.rect("effectControls.lane");
        let start = egui::pos2((initial[0] + initial[2] / 2.0) as f32, (initial[1] + initial[3] / 2.0) as f32);
        let offset = match gesture {
            "time" => egui::vec2((lane[2] / 8.0) as f32, 0.0),
            "value" => egui::vec2(0.0, -20.0),
            _ => egui::vec2(25.0, 0.0),
        };
        let sample = Tick::from_seconds_f64(if gesture == "value" { 2.0 } else { 4.0 });
        let before_image = render_graphic(&d, sample);
        assert!(before_image.px.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0.0), "the ordinary release fixture renders visible pixels");
        let project = d.harness.state().session.project.clone();
        let undo = d.undo_count();
        pointer(&mut d, vec![egui::Event::PointerMoved(start), button(start, true)]);
        // Paint again while held: previous-frame telemetry alone could hide a start jump.
        d.frames(1);
        let pressed = d.rect(&key_id);
        assert!(
            initial.iter().zip(&pressed).all(|(a, b)| (a - b).abs() < 0.05),
            "the batched {gesture} press must not move its key or handle: initial={initial:?}, pressed={pressed:?}"
        );
        assert_eq!(d.param(&effect, "position", None), before);
        assert_eq!(d.harness.state().session.project, project);
        assert_eq!(d.undo_count(), undo);
        pointer(&mut d, vec![egui::Event::PointerMoved(start + offset)]);
        d.frames(2);
        let preview = d.rect(&key_id);
        let axis = usize::from(gesture == "value");
        assert!((preview[axis] - initial[axis]).abs() > 10.0, "the held pointer must display an ordinary {gesture} preview");
        let expected_offset = f64::from(if gesture == "value" { offset.y } else { offset.x });
        assert!(
            (preview[axis] - initial[axis] - expected_offset).abs() < 0.05,
            "the {gesture} preview must follow only motion since the press: initial={initial:?}, preview={preview:?}, offset={offset:?}"
        );
        assert_eq!(d.param(&effect, "position", None), before, "holding the pointer cannot commit the {gesture} preview");
        assert_eq!(d.harness.state().session.project, project);
        assert_eq!(d.undo_count(), undo);
        pointer(&mut d, vec![button(start + offset, false)]);
        d.frames(3);
        let changed = d.param(&effect, "position", None);
        assert_ne!(changed, before, "a visible {gesture} release must commit its preview");
        assert_eq!(d.undo_count(), undo + 1, "the visible {gesture} release is one undo step");
        assert_ne!(render_graphic(&d, sample), before_image, "the visible {gesture} release affects rendered pixels");
        if gesture == "time" {
            assert_ne!(changed.keyframes[0].time, before.keyframes[0].time);
            assert_eq!(changed.keyframes[0].value, before.keyframes[0].value);
        } else {
            assert_eq!(changed.keyframes[0].time, before.keyframes[0].time);
            if gesture == "value" {
                assert!(changed.keyframes[0].value.as_vec2().unwrap().x > before.keyframes[0].value.as_vec2().unwrap().x);
                assert_eq!(changed.keyframes[0].value.as_vec2().unwrap().y, before.keyframes[0].value.as_vec2().unwrap().y);
            } else {
                assert_eq!(changed.keyframes[0].value, before.keyframes[0].value);
                assert!(changed.keyframes[0].out_influence > before.keyframes[0].out_influence);
            }
        }
        let after_image = render_graphic(&d, sample);
        let after_project = d.harness.state().session.project.clone();
        d.exec("edit.undo", json!({}));
        assert_eq!(d.param(&effect, "position", None), before);
        assert_eq!(d.harness.state().session.project, project);
        assert_eq!(render_graphic(&d, sample), before_image);
        d.exec("edit.redo", json!({}));
        assert_eq!(d.param(&effect, "position", None), changed);
        assert_eq!(d.harness.state().session.project, after_project);
        assert_eq!(render_graphic(&d, sample), after_image);
    }
}
