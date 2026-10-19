//! Point graph edits through the real UI: each coordinate keeps its counterpart and the shared
//! temporal easing, every gesture is undoable, and scalar/mask graph controls keep their ids.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::{ClipId, Param, ParamValue};
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
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
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
        let v = self.ok("ui.elements", json!({"prefix": id}));
        let e = v.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("no element {id}: {v}"));
        let r = e["rect"].as_array().unwrap();
        std::array::from_fn(|i| r[i].as_f64().unwrap())
    }

    fn open(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }

    fn drag(&mut self, id: &str, dx: f64, dy: f64) {
        let r = self.rect(id);
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
