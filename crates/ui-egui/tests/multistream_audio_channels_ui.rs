//! Actual Audio Channels controls, captured targets and per-stream local channels.
use std::sync::{
    Arc,
    mpsc::{Sender, channel},
};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::{MediaKind, MediaSource};
use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use filmcraft_ui_egui::{FilmcraftApp, control::ControlRequest, dock::PanelKind};
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    item: ItemId,
}
impl Driver {
    fn new(size: [f32; 2], nested: bool) -> Self {
        let mut session = Session::default();
        let source =
            filmcraft_media::generators::GeneratorSource::new(filmcraft_media::Generator::BarsAndTone, 32, 18, FrameRate::FPS_24, Tick::from_seconds_f64(2.0));
        let mut info = source.info().clone();
        info.video = None;
        info.kind = MediaKind::AudioOnly;
        info.audio_streams = vec![info.audio_streams[0].clone(); 3];
        let mut project = (*session.project).clone();
        let item = project.add_item(
            "Three streams",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(filmcraft_media::Generator::BarsAndTone),
                info,
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
        let sequence =
            project.new_sequence("Outer", SequenceSettings { width: 32, height: 18, frame_rate: FrameRate::FPS_24, ..Default::default() }, 0, 1, None);
        if nested {
            let inner =
                project.new_sequence("Nested", SequenceSettings { width: 32, height: 18, frame_rate: FrameRate::FPS_24, ..Default::default() }, 0, 1, None);
            let range = TimeRange::new(Tick::ZERO, Tick::from_seconds_f64(2.0));
            let audio = project.make_track_item(item, TrackKind::Audio, Tick::ZERO, range, FrameRate::FPS_24).unwrap();
            project.sequence_mut(inner).unwrap().audio_tracks[0].items.push(audio);
            let nest = project.make_track_item(inner, TrackKind::Audio, Tick::ZERO, range, FrameRate::FPS_24).unwrap();
            let selected = nest.id;
            project.sequence_mut(sequence).unwrap().audio_tracks[0].items.push(nest);
            session.state.selection = vec![selected];
        } else {
            session.state.project_selection = vec![item];
        }
        session.project = Arc::new(project);
        session.state.active_sequence = Some(sequence);
        session.state.open_sequences = vec![sequence];
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(session).with_control(rx);
        app.ui.focused = if nested { PanelKind::Timeline } else { PanelKind::Project };
        let harness = Harness::builder().with_size(egui::vec2(size[0], size[1])).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut driver = Self { harness, tx, item };
        driver.frames(5);
        driver
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
    fn call(&mut self, method: &str, params: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], true, "{method} {params}: {value}");
                return value["result"].clone();
            }
        }
        panic!("No reply: {method} {params}");
    }
    fn open(&mut self) {
        self.call("ui.menu.invoke", json!({"id": "clip.audioChannels"}));
        self.frames(3);
    }
    fn click(&mut self, id: &str) {
        self.call("ui.click", json!({"id": id}));
        self.frames(3);
    }
    fn execute(&mut self, id: &str, params: Value) -> Value {
        self.call("engine.execute", json!({"command": id, "params": params}))
    }
    fn element(&mut self, id: &str) -> Value {
        let elements = self.call("ui.elements", json!({"prefix": id}));
        elements.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("missing {id}: {elements}")).clone()
    }
    fn params(&mut self) -> Value {
        self.harness.state_mut().ui.clip_dialog.as_ref().unwrap().params.clone()
    }
}

#[test]
fn stream_combo_format_and_count_keep_parallel_selected_rows() {
    let mut d = Driver::new([1000.0, 700.0], false);
    d.open();
    assert_eq!(d.params()["streams"], json!([0, 1, 2]));
    d.click("audioChannels.clip.0.stream");
    d.click("audioChannels.clip.0.stream.2");
    assert_eq!(d.params()["streams"], json!([2, 1, 2]));
    d.click("audioChannels.format.mono");
    assert_eq!(d.params()["streams"], json!([2, 2, 1, 1]));
    assert_eq!(d.params()["clips"], json!([[0], [1], [0], [1]]));
    d.call("ui.click", json!({"id": "audioChannels.count", "count": 2}));
    d.call("ui.key", json!({"key": "Cmd+A"}));
    d.call("ui.type", json!({"text": "3"}));
    d.call("ui.key", json!({"key": "Enter"}));
    d.frames(3);
    assert_eq!(d.params()["streams"], json!([2, 2, 1]));
    assert_eq!(d.params()["clips"], json!([[0], [1], [0]]));
    let before = d.harness.state_mut().session.project.clone();
    let item = d.item;
    d.click("audioChannels.ok");
    let app = d.harness.state_mut();
    assert!(app.ui.clip_dialog.is_none());
    let map = app.session.project.item(item).unwrap().as_media().unwrap().interpret.audio_channels.as_ref().unwrap();
    assert_eq!(map.streams, [2, 2, 1]);
    app.session.undo().unwrap();
    assert_eq!(*app.session.project, *before);
}

