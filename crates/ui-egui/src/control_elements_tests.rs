//! Element queries use real rendered panels: a hidden Timeline cannot settle its zoom.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::{ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings};
use filmcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

use super::ControlRequest;
use crate::FilmcraftApp;
use crate::dock::PanelKind;

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    clip: u64,
}

impl Driver {
    fn new() -> Self {
        let rate = FrameRate::FPS_24;
        let source = GeneratorSource::new(Generator::BlackVideo, 32, 18, rate, Tick::from_seconds_f64(2.0));
        let mut project = Project::new("Control element queries");
        let item = project.add_item(
            "Tiny clip",
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
        let seq = project.new_sequence("Tiny sequence", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 1, None);
        let mut session = Session::default();
        session.project = Arc::new(project);
        session.state.active_sequence = Some(seq);
        session.state.open_sequences = vec![seq];
        session.execute("timeline.place", json!({"item": item.0, "track": "V1", "time": 0, "insert": false})).unwrap();
        let clip = session.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        session.execute("timeline.select", json!({"clips": [clip]})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut driver = Self { harness, tx, clip };
        driver.frames(4);
        driver.ok("ui.set", json!({"workspace": "Effects", "focused": "Effect Controls"}));
        driver.frames(4);
        driver
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn start(&mut self, method: &str, params: Value) -> Receiver<Value> {
        let (req, reply) = ControlRequest::new(method, params);
        self.tx.send(req).unwrap();
        reply
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = self.start(method, params.clone());
        for _ in 0..240 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], true, "{method} {params}: {value}");
                return value["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn menu(&mut self, id: &str) {
        self.ok("ui.menu.invoke", json!({"id": id}));
    }

    fn settle_timeline(&mut self) {
        for _ in 0..180 {
            let app = self.harness.state();
            if !app.ui.timeline.animating() && app.timeline_still >= 2 {
                return;
            }
            self.frames(1);
        }
        panic!("the visible tiny Timeline did not settle");
    }

    fn current_clip_rect(&self) -> [f32; 4] {
        let id = format!("timeline.clip.{}", self.clip);
        self.harness.state().auto.elements.iter().find(|element| element.id == id).unwrap().rect
    }
}

#[test]
fn maximized_effect_controls_queries_succeed_with_unsettled_hidden_timeline() {
    let mut driver = Driver::new();
    driver.menu("window.maximizeFrame");
    driver.frames(4);
    driver.menu("view.zoomOut");
    driver.frames(4);
    let app = driver.harness.state();
    assert_eq!(app.ui.keys.maximized, Some(PanelKind::EffectControls));
    assert!(app.ui.timeline.animating());
    assert_eq!(app.timeline_still, 0);
    assert!(app.auto.elements.iter().any(|element| element.id == "panel.EffectControls"));
    assert!(!app.auto.elements.iter().any(|element| element.id == "panel.Timeline"));

    for prefix in ["effectControls.lane", "source.", "", "timeline."] {
        let reply = driver.start("ui.elements", json!({"prefix": prefix}));
        driver.frames(1);
        let result = reply.try_recv().unwrap_or_else(|error| panic!("hidden Timeline blocked prefix {prefix:?}: {error}"));
        assert_eq!(result["ok"], true, "{prefix}: {result}");
        let elements = result["result"].as_array().unwrap();
        if prefix == "effectControls.lane" {
            assert!(elements.iter().any(|element| element["id"] == "effectControls.lane"));
        } else if prefix == "timeline." {
            assert!(elements.is_empty(), "the hidden Timeline has no rendered controls");
        }
        let app = driver.harness.state();
        assert!(app.ui.timeline.animating(), "querying must not change the hidden view");
        assert_eq!(app.timeline_still, 0);
    }
}

#[test]
fn visible_moving_timeline_queries_wait_but_unrelated_prefixes_do_not() {
    let mut driver = Driver::new();
    driver.settle_timeline();
    driver.menu("view.zoomOut");
    assert!(driver.harness.state().ui.timeline.animating());
    assert!(driver.harness.state().auto.elements.iter().any(|element| element.id == "panel.Timeline"));
    let before = driver.current_clip_rect();
    let timeline = driver.start("ui.elements", json!({"prefix": "timeline.clip."}));
    let partial = driver.start("ui.elements", json!({"prefix": "t"}));
    let full = driver.start("ui.elements", json!({}));
    let unrelated = driver.start("ui.elements", json!({"prefix": "effectControls.lane"}));
    driver.frames(1);
    assert!(matches!(timeline.try_recv(), Err(TryRecvError::Empty)));
    assert!(matches!(partial.try_recv(), Err(TryRecvError::Empty)));
    assert!(matches!(full.try_recv(), Err(TryRecvError::Empty)));
    let unrelated = unrelated.try_recv().unwrap();
    assert_eq!(unrelated["ok"], true);
    assert!(unrelated["result"].as_array().unwrap().iter().any(|element| element["id"] == "effectControls.lane"));
    let moving = driver.current_clip_rect();
    assert!((before[2] - moving[2]).abs() > 0.1, "real clip geometry moves during zoom: {before:?} -> {moving:?}");

    let mut results = [None, None, None];
    let replies = [timeline, partial, full];
    for _ in 0..180 {
        driver.frames(1);
        for (result, reply) in results.iter_mut().zip(&replies) {
            if result.is_none() {
                *result = reply.try_recv().ok();
            }
        }
        if results.iter().all(Option::is_some) {
            break;
        }
    }
    assert!(!driver.harness.state().ui.timeline.animating());
    assert!(driver.harness.state().timeline_still >= 2);
    let expected = driver.current_clip_rect();
    let id = format!("timeline.clip.{}", driver.clip);
    for result in results {
        let result = result.expect("visible Timeline must answer once its animation settles");
        assert_eq!(result["ok"], true, "{result}");
        let clip = result["result"].as_array().unwrap().iter().find(|element| element["id"] == id).unwrap();
        assert_eq!(clip["rect"], json!(expected), "returned clip coordinates agree with the settled UI");
    }
}

#[test]
fn empty_timeline_placeholder_queries_do_not_wait_for_an_unrendered_zoom() {
    let mut driver = Driver::new();
    driver.ok("engine.execute", json!({"command": "file.newProject", "params": {"name": "Empty project"}}));
    driver.frames(4);
    driver.ok("ui.set", json!({"timeline": {"pps": 80.0}}));
    driver.frames(4);
    let app = driver.harness.state();
    assert!(app.session.active_sequence().is_none());
    assert!(app.ui.timeline.animating());
    assert_eq!(app.timeline_still, 0);
    assert!(app.auto.elements.iter().any(|element| element.id == "panel.Timeline"));
    let placeholder = app.auto.elements.iter().find(|element| element.id == "timeline.openDemo").unwrap().rect;
    for prefix in ["timeline.openDemo", ""] {
        let reply = driver.start("ui.elements", json!({"prefix": prefix}));
        driver.frames(1);
        let result = reply.try_recv().unwrap_or_else(|error| panic!("empty Timeline blocked prefix {prefix:?}: {error}"));
        assert_eq!(result["ok"], true, "{result}");
        let button = result["result"].as_array().unwrap().iter().find(|element| element["id"] == "timeline.openDemo").unwrap();
        assert_eq!(button["rect"], json!(placeholder));
        assert!(driver.harness.state().ui.timeline.animating());
    }
}
