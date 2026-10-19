//! Project replacement discards pending UI edits before project-local IDs can be reused.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::{BinId, ItemId};
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn demo() -> Self {
        Self::launch(false)
    }

    fn parameter_edits() -> Self {
        Self::launch(true)
    }

    fn launch(parameters_only: bool) -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        // The tests exercise the drafts' own Enter handling, without scheduling render jobs.
        session.execute("shortcuts.set", json!({"command": "edit.deselectAll", "keys": "Enter"})).unwrap();
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(session).with_control(rx);
        if parameters_only {
            app.ui.dock = filmcraft_ui_egui::dock::DockNode::Tabs { panels: vec![filmcraft_ui_egui::dock::PanelKind::EffectControls], active: 0 };
        }
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { harness, tx };
        d.frames(4);
        d
    }

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
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
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], true, "{method} {params}: {value}");
                return value["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }

    fn has(&mut self, id: &str) -> bool {
        let elements = self.ok("ui.elements", json!({"prefix": id}));
        elements.as_array().unwrap().iter().any(|e| e["id"] == id)
    }

    fn point(&mut self, id: &str, x: f64, y: f64) -> egui::Pos2 {
        let elements = self.ok("ui.elements", json!({"prefix": id}));
        let e = elements.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("no {id}: {elements}"));
        let r = &e["rect"];
        egui::pos2((r[0].as_f64().unwrap() + r[2].as_f64().unwrap() * x) as f32, (r[1].as_f64().unwrap() + r[3].as_f64().unwrap() * y) as f32)
    }

    fn event(&mut self, event: egui::Event) {
        self.harness.input_mut().events.push(event);
        self.frames(1);
    }

    fn open_footage(&mut self) {
        assert_eq!(self.app().session.project.root.find_bin(BinId(1)).unwrap().name, "Footage");
        self.exec("projectPanel.openBin", json!({"bin": 1, "how": "inPlace"}));
        self.frames(3);
    }

    fn replace_text_parameter_target(&mut self, replacement: &str, clip: filmcraft_project::ClipId) {
        // Queue a real control-channel batch before the next UI pass. The same text field can
        // remain focused when a new project is opened and its reused IDs are selected immediately.
        let mut commands = Vec::new();
        if replacement == "file.newProject" {
            commands.push(("file.newProject", json!({"name": "Replacement"})));
        }
        commands.extend([
            ("file.openDemoProject", json!({})),
            ("timeline.select", json!({"clips": [clip.0]})),
            ("effects.apply", json!({"clips": [clip.0], "effect": "simple_text"})),
        ]);
        self.exec_batch(commands);
    }

    fn exec_batch(&mut self, commands: Vec<(&str, Value)>) {
        let mut pending = Vec::new();
        for (command, params) in commands {
            let (request, reply) = ControlRequest::new("engine.execute", json!({"command": command, "params": params}));
            self.tx.send(request).unwrap();
            pending.push((command, reply));
        }
        for _ in 0..600 {
            self.frames(1);
            pending.retain(|(command, reply)| {
                if let Ok(value) = reply.try_recv() {
                    assert_eq!(value["ok"], true, "{command}: {value}");
                    false
                } else {
                    true
                }
            });
            if pending.is_empty() {
                self.frames(3);
                return;
            }
        }
        panic!("project replacement batch did not complete");
    }

    fn finish_parameter_edit(&mut self, enter: bool) {
        if enter {
            self.ok("ui.key", json!({"key": "Enter"}));
        } else {
            // An actual click outside the editor exercises its focus-loss commit path.
            self.ok("ui.click", json!({"x": 1590.0, "y": 970.0}));
        }
        self.frames(3);
    }

    fn only_panel(&mut self, panel: filmcraft_ui_egui::dock::PanelKind) {
        self.app().ui.dock = filmcraft_ui_egui::dock::DockNode::Tabs { panels: vec![panel], active: 0 };
        self.app().ui.focused = panel;
        self.frames(3);
    }

    fn first_caption(&mut self) -> filmcraft_project::Caption {
        self.app().session.active_sequence().unwrap().caption_tracks[0].captions[0].clone()
    }

    fn replacement_commands(replacement: &str) -> Vec<(&'static str, Value)> {
        let mut commands = Vec::new();
        if replacement == "file.newProject" {
            commands.push(("file.newProject", json!({"name": "Replacement"})));
        }
        commands.push(("file.openDemoProject", json!({})));
        commands
    }

    fn add_original_caption(&mut self) {
        self.exec("captions.newTrack", json!({}));
        self.exec("captions.add", json!({"text": "Original caption", "time": 0, "durationSeconds": 3}));
        self.frames(3);
    }

    fn replace_caption_with_reused_id(&mut self, replacement: &str) {
        // Recreate the same field before the next UI pass, retaining its old widget focus.
        let mut commands = Self::replacement_commands(replacement);
        commands.extend([("captions.newTrack", json!({})), ("captions.add", json!({"text": "Original caption", "time": 0, "durationSeconds": 3}))]);
        self.exec_batch(commands);
    }

    fn first_clip(&mut self) -> filmcraft_project::TrackItem {
        self.app().session.active_sequence().unwrap().video_tracks[0].items[0].clone()
    }

    fn replace_with_reused_demo_ids(&mut self, replacement: &str) {
        if replacement == "file.newProject" {
            self.exec(replacement, json!({"name": "Replacement"}));
            self.frames(3);
        }
        self.exec("file.openDemoProject", json!({}));
        self.frames(3);
    }
}

