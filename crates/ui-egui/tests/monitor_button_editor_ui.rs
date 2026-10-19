//! Actual monitor + editing, independent persisted toolbars and transient draft lifecycle.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::{Generator, generators::GeneratorSource};
use filmcraft_project::{ItemId, Label, Project, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use filmcraft_ui_egui::{FilmcraftApp, control::ControlRequest, dock::PanelKind, state::UiState};
use serde_json::{Value, json};

const SOURCE: [&str; 12] = [
    "markers.add",
    "src.markIn",
    "src.markOut",
    "src.goIn",
    "src.stepBack",
    "src.play",
    "src.stepFwd",
    "src.goOut",
    "source.insert",
    "source.overwrite",
    "exportFrame",
    "media.toggleProxies",
];
const PROGRAM: [&str; 13] = [
    "markers.add",
    "markers.markIn",
    "markers.markOut",
    "markers.goToIn",
    "playhead.stepBack",
    "playback.toggle",
    "playhead.stepForward",
    "markers.goToOut",
    "sequence.lift",
    "sequence.extract",
    "exportFrame",
    "media.toggleProxies",
    "view.programFullScreen",
];
struct Driver {
    h: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    source: ItemId,
}
impl Driver {
    fn new() -> Self {
        Self::with_size(egui::vec2(1600.0, 980.0))
    }
    fn with_size(size: egui::Vec2) -> Self {
        let mut session = Session::default();
        let mut project = Project::new("Monitor buttons");
        let rate = FrameRate::FPS_25;
        let duration = Tick::from_units(4, 1);
        let source = filmcraft_engine::demo::add_generator(
            &mut project,
            &session.media,
            GeneratorSource::new(Generator::ColorMatte { color: [0.0, 0.6, 0.0, 1.0] }, 32, 18, rate, duration),
            "Source",
            Label::Iris,
            None,
        );
        let picture = filmcraft_engine::demo::add_generator(
            &mut project,
            &session.media,
            GeneratorSource::new(Generator::ColorMatte { color: [0.7, 0.0, 0.0, 1.0] }, 32, 18, rate, duration),
            "Picture",
            Label::Iris,
            None,
        );
        let sequence = project.new_sequence("Program", SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() }, 1, 1, None);
        let item = project.make_track_item(picture, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, duration), rate).unwrap();
        project.sequence_mut(sequence).unwrap().video_tracks[0].items.push(item);
        session.project = Arc::new(project);
        session.state.active_sequence = Some(sequence);
        session.state.open_sequences = vec![sequence];
        session.set_playhead(rate.tick_of(3));
        session.execute("source.open", json!({"item": source.0})).unwrap();
        session.execute("source.setPlayhead", json!({"frame": 12})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let h = Harness::builder().with_size(size).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut driver = Self { h, tx, source };
        driver.frames(4);
        driver
    }
    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            let ctx = self.h.ctx.clone();
            let mut raw = std::mem::take(self.h.input_mut());
            eframe::App::raw_input_hook(self.h.state_mut(), &ctx, &mut raw);
            *self.h.input_mut() = raw;
            self.h.step();
        }
    }
    fn call(&mut self, method: &str, params: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                return value;
            }
        }
        panic!("no reply to {method} {params}");
    }
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params.clone());
        assert_eq!(reply["ok"], true, "{method} {params}: {reply}");
        self.frames(3);
        reply["result"].clone()
    }
    fn command(&mut self, params: Value) -> Value {
        self.ok("ui.menu.invoke", json!({"id": "view.monitorButtonEditor", "params": params}))
    }
    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
    }
    fn elements(&mut self, prefix: &str) -> Vec<Value> {
        self.ok("ui.elements", json!({"prefix": prefix})).as_array().unwrap().clone()
    }
    fn transport(&mut self, monitor: &str) -> Vec<String> {
        let prefix = format!("{monitor}.transport.");
        let mut buttons = self.elements(&prefix);
        buttons.sort_by(|a, b| a["rect"][0].as_f64().unwrap().total_cmp(&b["rect"][0].as_f64().unwrap()));
        buttons.into_iter().map(|button| button["id"].as_str().unwrap().strip_prefix(&prefix).unwrap().into()).collect()
    }
    fn editor(&mut self) -> Value {
        self.ok("ui.inspect", json!({}))["monitorButtonEditor"].clone()
    }
    fn snapshot(&self) -> Value {
        let app = self.h.state();
        json!({"project": app.session.project.as_ref(), "revision": app.session.revision, "journal": app.session.journal, "undoDepth": app.session.history.undo.len(), "redoDepth": app.session.history.redo.len(), "time": app.session.playhead(), "source": app.session.state.source_playhead, "dirty": app.session.is_dirty()})
    }
    fn pointer_plus(&mut self, monitor: &str) {
        // Existing public panel/toolbar geometry also makes this opening regression runnable
        // before the fix: it does not depend on the newly registered + automation ID.
        let title = if monitor == "source" { "Source" } else { "Program" };
        let panel = self.elements(&format!("panel.{title}")).into_iter().find(|element| element["id"] == format!("panel.{title}")).unwrap();
        let rect = &panel["rect"];
        let x = rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap() - 19.0;
        let y = rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap() - 17.0;
        self.ok("ui.click", json!({"x": x, "y": y}));
    }
}

