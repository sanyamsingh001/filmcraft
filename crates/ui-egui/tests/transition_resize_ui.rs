//! Transition resizing through the actual timeline gesture, with small generated media.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::{Generator, generators::GeneratorSource};
use filmcraft_project::{Label, SequenceSettings, TrackKind, Transition, TransitionAlign, TransitionId, find_effect};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    id: TransitionId,
}

impl Driver {
    fn new(rate: FrameRate, kind: TrackKind, one_sided: bool) -> Self {
        let mut s = Session::default();
        let mut p = (*s.project).clone();
        let generator = if kind == TrackKind::Video { Generator::ColorMatte { color: [0.2, 0.5, 0.8, 1.0] } } else { Generator::Tone { hz: 440.0, db: -12.0 } };
        let item = filmcraft_engine::demo::add_generator(
            &mut p,
            &s.media,
            GeneratorSource::new(generator, 32, 18, rate, rate.tick_of(100)),
            "Resize fixture",
            Label::Iris,
            None,
        );
        let seq = p.new_sequence("Transition resize", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 1, None);
        let mut a = p.make_track_item(item, kind, Tick::ZERO, TimeRange::new(Tick::ZERO, rate.tick_of(50)), rate).unwrap();
        let mut b = p.make_track_item(item, kind, rate.tick_of(50), TimeRange::new(Tick::ZERO, rate.tick_of(50)), rate).unwrap();
        a.duration = rate.tick_of(50);
        b.duration = rate.tick_of(100) - b.start;
        let id = TransitionId(p.alloc_id());
        let start = rate.tick_of(if one_sided { 50 } else { 44 });
        let end = rate.tick_of(if one_sided { 62 } else { 56 });
        let effect = find_effect(if kind == TrackKind::Video { "cross_dissolve" } else { "constant_power" }).unwrap().instance();
        let tr = Transition {
            id,
            effect,
            start,
            duration: end - start,
            from: (!one_sided).then_some(a.id),
            to: Some(b.id),
            align: TransitionAlign::CenterAtCut,
            reverse: false,
        };
        let track = &mut p.sequence_mut(seq).unwrap().tracks_mut(kind)[0];
        track.items = vec![a, b];
        track.transitions = vec![tr];
        s.project = Arc::new(p);
        s.state.active_sequence = Some(seq);
        s.state.open_sequences = vec![seq];
        s.state.snapping = false;
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { harness, tx, id };
        d.frames(4);
        d.ok("ui.set", json!({"timeline": {"pps": 120.0, "scroll": 0.0}}));
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

    fn rect(&mut self, id: &str) -> egui::Rect {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        let e = v.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("no element {id}: {v}"));
        let r = e["rect"].as_array().unwrap();
        egui::Rect::from_min_size(
            egui::pos2(r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32),
            egui::vec2(r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32),
        )
    }

    fn edge(&mut self, edge: &str) -> egui::Pos2 {
        self.rect(&format!("timeline.transition.{}.{}", self.id.0, edge)).center()
    }

    fn bounds(&self) -> (i64, i64) {
        let s = &self.harness.state().session;
        let tr = s.active_sequence().unwrap().all_tracks().flat_map(|t| &t.transitions).find(|t| t.id == self.id).unwrap();
        let r = s.sequence_rate();
        (r.frame_at(tr.start), r.frame_at(tr.end()))
    }

    fn event(&mut self, event: egui::Event) {
        self.harness.input_mut().events.push(event);
        self.frames(1);
    }

    fn press(&mut self, pos: egui::Pos2) {
        self.event(egui::Event::PointerMoved(pos));
        self.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    }

    fn release(&mut self, pos: egui::Pos2) {
        self.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        self.frames(2);
    }

    fn drag_clip_out(&mut self, clip: filmcraft_project::ClipId, frames: i64, steps: usize) {
        let rect = self.rect(&format!("timeline.clip.{}", clip.0));
        // The top label strip is clear of the transition stripe at the same cut.
        let from = egui::pos2(rect.right(), rect.top() + 4.0);
        let hit = self.ok("ui.timeline.hit", json!({"x": from.x, "y": from.y}));
        assert_eq!(hit["clip"], clip.0);
        assert_eq!(hit["edge"], "Out");
        let pps = self.harness.state().ui.timeline.pps;
        let fps = self.harness.state().session.sequence_rate().as_f64();
        let to = from + egui::vec2((frames as f64 * pps / fps) as f32, 0.0);
        self.ok("ui.drag", json!({"from": {"x": from.x, "y": from.y}, "to": {"x": to.x, "y": to.y}, "steps": steps}));
        self.frames(2);
    }