#[test]
fn project_replacement_discards_pen_mask_before_enter_can_edit_a_reused_clip() {
    for replacement in ["file.newProject", "file.openDemoProject"] {
        let mut d = Driver::demo();
        d.ok("ui.set", json!({"workspace": "Effects"}));
        let old_clip = d.app().session.active_sequence().unwrap().video_tracks[0].items[0].clone();
        d.exec("playhead.set", json!({"time": old_clip.start.0 + old_clip.duration.0 / 2}));
        d.exec("timeline.select", json!({"clips": [old_clip.id.0]}));
        d.click("effectControls.effect.motion");
        d.click("effectControls.effect.time_remap");
        d.click("effectControls.opacity.mask.pen");
        for (x, y) in [(0.3, 0.3), (0.6, 0.3), (0.6, 0.7)] {
            let p = d.point("program.picture", x, y);
            d.ok("ui.click", json!({"x": p.x, "y": p.y}));
            d.frames(2);
        }
        assert_eq!(d.app().ui.mask_pen.as_ref().unwrap().points.len(), 3, "a real unfinished pen path would commit on Enter");
        let prefs = d.app().session.prefs.clone();
        let dock = serde_json::to_value(&d.app().ui.dock).unwrap();
        d.replace_with_reused_demo_ids(replacement);
        let new_clip = d.app().session.active_sequence().unwrap().video_tracks[0].items[0].clone();
        assert_eq!(new_clip.id, old_clip.id);
        assert!(d.app().ui.mask_pen.is_none());
        assert!(!d.has("program.maskPen"));
        let project = d.app().session.project.clone();
        let revision = d.app().session.revision;
        d.ok("ui.key", json!({"key": "Enter"}));
        d.frames(3);
        assert!(Arc::ptr_eq(&project, &d.app().session.project));
        assert_eq!(d.app().session.revision, revision);
        assert!(d.app().session.history.undo.is_empty());
        assert!(d.app().session.history.redo.is_empty());
        let clip = &d.app().session.active_sequence().unwrap().video_tracks[0].items[0];
        assert!(clip.effects.iter().all(|e| e.masks.is_empty()));
        assert_eq!(d.app().session.prefs, prefs);
        assert_eq!(serde_json::to_value(&d.app().ui.dock).unwrap(), dock);
        assert_eq!(d.app().ui.workspace, "Effects");
    }
}

#[test]
fn project_replacement_discards_inline_rename_before_focus_loss_or_enter() {
    for replacement in ["file.newProject", "file.openDemoProject"] {
        let mut d = Driver::demo();
        d.open_footage();
        let item = d.app().session.active_sequence().unwrap().video_tracks[0].items[0].item;
        let original_name = d.app().session.project.item(item).unwrap().name.clone();
        d.exec("project.select", json!({"items": [item.0]}));
        d.exec("projectPanel.rename", json!({"item": item.0}));
        assert!(d.has("project.rename"));
        d.ok("ui.key", json!({"key": "Cmd+A"}));
        d.ok("ui.type", json!({"text": "Old project draft"}));
        assert_eq!(d.app().ui.project_panel.rename.as_ref().unwrap().text, "Old project draft");
        d.replace_with_reused_demo_ids(replacement);
        assert!(d.app().ui.project_panel.rename.is_none());
        assert_eq!(d.app().session.project.item(item).unwrap().name, original_name, "replacement must not commit the old text on focus loss");
        assert!(d.app().session.history.undo.is_empty());
        d.open_footage();
        let project = d.app().session.project.clone();
        let revision = d.app().session.revision;
        d.ok("ui.key", json!({"key": "Enter"}));
        d.frames(3);
        assert!(Arc::ptr_eq(&project, &d.app().session.project));
        assert_eq!(d.app().session.project.item(item).unwrap().name, original_name);
        assert_eq!(d.app().session.revision, revision);
        assert!(d.app().session.history.undo.is_empty());
        assert!(!d.has("project.rename"));
    }
}

