//! #302: Program picture fullscreen, with playback and a reversible editor/window transition.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::dock::{DockNode, PanelKind, SplitSize};
use filmcraft_ui_egui::{FilmcraftApp, PlaybackTarget};
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new(demo: bool) -> Self {
        let mut session = Session::default();
        if demo {
            session.execute("file.openDemoProject", json!({})).unwrap();
        }
        Self::with_session(session)
    }

    fn with_session(session: Session) -> Self {
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { harness, tx };
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

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params.clone());
        assert_eq!(r["ok"], true, "{method} {params}: {r}");
        self.frames(2);
        r["result"].clone()
    }

    fn key(&mut self, key: &str) {
        self.ok("ui.key", json!({"key": key}));
    }

    fn elements(&mut self) -> Vec<Value> {
        self.ok("ui.elements", json!({})).as_array().unwrap().clone()
    }

    fn editor_state(&mut self) -> Value {
        let ui = &self.harness.state_mut().ui;
        json!({"dock": ui.dock, "focused": ui.focused, "mode": ui.mode, "program": ui.program, "keys": ui.keys})
    }
}

#[test]
fn program_picture_fills_the_window_and_restores_editor_view() {
    let mut d = Driver::new(true);
    d.ok(
        "ui.set",
        json!({"focused": "Program", "program": {
            "zoom": 2.0, "pan": [160.0, 50.0], "display": "Comparison", "multicam": true,
            "show_rulers": true, "show_guides": true, "safe_margins": true,
            "guides": [{"vertical": true, "position": 480.0}]
        }}),
    );
    let before = d.editor_state();
    assert!(d.elements().iter().any(|e| e["id"] == "panel.Timeline"), "the normal editor timeline is visible before entering preview");
    d.ok("ui.click", json!({"id": "program.fullscreen"}));
    assert!(d.harness.state_mut().ui.program_fullscreen.is_some());
    assert_eq!(d.editor_state(), before, "the preview must not overwrite editor view settings");
    let elements = d.elements();
    let picture = elements.iter().find(|e| e["id"] == "program.fullscreen.picture").unwrap();
    let r = picture["rect"].as_array().unwrap();
    assert!(r[2].as_f64().unwrap() > 1500.0, "the complete 16:9 picture fits the window: {picture}");
    assert!((r[2].as_f64().unwrap() / r[3].as_f64().unwrap() - 16.0 / 9.0).abs() < 0.01);
    assert!(elements.iter().all(|e| !e["id"].as_str().unwrap().starts_with("header.")));
    for hidden in ["panel.Timeline", "program.zoom", "program.ruler.top", "program.guide.0", "program.compare.reference", "trimMonitor.outgoing"] {
        assert!(elements.iter().all(|e| e["id"] != hidden), "editor element is hidden: {hidden}");
    }
    d.key("Escape");
    assert!(d.harness.state_mut().ui.program_fullscreen.is_none());
    assert_eq!(d.editor_state(), before);
    assert!(d.elements().iter().any(|e| e["id"] == "header.mode.edit"));

    // The registered shortcut enters and toggles out, including while another panel is maximized.
    d.harness.state_mut().ui.focused = PanelKind::Source;
    d.harness.state_mut().ui.keys.maximized = Some(PanelKind::Timeline);
    let before = d.editor_state();
    d.key("F11");
    assert!(d.harness.state_mut().ui.program_fullscreen.is_some());
    d.key("F11");
    assert!(d.harness.state_mut().ui.program_fullscreen.is_none());
    assert_eq!(d.editor_state(), before);
    d.key("F11");
    d.ok("ui.click", json!({"id": "program.fullscreen.picture", "count": 2}));
    assert!(d.harness.state_mut().ui.program_fullscreen.is_none(), "double-click exits the preview");
    assert_eq!(d.editor_state(), before);
}

#[test]
fn fullscreen_button_remains_clickable_in_a_narrow_program_pane() {
    let mut d = Driver::new(true);
    d.harness.state_mut().ui.dock = DockNode::Split {
        vertical: false,
        size: SplitSize::FixedB(400.0),
        a: Box::new(DockNode::Tabs { panels: vec![PanelKind::Source], active: 0 }),
        b: Box::new(DockNode::Tabs { panels: vec![PanelKind::Program], active: 0 }),
    };
    d.frames(4);
    let elements = d.elements();
    let pane = elements.iter().find(|e| e["id"] == "panel.Program").unwrap();
    assert!(pane["rect"][2].as_f64().unwrap() <= 400.0);
    d.ok("ui.click", json!({"id": "program.fullscreen"}));
    assert!(d.harness.state_mut().ui.program_fullscreen.is_some(), "the + button must not capture the fullscreen click");
    d.key("Escape");
}