#[test]
fn actual_plus_opens_a_scoped_modal_and_blocks_background_transport_and_shortcuts() {
    let mut d = Driver::new();
    d.ok("engine.execute", json!({"command": "markers.add", "params": {"target": "program", "name": "Undo positive control"}}));
    assert!(d.h.state().session.history.can_undo());
    assert_eq!(d.transport("source"), SOURCE);
    assert_eq!(d.transport("program"), PROGRAM);
    let before = d.snapshot();
    d.pointer_plus("source");
    assert_eq!(d.editor(), json!({"monitor": "source", "buttons": SOURCE}));
    let controls = d.elements("source.buttonEditor.");
    for suffix in ["window", "close", "reset", "cancel", "apply", "src.play.enabled", "src.play.up", "src.play.down"] {
        assert!(controls.iter().any(|control| control["id"] == format!("source.buttonEditor.{suffix}")), "missing control {suffix}");
    }
    for key in ["Space", "J", "L", "M", "Ctrl+Z"] {
        // Linux physical Ctrl also carries egui's logical command modifier.
        d.ok("ui.key", json!({"key": key, "command": key == "Ctrl+Z"}));
    }
    assert_eq!(d.snapshot(), before);
    assert!(!d.h.state().playback.playing);
    // The existing Source play button is outside the centered modal: its pointer cannot click
    // through the backdrop, and the outside click discards the editor instead.
    d.click("source.transport.src.play");
    assert!(!d.h.state().playback.playing);
    assert!(d.editor().is_null());
    assert_eq!(d.snapshot(), before);
    d.pointer_plus("program");
    assert_eq!(d.editor(), json!({"monitor": "program", "buttons": PROGRAM}));
    d.click("program.buttonEditor.close");
    assert!(d.editor().is_null());
    assert_eq!(d.snapshot(), before);
    // The same physical key works after close: blocking Ctrl+Z in the modal was not vacuous.
    d.ok("ui.key", json!({"key": "Ctrl+Z", "command": true}));
    assert_ne!(serde_json::to_value(d.h.state().session.project.as_ref()).unwrap(), before["project"]);
    assert!(d.h.state().session.history.can_redo());
    d.ok("engine.execute", json!({"command": "edit.redo"}));
    assert_eq!(serde_json::to_value(d.h.state().session.project.as_ref()).unwrap(), before["project"]);
}