#[test]
fn held_project_item_drag_cannot_drop_into_a_replacement_project() {
    for replacement in [None, Some("file.newProject"), Some("file.openDemoProject")] {
        let mut d = Driver::demo();
        d.open_footage();
        let seq = d.app().session.active_sequence().unwrap();
        let (item, target_clip) = (seq.video_tracks[0].items[0].item, seq.video_tracks[0].items[2].id);
        let from = d.point(&format!("project.item.{}", item.0), 0.3, 0.5);
        let to = d.point(&format!("timeline.clip.{}", target_clip.0), 0.5, 0.5);
        d.event(egui::Event::PointerMoved(from));
        d.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        for step in 1..=8 {
            d.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
        }
        if let Some(replacement) = replacement {
            d.replace_with_reused_demo_ids(replacement);
            assert!(d.app().session.project.item(item).is_some(), "the dragged ItemId really is reused");
        }
        let project = d.app().session.project.clone();
        let revision = d.app().session.revision;
        let history = d.app().session.history.undo.len();
        let to = d.point(&format!("timeline.clip.{}", target_clip.0), 0.5, 0.5);
        d.event(egui::Event::PointerMoved(to));
        d.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        d.frames(3);
        if replacement.is_none() {
            assert!(!Arc::ptr_eq(&project, &d.app().session.project), "the positive control proves the held drag normally places media");
            assert_eq!(d.app().session.history.undo.len(), history + 1);
            d.exec("edit.undo", json!({}));
            assert_eq!(serde_json::to_value(&*d.app().session.project).unwrap(), serde_json::to_value(&*project).unwrap());
        } else {
            assert!(Arc::ptr_eq(&project, &d.app().session.project));
            assert_eq!(d.app().session.revision, revision);
            assert_eq!(d.app().session.history.undo.len(), history);
        }
    }
}

#[test]
fn replacement_clears_id_bound_dialogs_and_retains_preferences_and_library_state() {
    use filmcraft_ui_egui::panels::{audio_fx_editor::FxTarget, project::ProjectDialog};
    use filmcraft_ui_egui::state::{ColorDialog, EditCamerasDraft, ProxyDraft, SavePresetDraft, SyncDraft, TextPropsDialog};

    let mut d = Driver::demo();
    let seq = d.app().session.active_sequence().unwrap();
    let clip = seq.video_tracks[0].items[0].id.0;
    let item = seq.video_tracks[0].items[0].item.0;
    let sequence = d.app().session.state.active_sequence.unwrap().0;
    let ui = &mut d.app().ui;
    ui.save_preset = Some(SavePresetDraft { clip, effects: vec![1], name: "Pending".into(), ..Default::default() });
    ui.text_props_dialog = Some(TextPropsDialog { clip, layer: 0, paragraph: false, vertical: false, ligatures: true, ligatures_was: true });
    ui.audio_fx_editors.push(FxTarget::Clip { clip, index: 1 });
    ui.sync_dialog = Some(SyncDraft { kind: "merge".into(), items: vec![item], ..Default::default() });
    ui.edit_cameras = Some(EditCamerasDraft { sequence, names: vec!["Old".into()], enabled: vec![true] });
    ui.create_proxies = Some(ProxyDraft { items: vec![item], preset: "Old".into(), destination: String::new() });
    ui.color_dialog = Some(ColorDialog::Interpret { items: vec![item], color_space: "auto".into() });
    ui.project_search = "User filter".into();
    ui.effects_search = "Blur".into();
    ui.gfx_templates.query = "Title".into();
    ui.gfx_templates.category = "Minimal".into();
    ui.project_panel.dialog = Some(ProjectDialog::MetadataDisplay { columns: vec!["Name".into()], filter: "Tape".into() });
    let prefs = d.app().session.prefs.clone();
    d.exec("file.openDemoProject", json!({}));
    let ui = &d.app().ui;
    assert!(ui.save_preset.is_none() && ui.text_props_dialog.is_none() && ui.audio_fx_editors.is_empty());
    assert!(ui.sync_dialog.is_none() && ui.edit_cameras.is_none() && ui.create_proxies.is_none() && ui.color_dialog.is_none());
    assert_eq!(ui.project_search, "User filter");
    assert_eq!(ui.effects_search, "Blur");
    assert_eq!((ui.gfx_templates.query.as_str(), ui.gfx_templates.category.as_str()), ("Title", "Minimal"));
    assert!(matches!(&ui.project_panel.dialog, Some(ProjectDialog::MetadataDisplay { filter, .. }) if filter == "Tape"));
    assert_eq!(d.app().session.prefs, prefs);
    assert!(d.app().session.project.item(ItemId(item)).is_some());
    assert!(d.app().session.history.undo.is_empty());
}