#[test]
fn entering_fullscreen_finishes_dynamic_trim_and_keeps_preview_transport_separate() {
    let mut d = Driver::new(true);
    let clip = d.harness.state_mut().session.active_sequence().unwrap().video_tracks[0].items[0].id;
    for track in ["V2", "V3", "A2", "A3"] {
        d.ok("engine.execute", json!({"command": "timeline.setTrack", "params": {"track": track, "syncLock": false}}));
    }
    d.ok("engine.execute", json!({"command": "trim.selectEditPoint", "params": {"clip": clip.0, "edge": "out", "kind": "roll"}}));
    d.harness.state_mut().ui.focused = PanelKind::Timeline;
    let selected = d.harness.state_mut().session.state.edit_points.clone();
    d.key("L");
    d.frames(10);
    assert!(d.harness.state_mut().session.trim_play.dynamic.is_some());
    d.ok("ui.menu.invoke", json!({"id": "view.programFullScreen"}));
    assert!(!d.harness.state_mut().session.trim_play.active());
    let committed = serde_json::to_value(&d.harness.state_mut().session.project).unwrap();
    d.frames(10);
    d.key("L");
    d.frames(60);
    d.key("K");
    assert!(!d.harness.state_mut().session.trim_play.active());
    assert_eq!(serde_json::to_value(&d.harness.state_mut().session.project).unwrap(), committed, "preview playback must not change the hidden edit");
    d.key("Escape");
    assert_eq!(d.harness.state_mut().session.state.edit_points, selected);
    assert!(d.elements().iter().any(|e| e["id"] == "trimMonitor.outgoing"));
}

#[test]
fn fullscreen_stops_source_and_transport_plays_program_with_preserved_source_focus() {
    let mut d = Driver::new(true);
    let source = d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0].item;
    d.ok("engine.execute", json!({"command": "source.open", "params": {"item": source.0}}));
    d.ok("engine.execute", json!({"command": "source.setPlayhead", "params": {"frame": 12}}));
    d.harness.state_mut().ui.focused = PanelKind::Source;
    let before = d.harness.state().session.playhead();
    d.ok("ui.playback", json!({"target": "source", "action": "play"}));
    assert!(d.harness.state().source_is_playing());
    assert_eq!(d.harness.state().playback.target, Some(PlaybackTarget::Source(source)));
    d.frames(30);
    d.ok("ui.menu.invoke", json!({"id": "view.programFullScreen"}));
    assert!(!d.harness.state().source_is_playing(), "hidden Source playback stops on entry");
    assert_eq!(d.harness.state().session.playhead(), before, "entering preview does not move Program");
    assert_eq!(d.harness.state().ui.focused, PanelKind::Source, "the editor focus is retained for exit");
    let source_time = d.harness.state().session.state.source_playhead;
    d.key("Space");
    d.frames(60);
    assert!(d.harness.state_mut().playback.playing);
    assert!(d.harness.state().program_is_playing());
    assert_eq!(d.harness.state().playback.target, d.harness.state().session.state.active_sequence.map(PlaybackTarget::Program));
    assert!(d.harness.state_mut().session.playhead() > before, "the Program playhead advances during preview");
    assert_eq!(d.harness.state().session.state.source_playhead, source_time);
    d.key("K");
    assert!(!d.harness.state_mut().playback.playing);
    d.key("L");
    assert!(d.harness.state_mut().playback.playing);
    d.key("K");
    d.key("J");
    assert!(d.harness.state_mut().playback.speed < 0.0);
    d.key("K");
    let before_step = d.harness.state().session.playhead();
    d.key("Right");
    assert!(d.harness.state().session.playhead() > before_step, "preview frame navigation follows Program");
    assert_eq!(d.harness.state().session.state.source_playhead, source_time);
    d.ok("ui.click", json!({"id": "program.fullscreen.exit"}));
    assert!(d.harness.state_mut().ui.program_fullscreen.is_none());
    assert_eq!(d.harness.state_mut().ui.focused, PanelKind::Source);
    assert_eq!(d.harness.state().session.state.source_playhead, source_time);
}