#[test]
fn enabled_order_apply_is_per_monitor_and_real_steps_keep_their_original_target() {
    let mut d = Driver::new();
    let before = d.snapshot();
    d.pointer_plus("source");
    d.click("source.buttonEditor.media.toggleProxies.enabled");
    d.click("source.buttonEditor.src.play.up");
    let mut expected = SOURCE.map(String::from).to_vec();
    expected.pop();
    expected.swap(4, 5);
    assert_eq!(d.editor()["buttons"], json!(expected));
    assert_eq!(d.transport("source"), SOURCE, "only Apply changes the displayed transport");
    assert_eq!(d.transport("program"), PROGRAM);
    assert_eq!(d.snapshot(), before);
    d.click("source.buttonEditor.apply");
    assert!(d.editor().is_null());
    assert_eq!(d.transport("source"), expected);
    assert_eq!(d.transport("program"), PROGRAM);
    assert_eq!(d.snapshot(), before);
    d.ok("ui.set", json!({"focused": "Program"}));
    let program = d.h.state().session.playhead();
    let source = d.h.state().session.state.source_playhead;
    d.click("source.transport.src.stepFwd");
    assert_eq!(d.h.state().session.state.source_playhead, source + FrameRate::FPS_25.frame_duration());
    assert_eq!(d.h.state().session.playhead(), program);
    d.pointer_plus("program");
    d.click("program.buttonEditor.sequence.lift.enabled");
    d.click("program.buttonEditor.view.programFullScreen.up");
    let mut program_expected = PROGRAM.map(String::from).to_vec();
    program_expected.remove(8);
    program_expected.swap(10, 11);
    d.click("program.buttonEditor.apply");
    assert_eq!(d.transport("program"), program_expected);
    assert_eq!(d.transport("source"), expected);
    d.ok("ui.set", json!({"focused": "Source"}));
    let source = d.h.state().session.state.source_playhead;
    d.click("program.transport.playhead.stepBack");
    assert_eq!(d.h.state().session.playhead(), program - FrameRate::FPS_25.frame_duration());
    assert_eq!(d.h.state().session.state.source_playhead, source);
    let saved = serde_json::to_value(&d.h.state().ui).unwrap();
    assert_eq!(saved["source"]["toolbar"], json!(expected));
    assert_eq!(saved["program"]["toolbar"], json!(program_expected));
    let restored: UiState = serde_json::from_value(saved).unwrap();
    d.h.state_mut().ui = restored;
    d.frames(4);
    assert_eq!(d.transport("source"), expected);
    assert_eq!(d.transport("program"), program_expected);
}

#[test]
fn empty_toolbar_keeps_plus_and_reset_cancel_apply_have_distinct_results() {
    let mut d = Driver::new();
    let before = d.snapshot();
    d.pointer_plus("source");
    d.command(json!({"action": "set", "buttons": []}));
    d.click("source.buttonEditor.apply");
    assert!(d.transport("source").is_empty());
    assert!(d.elements("source.buttonEditor").iter().any(|element| element["id"] == "source.buttonEditor"));
    assert_eq!(d.transport("program"), PROGRAM);
    d.click("source.buttonEditor");
    d.click("source.buttonEditor.reset");
    assert_eq!(d.editor()["buttons"], json!(SOURCE));
    assert!(d.transport("source").is_empty());
    d.click("source.buttonEditor.cancel");
    assert!(d.transport("source").is_empty());
    d.click("source.buttonEditor");
    d.click("source.buttonEditor.reset");
    d.ok("ui.key", json!({"key": "Escape"}));
    assert!(d.transport("source").is_empty());
    d.click("source.buttonEditor");
    d.click("source.buttonEditor.reset");
    d.click("source.buttonEditor.apply");
    assert_eq!(d.transport("source"), SOURCE);
    assert_eq!(d.transport("program"), PROGRAM);
    assert!(d.h.state().ui.source.toolbar.is_none(), "applied defaults retain the old missing-field representation");
    assert_eq!(d.snapshot(), before);
}