#[test]
fn replacement_discards_numeric_parameter_text_before_enter_or_focus_loss() {
    for enter in [true, false] {
        for replacement in [None, Some("file.newProject"), Some("file.openDemoProject")] {
            let mut d = Driver::parameter_edits();
            let original = d.first_clip();
            d.exec("timeline.select", json!({"clips": [original.id.0]}));
            d.frames(3);
            let before = d.app().session.project.clone();
            let history = d.app().session.history.undo.len();
            d.click("effectControls.motion.scale.value");
            d.ok("ui.key", json!({"key": "Cmd+A"}));
            d.ok("ui.type", json!({"text": "125"}));
            d.frames(2);
            assert!(Arc::ptr_eq(&before, &d.app().session.project), "typing only edits the numeric draft");
            assert_eq!(d.first_clip().effect("motion").unwrap().param("scale").unwrap().value.as_f64(), Some(100.0));

            if let Some(replacement) = replacement {
                let global_draft = egui::Id::new("preferences-global-numeric-draft-fixture");
                d.harness.ctx.data_mut(|data| data.insert_temp(global_draft, String::from("42")));
                d.replace_with_reused_demo_ids(replacement);
                assert_eq!(d.first_clip().id, original.id, "the replacement really reuses the edited ClipId");
                let project = d.app().session.project.clone();
                let revision = d.app().session.revision;
                d.exec("timeline.select", json!({"clips": [original.id.0]}));
                d.frames(3);
                assert!(Arc::ptr_eq(&project, &d.app().session.project), "showing the reused numeric widget cannot commit old text");
                d.finish_parameter_edit(enter);
                assert!(Arc::ptr_eq(&project, &d.app().session.project));
                assert_eq!(d.first_clip().effect("motion").unwrap().param("scale").unwrap().value.as_f64(), Some(100.0));
                assert_eq!(d.app().session.revision, revision);
                assert!(d.app().session.history.undo.is_empty());
                assert!(d.app().session.history.redo.is_empty());
                assert_eq!(
                    d.harness.ctx.data(|data| data.get_temp::<String>(global_draft)).as_deref(),
                    Some("42"),
                    "unrelated preference text remains in egui memory"
                );
            } else {
                d.finish_parameter_edit(enter);
                assert_eq!(
                    d.first_clip().effect("motion").unwrap().param("scale").unwrap().value.as_f64(),
                    Some(125.0),
                    "the positive control commits the actual typed value"
                );
                assert_eq!(d.app().session.history.undo.len(), history + 1);
                d.exec("edit.undo", json!({}));
                assert_eq!(serde_json::to_value(&*d.app().session.project).unwrap(), serde_json::to_value(&*before).unwrap());
            }
        }
    }
}