#[test]
fn fullscreen_without_sequence_reports_error_and_closed_sequence_returns() {
    let mut d = Driver::new(false);
    let menu = d.ok("ui.menu.list", json!({}));
    let entry = menu.as_array().unwrap().iter().find(|e| e["id"] == "view.programFullScreen").unwrap();
    assert_eq!(entry["enabled"], false);
    let r = d.call("ui.menu.invoke", json!({"id": "view.programFullScreen"}));
    assert_eq!(r["ok"], false);
    assert!(d.harness.state_mut().ui.program_fullscreen.is_none());
    d.ok("engine.execute", json!({"command": "file.openDemoProject", "params": {}}));
    d.key("F11");
    assert!(d.harness.state_mut().ui.program_fullscreen.is_some());
    let seq = d.harness.state_mut().session.state.active_sequence.unwrap();
    d.ok("engine.execute", json!({"command": "sequence.close", "params": {"item": seq.0}}));
    assert!(d.harness.state_mut().ui.program_fullscreen.is_none());
    assert!(d.elements().iter().any(|e| e["id"] == "header.mode.edit"));
}

#[test]
fn fullscreen_restores_the_previous_native_window_state() {
    for previous_fullscreen in [false, true] {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        let mut app = FilmcraftApp::new(session);
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().fullscreen = Some(previous_fullscreen);
        let mut enter = ctx.run_ui(raw, |ui| {
            filmcraft_ui_egui::menus::invoke(&mut app, ui.ctx(), "view.programFullScreen", json!({})).unwrap();
        });
        assert!(enter.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::Fullscreen(true))));
        let mut exit = ctx.run_ui(egui::RawInput::default(), |ui| {
            filmcraft_ui_egui::menus::invoke(&mut app, ui.ctx(), "view.programFullScreen", json!({})).unwrap();
        });
        assert!(
            exit.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::Fullscreen(v) if *v == previous_fullscreen))
        );
        // These control-only frames are intentionally not submitted to a renderer.
        enter.textures_delta.clear();
        exit.textures_delta.clear();
        assert!(app.ui.program_fullscreen.is_none());
    }
}

#[test]
fn entering_and_leaving_preview_preserves_existing_program_playback() {
    let mut d = Driver::new(true);
    let source = d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0].item;
    d.ok("engine.execute", json!({"command": "source.open", "params": {"item": source.0}}));
    d.harness.state_mut().ui.focused = PanelKind::Source;
    d.ok("ui.playback", json!({"target": "program", "action": "play"}));
    assert!(d.harness.state().program_is_playing());
    d.ok("ui.menu.invoke", json!({"id": "view.programFullScreen"}));
    assert!(d.harness.state().program_is_playing());
    let before = d.harness.state().session.playhead();
    d.frames(60);
    assert!(d.harness.state().session.playhead() > before);
    d.key("Escape");
    assert!(d.harness.state().program_is_playing());
    assert_eq!(d.harness.state().ui.focused, PanelKind::Source);
    d.ok("ui.playback", json!({"action": "stop"}));
}

#[test]
fn preview_play_around_and_view_commands_follow_program_with_source_focus_preserved() {
    let mut d = Driver::new(true);
    let source = d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0].item;
    d.ok("engine.execute", json!({"command": "source.open", "params": {"item": source.0}}));
    d.ok("engine.execute", json!({"command": "source.setPlayhead", "params": {"frame": 12}}));
    d.harness.state_mut().ui.focused = PanelKind::Source;
    d.harness.state_mut().session.prefs.playback.preroll_seconds = 0.5;
    d.harness.state_mut().session.prefs.playback.postroll_seconds = 0.5;
    let source_time = d.harness.state().session.state.source_playhead;
    let source_view = serde_json::to_value(&d.harness.state().ui.source).unwrap();
    let project = serde_json::to_value(&d.harness.state().session.project).unwrap();
    d.key("F11");
    d.ok("ui.menu.invoke", json!({"id": "view.playbackRes.quarter"}));
    assert_eq!(d.harness.state().ui.program.res, filmcraft_ui_egui::state::PlaybackRes::Quarter);
    assert_eq!(serde_json::to_value(&d.harness.state().ui.source).unwrap(), source_view);
    d.key("Shift+K");
    assert!(d.harness.state().program_is_playing());
    assert!(d.harness.state().playback.stop_at.is_some());
    assert!(!d.harness.state().session.trim_play.active());
    d.frames(360);
    assert!(!d.harness.state().playback.playing, "play around stops after its postroll");
    assert_eq!(d.harness.state().session.state.source_playhead, source_time);
    assert_eq!(serde_json::to_value(&d.harness.state().session.project).unwrap(), project, "preview play around must not change hidden edits");
    // Explicit requests retain their monitor target even during preview.
    d.ok("ui.menu.invoke", json!({"id": "view.playbackRes.eighth", "params": {"monitor": "source"}}));
    assert_eq!(d.harness.state().ui.source.res, filmcraft_ui_egui::state::PlaybackRes::Eighth);
    d.key("Escape");
    assert_eq!(d.harness.state().ui.focused, PanelKind::Source);
}