    fn drag_edge(&mut self, edge: &str, frames: i64) {
        let from = self.edge(edge);
        let pps = self.harness.state().ui.timeline.pps;
        let fps = self.harness.state().session.sequence_rate().as_f64();
        let to = from + egui::vec2((frames as f64 * pps / fps) as f32, 0.0);
        // Move outward by more than the handle width in the first frame. The press origin must
        // identify the handle even when the pointer has already left the transition stripe.
        self.ok("ui.drag", json!({"from": {"x": from.x, "y": from.y}, "to": {"x": to.x, "y": to.y}, "steps": 2}));
        self.frames(2);
    }
}

#[test]
fn transition_handles_stretch_compress_video_and_audio_with_one_undo_per_drag() {
    for kind in [TrackKind::Video, TrackKind::Audio] {
        for rate in [FrameRate::FPS_24, FrameRate::new(31, 1)] {
            let mut d = Driver::new(rate, kind, false);
            let before = d.harness.state().session.project.clone();
            let p = d.edge("out");
            let hit = d.ok("ui.timeline.hit", json!({"x": p.x, "y": p.y}));
            assert_eq!(hit["transition"], d.id.0);
            assert_eq!(hit["edge"], "Out");
            d.drag_edge("out", 4);
            assert_eq!(d.bounds(), (40, 60));
            assert_eq!(d.harness.state().session.history.undo.len(), 1);
            d.drag_edge("in", 7);
            assert_eq!(d.bounds(), (47, 53));
            assert_eq!(d.harness.state().session.history.undo.len(), 2);
            d.exec("edit.undo", json!({}));
            assert_eq!(d.bounds(), (40, 60));
            d.exec("edit.undo", json!({}));
            assert_eq!(d.harness.state().session.project, before);
        }
    }
}

#[test]
fn provisional_drag_is_visible_and_escape_or_focus_loss_cancels_without_an_edit() {
    for escape in [true, false] {
        let mut d = Driver::new(FrameRate::FPS_24, TrackKind::Video, false);
        let before = d.harness.state().session.project.clone();
        let body = format!("timeline.transition.{}", d.id.0);
        let width = d.rect(&body).width();
        let from = d.edge("out");
        let to = from + egui::vec2(20.0, 0.0);
        d.press(from);
        d.event(egui::Event::PointerMoved(to));
        d.frames(2);
        assert_eq!(d.bounds(), (44, 56), "preview leaves the project untouched");
        assert!(d.rect(&body).width() > width + 20.0, "the stripe shows the longer provisional range");
        assert!(d.harness.state().session.history.undo.is_empty());
        if escape {
            d.ok("ui.key", json!({"key": "Escape"}));
        } else {
            d.harness.input_mut().focused = false;
            d.frames(1);
            d.harness.input_mut().focused = true;
        }
        d.release(to);
        assert_eq!(d.harness.state().session.project, before);
        assert!(d.harness.state().session.history.undo.is_empty());
        assert!((d.rect(&body).width() - width).abs() < 0.01, "cancellation restores the original stripe");
    }
}

#[test]
fn resize_snaps_to_a_marker_and_stops_at_one_frame() {
    let mut d = Driver::new(FrameRate::FPS_24, TrackKind::Video, false);
    let r = d.harness.state().session.sequence_rate();
    d.exec("markers.add", json!({"time": r.tick_of(60).0, "name": "Resize target"}));
    d.exec("sequence.snap", json!({"on": true}));
    d.drag_edge("out", 3); // Frame 59 is five pixels from the marker at frame 60.
    assert_eq!(d.bounds(), (40, 60));
    d.exec("sequence.snap", json!({"on": false}));
    d.drag_edge("out", -10);
    assert_eq!(d.bounds(), (49, 50));
}