#[test]
fn modal_selection_is_captured_and_project_replacement_is_rejected() {
    let mut d = Driver::new([1000.0, 700.0], false);
    d.open();
    let captured = d.params()["items"].clone();
    d.harness.state_mut().session.state.project_selection.clear();
    d.click("audioChannels.clip.0.stream");
    d.click("audioChannels.clip.0.stream.2");
    assert_eq!(d.params()["items"], captured);
    d.click("audioChannels.ok");
    assert!(d.harness.state_mut().ui.clip_dialog.is_none());
    let item = d.item;
    assert_eq!(d.harness.state_mut().session.project.item(item).unwrap().as_media().unwrap().interpret.audio_channels.as_ref().unwrap().streams[0], 2);
    d.harness.state_mut().session.state.project_selection = vec![item];
    d.open();
    let old_dialog = d.harness.state_mut().ui.clip_dialog.clone();
    d.execute("file.newProject", json!({"name": "Replacement"}));
    // Lifecycle-aware hosts dismiss old project dialogs. A restored serialized draft
    // must still reject Apply without recapturing the replacement project.
    if d.harness.state_mut().ui.clip_dialog.is_none() {
        d.harness.state_mut().ui.clip_dialog = old_dialog;
        d.frames(3);
    }
    let before = serde_json::to_value(&*d.harness.state_mut().session.project).unwrap();
    let history = d.harness.state_mut().session.history.undo.len();
    d.click("audioChannels.ok");
    assert_eq!(serde_json::to_value(&*d.harness.state_mut().session.project).unwrap(), before);
    assert_eq!(d.harness.state_mut().session.history.undo.len(), history);
    assert!(d.harness.state_mut().ui.clip_dialog.as_ref().unwrap().error.contains("project or source changed"));
    d.click("audioChannels.cancel");
}

#[test]
fn nested_timeline_audio_retains_local_channel_controls() {
    let mut d = Driver::new([1000.0, 700.0], true);
    d.open();
    d.element("audioChannels.clip.0.ch.1");
    let id = d.harness.state_mut().session.state.selection[0];
    d.harness.state_mut().session.state.selection.clear();
    d.click("audioChannels.clip.0.ch.1");
    assert_eq!(d.params()["channels"], json!([0]));
    assert!(d.params().get("stream").is_none());
    d.click("audioChannels.ok");
    let app = d.harness.state_mut();
    assert!(app.ui.clip_dialog.is_none(), "Audio Channels Apply failed: {:?}", app.ui.clip_dialog.as_ref().map(|draft| (&draft.params, &draft.error)));
    let it = app.session.active_sequence().unwrap().find_item(id).unwrap().1;
    assert_eq!(it.audio_stream, 0);
    assert_eq!(it.source_channels, [0]);
}

#[test]
fn small_window_keeps_multirow_footer_visible_and_clickable() {
    for size in [[360.0, 300.0], [420.0, 300.0]] {
        let mut d = Driver::new(size, false);
        d.open();
        d.click("audioChannels.format.mono");
        assert_eq!(d.params()["clips"].as_array().unwrap().len(), 6);
        for id in ["audioChannels.cancel", "audioChannels.ok"] {
            let e = d.element(id);
            let r = e["rect"].as_array().unwrap();
            let x = r[0].as_f64().unwrap() as f32;
            let y = r[1].as_f64().unwrap() as f32;
            let w = r[2].as_f64().unwrap() as f32;
            let h = r[3].as_f64().unwrap() as f32;
            assert!(x >= 0.0 && y >= 0.0 && x + w <= size[0] && y + h <= size[1], "{size:?}: {id}: {e}");
        }
        d.click("audioChannels.cancel");
        assert!(d.harness.state_mut().ui.clip_dialog.is_none());
        let before = d.harness.state_mut().session.project.clone();
        d.open();
        d.click("audioChannels.format.mono");
        d.click("audioChannels.ok");
        let item = d.item;
        let app = d.harness.state_mut();
        assert!(app.ui.clip_dialog.is_none(), "{size:?}: the visible OK button applies the map");
        let map = app.session.project.item(item).unwrap().as_media().unwrap().interpret.audio_channels.as_ref().unwrap();
        assert_eq!(map.streams, [0, 0, 1, 1, 2, 2]);
        assert_eq!(map.clips, [vec![0], vec![1], vec![0], vec![1], vec![0], vec![1]]);
        app.session.undo().unwrap();
        assert_eq!(*app.session.project, *before);
        d.open();
        d.click("audioChannels.format.mono");
        let old_dialog = d.harness.state_mut().ui.clip_dialog.clone();
        d.execute("file.newProject", json!({"name": "Replacement"}));
        if d.harness.state_mut().ui.clip_dialog.is_none() {
            d.harness.state_mut().ui.clip_dialog = old_dialog;
            d.frames(3);
        }
        let replacement = serde_json::to_value(&*d.harness.state_mut().session.project).unwrap();
        let history = d.harness.state_mut().session.history.undo.len();
        d.click("audioChannels.ok");
        assert!(d.harness.state_mut().ui.clip_dialog.as_ref().unwrap().error.contains("project or source changed"));
        assert_eq!(serde_json::to_value(&*d.harness.state_mut().session.project).unwrap(), replacement);
        assert_eq!(d.harness.state_mut().session.history.undo.len(), history);
        for id in ["audioChannels.cancel", "audioChannels.ok"] {
            let e = d.element(id);
            let r = e["rect"].as_array().unwrap();
            let x = r[0].as_f64().unwrap() as f32;
            let y = r[1].as_f64().unwrap() as f32;
            let w = r[2].as_f64().unwrap() as f32;
            let h = r[3].as_f64().unwrap() as f32;
            assert!(x >= 0.0 && y >= 0.0 && x + w <= size[0] && y + h <= size[1], "{size:?}: error footer {id}: {e}");
        }
        d.click("audioChannels.cancel");
        assert!(d.harness.state_mut().ui.clip_dialog.is_none());
    }
}