#[test]
fn invalid_commands_leave_the_live_draft_toolbar_and_project_unchanged() {
    let mut d = Driver::new();
    d.pointer_plus("source");
    let editor = d.editor();
    let before = d.snapshot();
    for params in [
        json!({"monitor": "other", "action": "apply"}),
        json!({"monitor": 0, "action": "apply"}),
        json!({"monitor": "program", "action": "apply"}),
        json!({"action": 3}),
        json!({"action": "unknown"}),
        json!({"action": "open", "buttons": ["unknown"]}),
        json!({"action": "reset", "buttons": []}),
        json!({"action": "set", "buttons": "src.play"}),
        json!({"action": "set", "buttons": [3]}),
        json!({"action": "set", "buttons": ["unknown"]}),
        json!({"action": "set", "buttons": ["playback.toggle"]}),
        json!({"action": "set", "buttons": ["src.play", "src.play"]}),
        json!({"action": "set", "buttons": vec!["src.play"; 13]}),
        json!({"action": "toggle", "button": "unknown", "enabled": true}),
        json!({"action": "toggle", "button": "src.play", "enabled": 1}),
        json!({"action": "move", "button": "src.play", "position": -1}),
        json!({"action": "move", "button": "src.play", "position": u64::MAX}),
        json!({"action": "move", "button": "src.play", "position": 12}),
    ] {
        let reply = d.call("ui.menu.invoke", json!({"id": "view.monitorButtonEditor", "params": params}));
        assert_eq!(reply["ok"], false, "{params}: {reply}");
        assert_eq!(d.editor(), editor, "failed command mutated its draft: {params}");
        assert_eq!(d.transport("source"), SOURCE);
        assert_eq!(d.transport("program"), PROGRAM);
        assert_eq!(d.snapshot(), before);
    }
    d.command(json!({"action": "toggle", "button": "src.play", "enabled": false}));
    let editor = d.editor();
    let reply = d.call("ui.menu.invoke", json!({"id": "view.monitorButtonEditor", "params": {"action": "move", "button": "src.play", "position": 0}}));
    assert_eq!(reply["ok"], false);
    assert_eq!(d.editor(), editor);
    d.command(json!({"action": "cancel"}));
}

#[test]
fn hidden_closed_maximized_and_fullscreen_monitors_discard_unapplied_drafts() {
    let mut d = Driver::new();
    let before = d.snapshot();
    d.pointer_plus("source");
    d.command(json!({"action": "set", "buttons": []}));
    d.h.state_mut().ui.dock.activate(PanelKind::EffectControls);
    d.frames(4);
    assert!(d.editor().is_null());
    assert!(d.h.state().ui.source.toolbar.is_none());
    d.ok("ui.panel.show", json!({"panel": "Source"}));
    d.pointer_plus("source");
    d.ok("ui.panel.close", json!({"panel": "Source"}));
    assert!(d.editor().is_null());
    d.ok("ui.panel.show", json!({"panel": "Source"}));
    d.pointer_plus("source");
    d.h.state_mut().ui.keys.maximized = Some(PanelKind::Program);
    d.frames(4);
    assert!(d.editor().is_null());
    assert!(d.h.state().ui.source.toolbar.is_none());
    d.h.state_mut().ui.keys.maximized = None;
    d.frames(4);
    d.pointer_plus("program");
    d.ok("ui.menu.invoke", json!({"id": "view.programFullScreen"}));
    assert!(d.editor().is_null());
    assert!(d.h.state().ui.program.toolbar.is_none());
    d.ok("ui.key", json!({"key": "Escape"}));
    d.pointer_plus("source");
    let sequence = d.h.state().session.state.active_sequence.unwrap();
    d.ok("engine.execute", json!({"command": "source.open", "params": {"item": sequence.0}}));
    assert!(d.editor().is_null());
    assert!(d.h.state().ui.source.toolbar.is_none());
    assert_eq!(serde_json::to_value(d.h.state().session.project.as_ref()).unwrap(), before["project"]);
    assert_eq!(d.h.state().session.playhead().0, before["time"].as_i64().unwrap());
}

#[test]
fn a_replaced_session_cannot_apply_a_still_upgradable_old_draft_before_the_ui_pass() {
    let mut d = Driver::new();
    d.pointer_plus("source");
    d.command(json!({"action": "set", "buttons": []}));
    // Keep the old pool alive, as previous frame/render work can. Weak upgrade alone is not a
    // session identity check, and a delayed command may arrive before sync_pool clears dialogs.
    let old_pool = d.h.state().session.media.clone();
    let previous = d.h.state().session.project.clone();
    let state = d.h.state().session.state.clone();
    let mut replacement = Session::default();
    replacement.project = previous;
    replacement.state = state;
    assert_eq!(replacement.state.source_item, Some(d.source));
    assert!(!Arc::ptr_eq(&old_pool, &replacement.media));
    d.h.state_mut().session = replacement;
    let ctx = d.h.ctx.clone();
    let result = filmcraft_ui_egui::menus::invoke(d.h.state_mut(), &ctx, "view.monitorButtonEditor", json!({"monitor": "source", "action": "apply"}));
    assert!(result.is_err());
    assert!(d.h.state().monitor_button_editor.is_none());
    assert!(d.h.state().ui.source.toolbar.is_none());
    d.frames(4);
    assert!(d.editor().is_null());
    assert_eq!(d.transport("source"), SOURCE);
}