#[test]
fn replacement_discards_text_parameter_drafts_before_enter_or_focus_loss() {
    for enter in [true, false] {
        for replacement in [None, Some("file.newProject"), Some("file.openDemoProject")] {
            let mut d = Driver::parameter_edits();
            let original = d.first_clip();
            d.exec("timeline.select", json!({"clips": [original.id.0]}));
            d.exec("effects.apply", json!({"clips": [original.id.0], "effect": "simple_text"}));
            d.frames(3);
            let index = d.first_clip().effects.iter().position(|effect| effect.effect == "simple_text").unwrap();
            let before = d.app().session.project.clone();
            let history = d.app().session.history.undo.len();
            d.click(&format!("effectControls.text.{index}.text"));
            d.ok("ui.key", json!({"key": "Cmd+A"}));
            d.ok("ui.type", json!({"text": "Old project draft"}));
            d.frames(2);
            assert!(Arc::ptr_eq(&before, &d.app().session.project), "typing only edits the text draft");
            assert_eq!(
                serde_json::to_value(&d.first_clip().effect("simple_text").unwrap().param("text").unwrap().value).unwrap(),
                json!({"Text": "Simple Text"})
            );

            if let Some(replacement) = replacement {
                d.replace_text_parameter_target(replacement, original.id);
                assert_eq!(d.first_clip().id, original.id);
                let history = 0;
                assert_eq!(
                    d.first_clip().effects.iter().position(|effect| effect.effect == "simple_text"),
                    Some(index),
                    "clip, effect and parameter IDs are all reused"
                );
                assert_eq!(d.app().session.history.undo.len(), history + 1, "only applying the new effect may edit the replacement");
                assert_eq!(
                    serde_json::to_value(&d.first_clip().effect("simple_text").unwrap().param("text").unwrap().value).unwrap(),
                    json!({"Text": "Simple Text"})
                );
                let project = d.app().session.project.clone();
                let revision = d.app().session.revision;
                d.finish_parameter_edit(enter);
                assert!(Arc::ptr_eq(&project, &d.app().session.project));
                assert_eq!(d.app().session.revision, revision);
                assert_eq!(d.app().session.history.undo.len(), history + 1);
                assert!(d.app().session.history.redo.is_empty());
                assert_eq!(
                    serde_json::to_value(&d.first_clip().effect("simple_text").unwrap().param("text").unwrap().value).unwrap(),
                    json!({"Text": "Simple Text"})
                );
            } else {
                d.finish_parameter_edit(enter);
                assert_eq!(
                    serde_json::to_value(&d.first_clip().effect("simple_text").unwrap().param("text").unwrap().value).unwrap(),
                    json!({"Text": "Old project draft"}),
                    "the positive control commits the actual typed text"
                );
                assert_eq!(d.app().session.history.undo.len(), history + 1);
                d.exec("edit.undo", json!({}));
                assert_eq!(serde_json::to_value(&*d.app().session.project).unwrap(), serde_json::to_value(&*before).unwrap());
            }
        }
    }
}

#[test]
fn replacement_discards_metadata_text_before_enter_or_focus_loss() {
    use filmcraft_ui_egui::dock::PanelKind;

    for enter in [true, false] {
        for replacement in [None, Some("file.newProject"), Some("file.openDemoProject")] {
            let mut d = Driver::parameter_edits();
            d.only_panel(PanelKind::Metadata);
            let item = d.first_clip().item;
            let original_name = d.app().session.project.item(item).unwrap().name.clone();
            d.exec("project.select", json!({"items": [item.0]}));
            d.frames(3);
            let before = d.app().session.project.clone();
            let history = d.app().session.history.undo.len();
            d.click("metadata.field.Name");
            d.ok("ui.key", json!({"key": "Cmd+A"}));
            d.ok("ui.type", json!({"text": "Old metadata draft"}));
            d.frames(2);
            assert!(Arc::ptr_eq(&before, &d.app().session.project), "typing only edits the Metadata buffer");
            if let Some(replacement) = replacement {
                let mut commands = Driver::replacement_commands(replacement);
                commands.push(("project.select", json!({"items": [item.0]})));
                d.exec_batch(commands);
                assert_eq!(d.first_clip().item, item, "the new media item reuses the metadata target ID");
                assert_eq!(d.app().session.project.item(item).unwrap().name, original_name);
                assert!(d.app().session.history.undo.is_empty(), "repainting cannot commit the old Name draft");
                let project = d.app().session.project.clone();
                let revision = d.app().session.revision;
                let prefs = d.app().session.prefs.clone();
                d.finish_parameter_edit(enter);
                assert!(Arc::ptr_eq(&project, &d.app().session.project));
                assert_eq!(d.app().session.project.item(item).unwrap().name, original_name);
                assert_eq!(d.app().session.revision, revision);
                assert!(d.app().session.history.undo.is_empty());
                assert!(d.app().session.history.redo.is_empty());
                assert_eq!(d.app().session.prefs, prefs);
            } else {
                d.finish_parameter_edit(enter);
                assert_eq!(d.app().session.project.item(item).unwrap().name, "Old metadata draft", "ordinary Metadata edits still commit");
                assert_eq!(d.app().session.history.undo.len(), history + 1);
                d.exec("edit.undo", json!({}));
                assert_eq!(serde_json::to_value(&*d.app().session.project).unwrap(), serde_json::to_value(&*before).unwrap());
            }
        }
    }
}