#[test]
fn fullscreen_cancels_a_timeline_transition_drag_without_editing_or_leaving_clicks_blocked() {
    use filmcraft_media::{Generator, generators::GeneratorSource};
    use filmcraft_project::{Label, SequenceSettings, TrackKind, Transition, TransitionAlign, TransitionId, find_effect};
    use filmcraft_time::{FrameRate, Tick, TimeRange};
    use filmcraft_ui_egui::panels::timeline::Drag;

    let mut session = Session::default();
    let mut project = (*session.project).clone();
    let rate = FrameRate::FPS_25;
    let media = filmcraft_engine::demo::add_generator(
        &mut project,
        &session.media,
        GeneratorSource::new(Generator::ColorMatte { color: [0.2, 0.5, 0.8, 1.0] }, 32, 18, rate, rate.tick_of(100)),
        "Fullscreen drag fixture",
        Label::Iris,
        None,
    );
    let seq = project.new_sequence("Fullscreen drag", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 1, None);
    let a = project.make_track_item(media, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, rate.tick_of(50)), rate).unwrap();
    let b = project.make_track_item(media, TrackKind::Video, rate.tick_of(50), TimeRange::new(Tick::ZERO, rate.tick_of(50)), rate).unwrap();
    let id = TransitionId(project.alloc_id());
    let transition = Transition {
        id,
        effect: find_effect("cross_dissolve").unwrap().instance(),
        start: rate.tick_of(44),
        duration: rate.tick_of(12),
        from: Some(a.id),
        to: Some(b.id),
        align: TransitionAlign::CenterAtCut,
        reverse: false,
    };
    let click_clip = a.id;
    let track = &mut project.sequence_mut(seq).unwrap().video_tracks[0];
    track.items = vec![a, b];
    track.transitions = vec![transition];
    session.project = Arc::new(project);
    session.state.active_sequence = Some(seq);
    session.state.open_sequences = vec![seq];
    session.state.snapping = false;
    let mut d = Driver::with_session(session);
    d.ok("ui.set", json!({"timeline": {"pps": 120.0, "scroll": 0.0}}));
    let stripe_id = format!("timeline.transition.{}", id.0);
    let out_id = format!("{stripe_id}.out");
    let elements = d.elements();
    let original_stripe = elements.iter().find(|e| e["id"] == stripe_id).unwrap()["rect"].clone();
    let r = elements.iter().find(|e| e["id"] == out_id).unwrap()["rect"].as_array().unwrap();
    let from = egui::pos2((r[0].as_f64().unwrap() + r[2].as_f64().unwrap() / 2.0) as f32, (r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0) as f32);
    d.harness.input_mut().events.push(egui::Event::PointerMoved(from));
    d.frames(1);
    d.harness.input_mut().events.push(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    d.frames(1);
    let to = from + egui::vec2(30.0, 0.0);
    d.harness.input_mut().events.push(egui::Event::PointerMoved(to));
    d.frames(1);
    assert!(matches!(d.harness.state().tl.drag, Some(Drag::Transition { .. })), "the actual handle gesture has a provisional transition");
    let project = serde_json::to_value(&d.harness.state().session.project).unwrap();
    let history = d.harness.state().session.history.undo.len();
    let selection = d.harness.state().session.state.selection.clone();
    d.key("F11");
    assert!(d.harness.state().tl.drag.is_none());
    assert_eq!(d.harness.state().session.state.selection, selection);
    d.harness.input_mut().events.push(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    d.frames(2);
    d.key("Escape");
    assert_eq!(serde_json::to_value(&d.harness.state().session.project).unwrap(), project);
    assert_eq!(d.harness.state().session.history.undo.len(), history);
    let elements = d.elements();
    assert_eq!(elements.iter().find(|e| e["id"] == stripe_id).unwrap()["rect"], original_stripe, "the original transition stripe is restored");
    d.harness.state_mut().session.state.selection.clear();
    d.ok("ui.click", json!({"id": format!("timeline.clip.{}", click_clip.0)}));
    assert!(d.harness.state().session.state.selection.contains(&click_clip), "timeline clicks work after the interrupted gesture");
}