#[test]
fn one_sided_audio_fade_has_only_its_outer_handle_and_locked_track_has_neither() {
    let mut d = Driver::new(FrameRate::FPS_24, TrackKind::Audio, true);
    let prefix = format!("timeline.transition.{}.", d.id.0);
    let handles = d.ok("ui.elements", json!({"prefix": prefix}));
    let ids: Vec<_> = handles.as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids, vec![format!("timeline.transition.{}.out", d.id.0)]);
    d.drag_edge("out", 8);
    assert_eq!(d.bounds(), (50, 70));
    d.exec("timeline.setTrack", json!({"track": "A1", "locked": true}));
    let handles = d.ok("ui.elements", json!({"prefix": prefix}));
    assert!(handles.as_array().unwrap().is_empty());
    let rect = d.rect(&format!("timeline.transition.{}", d.id.0));
    let p = egui::pos2(rect.right() - 1.0, rect.center().y);
    let hit = d.ok("ui.timeline.hit", json!({"x": p.x, "y": p.y}));
    assert!(hit["edge"].is_null());
    let undo = d.harness.state().session.history.undo.len();
    d.ok("ui.drag", json!({"from": {"x": p.x, "y": p.y}, "to": {"x": p.x + 30.0, "y": p.y}, "steps": 2}));
    assert_eq!(d.bounds(), (50, 70));
    assert_eq!(d.harness.state().session.history.undo.len(), undo);
}

#[test]
fn selection_edge_after_razor_shortens_and_restores_footage_with_large_or_gradual_moves() {
    let mut results = Vec::new();
    for steps in [2, 16] {
        let mut d = Driver::new(FrameRate::FPS_24, TrackKind::Video, false);
        d.harness.state_mut().session.prefs.trim.selection_tool_roll_ripple = false;
        let r = d.harness.state().session.sequence_rate();
        let first = d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0].id;
        d.exec("timeline.razor", json!({"clip": first.0, "time": r.tick_of(30).0}));
        let before = d.harness.state().session.project.clone();
        let undo = d.harness.state().session.history.undo.len();
        d.drag_clip_out(first, -10, steps);
        let it = d.harness.state().session.active_sequence().unwrap().find_item(first).unwrap().1;
        assert_eq!((r.frame_at(it.start), r.frame_at(it.end()), r.frame_at(it.source_in)), (0, 20, 0), "the edge trims instead of moving the clip");
        d.drag_clip_out(first, 6, steps);
        let it = d.harness.state().session.active_sequence().unwrap().find_item(first).unwrap().1;
        assert_eq!((r.frame_at(it.start), r.frame_at(it.end()), r.frame_at(it.source_out())), (0, 26, 26), "extend restores hidden source footage");
        results.push(d.harness.state().session.active_sequence().unwrap().video_tracks[0].items.clone());
        assert_eq!(d.harness.state().session.history.undo.len(), undo + 2);
        d.exec("edit.undo", json!({}));
        d.exec("edit.undo", json!({}));
        assert_eq!(d.harness.state().session.project, before, "both gestures undo without removing the razor cut");
    }
    assert_eq!(results[0], results[1], "drag event spacing cannot change edge intent");
}

#[test]
fn n_key_roll_moves_the_cut_for_large_and_gradual_drags_with_one_undo() {
    let mut results = Vec::new();
    for steps in [2, 16] {
        let mut d = Driver::new(FrameRate::FPS_24, TrackKind::Video, false);
        let before = d.harness.state().session.project.clone();
        let r = d.harness.state().session.sequence_rate();
        let first = d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0].id;
        d.ok("ui.key", json!({"key": "N"}));
        assert_eq!(d.harness.state().ui.tool, filmcraft_ui_egui::state::Tool::Rolling);
        d.drag_clip_out(first, 10, steps);
        let items = &d.harness.state().session.active_sequence().unwrap().video_tracks[0].items;
        assert_eq!((r.frame_at(items[0].start), r.frame_at(items[0].end())), (0, 60));
        assert_eq!(
            (r.frame_at(items[1].start), r.frame_at(items[1].end()), r.frame_at(items[1].source_in)),
            (60, 100, 10),
            "the neighbor loses its first ten source frames while the total span stays fixed"
        );
        results.push(items.clone());
        assert_eq!(d.harness.state().session.history.undo.len(), 1);
        assert_eq!(d.harness.state().session.history.undo[0].0, "Rolling Edit");
        d.exec("edit.undo", json!({}));
        assert_eq!(d.harness.state().session.project, before);
    }
    assert_eq!(results[0], results[1], "the N tool uses the pressed edge even after a large pointer jump");
}