#[test]
fn replacement_discards_caption_timecode_before_enter_or_focus_loss() {
    use filmcraft_ui_egui::dock::PanelKind;

    for edge in ["in", "out"] {
        for enter in [true, false] {
            for replacement in [None, Some("file.newProject"), Some("file.openDemoProject")] {
                let mut d = Driver::parameter_edits();
                d.only_panel(PanelKind::Text);
                d.app().ui.text_tab = "Captions".into();
                d.add_original_caption();
                let original = d.first_caption();
                let rate = d.app().session.sequence_rate();
                let before = d.app().session.project.clone();
                let history = d.app().session.history.undo.len();
                d.click(&format!("text.captions.{}.{edge}", original.id.0));
                d.ok("ui.key", json!({"key": "Cmd+A"}));
                let typed = if edge == "in" { "00:00:01:00" } else { "00:00:02:00" };
                d.ok("ui.type", json!({"text": typed}));
                d.frames(2);
                assert!(Arc::ptr_eq(&before, &d.app().session.project));
                if let Some(replacement) = replacement {
                    d.replace_caption_with_reused_id(replacement);
                    assert_eq!(d.first_caption(), original, "the recreated caption has the same ID and original timing");
                    assert_eq!(d.app().session.history.undo.len(), 2, "only creating the new track and caption may edit the replacement");
                    let project = d.app().session.project.clone();
                    let revision = d.app().session.revision;
                    d.finish_parameter_edit(enter);
                    assert!(Arc::ptr_eq(&project, &d.app().session.project));
                    assert_eq!(d.first_caption(), original);
                    assert_eq!(d.app().session.revision, revision);
                    assert_eq!(d.app().session.history.undo.len(), 2);
                    assert!(d.app().session.history.redo.is_empty());
                } else {
                    d.finish_parameter_edit(enter);
                    let caption = d.first_caption();
                    if edge == "in" {
                        assert_eq!(caption.start, rate.tick_of(24));
                        assert_eq!(caption.end(), original.end());
                    } else {
                        assert_eq!(caption.start, original.start);
                        assert_eq!(caption.end(), rate.tick_of(48));
                    }
                    assert_eq!(d.app().session.history.undo.len(), history + 1, "the normal timecode commit is one undo step");
                    d.exec("edit.undo", json!({}));
                    assert_eq!(serde_json::to_value(&*d.app().session.project).unwrap(), serde_json::to_value(&*before).unwrap());
                }
            }
        }
    }
}

#[test]
fn replacement_discards_caption_text_before_focus_loss() {
    use filmcraft_ui_egui::dock::PanelKind;

    for replacement in [None, Some("file.newProject"), Some("file.openDemoProject")] {
        let mut d = Driver::parameter_edits();
        d.only_panel(PanelKind::Text);
        d.app().ui.text_tab = "Captions".into();
        d.add_original_caption();
        let original = d.first_caption();
        let before = d.app().session.project.clone();
        let history = d.app().session.history.undo.len();
        d.click(&format!("text.captions.{}.text", original.id.0));
        d.ok("ui.key", json!({"key": "Cmd+A"}));
        d.ok("ui.type", json!({"text": "Old caption draft"}));
        d.frames(2);
        assert!(Arc::ptr_eq(&before, &d.app().session.project), "typing does not apply caption text");
        if let Some(replacement) = replacement {
            d.replace_caption_with_reused_id(replacement);
            assert_eq!(d.first_caption(), original, "the new caption reuses the ID with its original text");
            assert_eq!(d.app().session.history.undo.len(), 2);
            let project = d.app().session.project.clone();
            let revision = d.app().session.revision;
            d.finish_parameter_edit(false);
            assert!(Arc::ptr_eq(&project, &d.app().session.project));
            assert_eq!(d.first_caption(), original);
            assert_eq!(d.app().session.revision, revision);
            assert_eq!(d.app().session.history.undo.len(), 2);
            assert!(d.app().session.history.redo.is_empty());
        } else {
            d.finish_parameter_edit(false);
            assert_eq!(d.first_caption().text, "Old caption draft", "multiline caption text still commits on focus loss");
            assert_eq!(d.app().session.history.undo.len(), history + 1);
            d.exec("edit.undo", json!({}));
            assert_eq!(serde_json::to_value(&*d.app().session.project).unwrap(), serde_json::to_value(&*before).unwrap());
        }
    }
}