#[test]
fn old_ui_state_defaults_and_corrupt_saved_ids_render_a_bounded_monitor_subset() {
    let mut d = Driver::new();
    let old = serde_json::to_value(&d.h.state().ui).unwrap();
    assert!(old["source"].get("toolbar").is_none() && old["program"].get("toolbar").is_none());
    let restored: UiState = serde_json::from_value(old.clone()).unwrap();
    d.h.state_mut().ui = restored;
    d.frames(4);
    assert_eq!(d.transport("source"), SOURCE);
    assert_eq!(d.transport("program"), PROGRAM);
    let mut damaged = old;
    damaged["source"]["toolbar"] =
        json!(["unknown", "src.play", "src.play", "playback.toggle", "src.stepFwd", "markers.add", "bad", "bad", "bad", "bad", "bad", "bad", "src.markIn"]);
    d.h.state_mut().ui = serde_json::from_value(damaged).unwrap();
    d.frames(4);
    assert_eq!(
        d.transport("source"),
        ["src.play", "src.stepFwd", "markers.add"],
        "unknown/duplicate/cross-monitor IDs are ignored and only the first12 persisted entries are inspected"
    );
    assert_eq!(d.transport("program"), PROGRAM);
    d.click("source.buttonEditor");
    assert_eq!(d.editor()["buttons"], json!(["src.play", "src.stepFwd", "markers.add"]));
    d.click("source.buttonEditor.reset");
    d.click("source.buttonEditor.apply");
    assert_eq!(d.transport("source"), SOURCE);
}

#[test]
fn an_existing_save_preset_window_keeps_its_draft_and_excludes_the_monitor_editor() {
    use filmcraft_ui_egui::state::SavePresetDraft;
    let mut d = Driver::new();
    let before = d.snapshot();
    let clip = d.h.state().session.active_sequence().unwrap().video_tracks[0].items[0].id;
    let preset = SavePresetDraft { clip: clip.0, effects: vec![0], name: "Existing preset".into(), description: String::new(), keyframes: "scale".into() };
    d.h.state_mut().ui.save_preset = Some(preset.clone());
    d.frames(4);
    assert!(d.elements("savePreset.").iter().any(|element| element["id"] == "savePreset.name"));
    let reply = d.call("ui.menu.invoke", json!({"id": "view.monitorButtonEditor", "params": {"monitor": "source"}}));
    assert_eq!(reply["ok"], false);
    assert!(d.editor().is_null());
    assert_eq!(d.h.state().ui.save_preset, Some(preset.clone()));
    assert_eq!(d.snapshot(), before);
    d.click("savePreset.cancel");
    d.pointer_plus("source");
    d.command(json!({"action": "set", "buttons": []}));
    d.h.state_mut().ui.save_preset = Some(preset.clone());
    d.frames(4);
    assert!(d.editor().is_null());
    assert!(d.h.state().ui.source.toolbar.is_none());
    assert_eq!(d.h.state().ui.save_preset, Some(preset));
    assert_eq!(d.snapshot(), before);
    assert!(d.elements("savePreset.").iter().any(|element| element["id"] == "savePreset.name"));
    d.click("savePreset.cancel");
}

#[test]
fn small_viewports_keep_the_footer_visible_and_scroll_to_real_last_row_edits() {
    fn rect(d: &mut Driver, id: &str) -> egui::Rect {
        let element = d.elements(id).into_iter().find(|element| element["id"] == id).unwrap_or_else(|| panic!("missing visible {id}"));
        let r = element["rect"].as_array().unwrap();
        egui::Rect::from_min_size(
            egui::pos2(r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32),
            egui::vec2(r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32),
        )
    }
    fn contained(outer: egui::Rect, inner: egui::Rect, what: &str) {
        assert!(inner.is_positive() && outer.expand(0.75).contains_rect(inner), "{what}: {inner:?} outside {outer:?}");
    }
    fn geometry(d: &mut Driver, monitor: &str, viewport: egui::Rect) -> egui::Rect {
        let pre = format!("{monitor}.buttonEditor");
        // These existing controls make the before failure an actual offscreen footer/header,
        // rather than the absence of the new list/modal geometry automation.
        for suffix in ["close", "reset", "cancel", "apply"] {
            let button = rect(d, &format!("{pre}.{suffix}"));
            contained(viewport, button, suffix);
            assert!(button.height() >= 16.0, "{suffix} was clipped: {button:?}");
        }
        let header = rect(d, &format!("{pre}.window"));
        contained(viewport, header, "header");
        assert!(header.width() <= 470.75);
        assert!(header.width() + 28.0 <= viewport.width());
        let bounds = rect(d, &format!("{pre}.bounds"));
        contained(viewport, bounds, "entire modal");
        let list = rect(d, &format!("{pre}.list"));
        contained(bounds, list, "scroll viewport");
        for suffix in ["close", "reset", "cancel", "apply"] {
            contained(bounds, rect(d, &format!("{pre}.{suffix}")), suffix);
        }
        list
    }
    for size in [egui::vec2(360.0, 300.0), egui::vec2(360.0, 420.0), egui::vec2(640.0, 480.0)] {
        for (monitor, last, defaults) in [("program", "view.programFullScreen", PROGRAM.as_slice()), ("source", "media.toggleProxies", SOURCE.as_slice())] {
            let mut d = Driver::with_size(size);
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let before = d.snapshot();
            d.command(json!({"monitor": monitor}));
            let list = geometry(&mut d, monitor, viewport);
            let pre = format!("{monitor}.buttonEditor");
            let id = format!("{pre}.{last}.enabled");
            if size.x == 360.0 {
                assert!(!d.elements(&id).iter().any(|element| element["id"] == id), "the bottom catalog row must begin below the clipped list in this fixture");
            }
            d.ok("ui.scroll", json!({"x": list.center().x, "y": list.center().y, "dy": -10000}));
            d.frames(12);
            let list = geometry(&mut d, monitor, viewport);
            contained(list, rect(&mut d, &id), "last row checkbox after physical scrolling");
            contained(list, rect(&mut d, &format!("{pre}.{last}.up")), "last row Up after physical scrolling");
            d.click(&id);
            let selected: Vec<String> = defaults[..defaults.len() - 1].iter().map(|id| (*id).into()).collect();
            assert_eq!(d.editor()["buttons"], json!(selected));
            assert_eq!(d.snapshot(), before);
            d.click(&format!("{pre}.cancel"));
            assert!(d.editor().is_null());
            assert!(d.h.state().ui.source.toolbar.is_none() && d.h.state().ui.program.toolbar.is_none());
            assert_eq!(d.snapshot(), before);
            d.command(json!({"monitor": monitor}));
            d.command(json!({"action": "set", "buttons": selected}));
            geometry(&mut d, monitor, viewport);
            d.click(&format!("{pre}.apply"));
            assert!(d.editor().is_null());
            let saved = if monitor == "source" { &d.h.state().ui.source.toolbar } else { &d.h.state().ui.program.toolbar };
            assert_eq!(saved.as_ref().unwrap(), &selected);
            assert_eq!(d.snapshot(), before);
            d.command(json!({"monitor": monitor}));
            geometry(&mut d, monitor, viewport);
            d.click(&format!("{pre}.reset"));
            assert_eq!(d.editor()["buttons"], json!(defaults));
            d.click(&format!("{pre}.cancel"));
            let saved = if monitor == "source" { &d.h.state().ui.source.toolbar } else { &d.h.state().ui.program.toolbar };
            assert_eq!(saved.as_ref().unwrap(), &selected, "Reset+Cancel must keep the applied subset");
            d.command(json!({"monitor": monitor}));
            geometry(&mut d, monitor, viewport);
            d.click(&format!("{pre}.reset"));
            d.click(&format!("{pre}.apply"));
            assert!(d.h.state().ui.source.toolbar.is_none() && d.h.state().ui.program.toolbar.is_none());
            assert_eq!(d.snapshot(), before);
        }
    }
    let mut desktop = Driver::new();
    desktop.command(json!({"monitor": "program"}));
    let header = rect(&mut desktop, "program.buttonEditor.window");
    assert!((header.width() - 470.0).abs() < 0.1, "desktop editor keeps its existing width");
    desktop.click("program.buttonEditor.cancel");
}
