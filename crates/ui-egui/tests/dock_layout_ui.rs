//! Ordinary panel-tab moves and temporary maximize, driven through the real dock chrome.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::{ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings};
use filmcraft_time::{FrameRate, Tick};
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::dock::{DockNode, PanelKind, SplitSize};
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    snapshots: Option<PathBuf>,
}

impl Driver {
    fn launch(data: &Path) -> Self {
        let rate = FrameRate::FPS_24;
        let source = GeneratorSource::new(Generator::BlackVideo, 32, 18, rate, Tick::from_seconds_f64(4.0));
        let mut project = Project::new("Dock layout");
        let item = project.add_item(
            "Black",
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
        let settings = SequenceSettings { width: 32, height: 18, frame_rate: rate, ..Default::default() };
        let first = project.new_sequence("Cut", settings.clone(), 1, 1, None);
        let second = project.new_sequence("Other", settings, 1, 1, None);
        let mut session = Session::default();
        session.project = Arc::new(project);
        session.state.active_sequence = Some(first);
        session.state.open_sequences = vec![first, second];
        session.prefs_path = Some(data.join("preferences.json"));
        session.execute("timeline.place", json!({"item": item.0, "track": "V1", "time": 0, "insert": false})).unwrap();
        let clip = session.active_sequence().unwrap().video_tracks[0].items[0].id;
        session.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let snapshots = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").map(PathBuf::from);
        let mut builder = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000);
        if snapshots.is_some() {
            builder = builder.wgpu();
        }
        let harness = builder.build_eframe(move |_cc| app);
        let mut driver = Self { harness, tx, snapshots };
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

    fn layout(&mut self, layout: DockNode) {
        self.harness.state_mut().ui.dock = layout;
        self.harness.state_mut().ui.keys.maximized = None;
        self.frames(3);
    }

    fn point(&mut self, id: &str, fraction: f32) -> egui::Pos2 {
        let elements = self.ok("ui.elements", json!({"prefix": id}));
        let element = elements.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("missing {id}: {elements}"));
        let rect = element["rect"].as_array().unwrap();
        egui::pos2(
            (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap() * f64::from(fraction)) as f32,
            (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap() / 2.0) as f32,
        )
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }

    fn menu(&mut self, panel: &str) {
        self.ok("ui.click", json!({"id": format!("panel.tab.{panel}"), "button": "right"}));
        self.frames(3);
    }

    fn event(&mut self, event: egui::Event) {
        self.harness.input_mut().events.push(event);
        self.frames(1);
    }

    fn begin_drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
        self.event(egui::Event::PointerMoved(from));
        self.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        for step in 1..=8 {
            self.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
        }
    }

    fn release(&mut self, at: egui::Pos2) {
        self.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        self.frames(3);
    }

    fn drag(&mut self, panel: &str, target: &str, fraction: f32) {
        let from = self.point(&format!("panel.tab.{panel}"), 0.3);
        let to = self.point(&format!("panel.tab.{target}"), fraction);
        self.begin_drag(from, to);
        self.release(to);
    }

    fn snapshot(&mut self, name: &str) {
        let Some(dir) = self.snapshots.clone() else { return };
        self.frames(3);
        let image = self.harness.render().expect("render dock screenshot");
        std::fs::create_dir_all(&dir).unwrap();
        image.save(dir.join(format!("dock-{name}.png"))).unwrap();
    }
}

fn tabs(panels: &[PanelKind], active: usize) -> DockNode {
    DockNode::Tabs { panels: panels.to_vec(), active }
}

fn split(left: DockNode, right: DockNode) -> DockNode {
    DockNode::Split { vertical: false, size: SplitSize::Ratio(0.37), a: Box::new(left), b: Box::new(right) }
}

fn data_dir(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/dock-ui").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn effects_move_to_program_group_and_same_group_reorders_on_drop() {
    use PanelKind::*;
    let data = data_dir("move");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[Effects, Info], 1), tabs(&[Program], 0)));
    let original = d.harness.state().ui.dock.clone();
    let from = d.point("panel.tab.Effects", 0.3);
    let to = d.point("panel.tab.Program", 0.9);
    d.begin_drag(from, to);
    assert_eq!(d.harness.state().ui.dock, original, "a drag does not edit the layout before release");
    d.snapshot("drop-indicator");
    d.release(to);
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[Info], 0), tabs(&[Program, Effects], 1)));
    assert_eq!(d.harness.state().ui.focused, Effects);
    d.drag("Effects", "Program", 0.1);
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[Info], 0), tabs(&[Effects, Program], 0)));
    d.drag("Effects", "Program", 0.9);
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[Info], 0), tabs(&[Program, Effects], 1)));
    d.snapshot("reordered");
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn escape_focus_loss_and_drop_outside_preserve_layout_and_active_tab() {
    use PanelKind::*;
    let data = data_dir("cancel");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[Effects, Info], 1), tabs(&[Program], 0)));
    let original = d.harness.state().ui.dock.clone();
    for cancel in ["escape", "focus", "outside"] {
        let from = d.point("panel.tab.Effects", 0.3);
        let to = if cancel == "outside" { egui::pos2(5.0, 5.0) } else { d.point("panel.tab.Program", 0.9) };
        d.begin_drag(from, to);
        if cancel == "escape" {
            d.event(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: Some(egui::Key::Escape),
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
            d.event(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: Some(egui::Key::Escape),
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            });
        } else if cancel == "focus" {
            d.harness.input_mut().focused = false;
            d.frames(1);
            d.harness.input_mut().focused = true;
            d.frames(1);
        }
        d.release(to);
        assert_eq!(d.harness.state().ui.dock, original, "{cancel} should cancel without activating or moving a tab");
    }
    // Cancelled drag state cannot cause a later click on a destination tab to move the source.
    d.click("panel.tab.Program");
    assert_eq!(d.harness.state().ui.dock, original);
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn releasing_a_drag_while_another_mode_hides_the_dock_cannot_move_a_panel_on_return() {
    use PanelKind::*;
    let data = data_dir("hidden");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[Effects, Info], 1), tabs(&[Program], 0)));
    let original = d.harness.state().ui.dock.clone();
    let from = d.point("panel.tab.Effects", 0.3);
    let to = d.point("panel.tab.Program", 0.9);
    d.begin_drag(from, to);
    d.ok("ui.set", json!({"mode": "import"}));
    d.release(to);
    d.ok("ui.set", json!({"mode": "edit"}));
    assert_eq!(d.harness.state().ui.dock, original);
    d.click("panel.tab.Program");
    assert_eq!(d.harness.state().ui.dock, original);
    // A fresh drag still works after the cancelled hidden gesture.
    d.drag("Effects", "Program", 0.9);
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[Info], 0), tabs(&[Program, Effects], 1)));
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn moving_last_panel_collapses_source_and_saved_layout_reopens_with_timeline_tabs() {
    use PanelKind::*;
    let data = data_dir("saved");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[Effects], 0), split(tabs(&[Program], 0), tabs(&[Timeline, Info], 1))));
    let open = d.harness.state().session.state.open_sequences.clone();
    let active = d.harness.state().session.state.active_sequence;
    d.drag("Effects", "Program", 0.9);
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[Program, Effects], 1), tabs(&[Timeline, Info], 1)));
    d.drag("Timeline", "Effects", 0.1);
    let expected = split(tabs(&[Program, Timeline, Effects], 1), tabs(&[Info], 0));
    assert_eq!(d.harness.state().ui.dock, expected);
    assert_eq!(d.harness.state().session.state.open_sequences, open);
    assert_eq!(d.harness.state().session.state.active_sequence, active);
    for id in &open {
        let elements = d.ok("ui.elements", json!({"prefix": format!("timeline.tab.{}", id.0)}));
        assert!(elements.as_array().unwrap().iter().any(|e| e["id"] == format!("timeline.tab.{}", id.0)));
    }
    d.ok("ui.menu.invoke", json!({"id": "window.workspace.saveAs", "params": {"name": "Docked"}}));
    drop(d);
    let d = Driver::launch(&data);
    assert_eq!(d.harness.state().ui.workspace, "Docked");
    assert_eq!(d.harness.state().ui.dock, expected);
    assert_eq!(d.harness.state().session.state.open_sequences, open);
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn menu_maximize_restore_and_close_keep_the_unsaved_dock_layout() {
    use PanelKind::*;
    let data = data_dir("maximize");
    let mut d = Driver::launch(&data);
    let original = split(tabs(&[Effects, Info], 0), split(tabs(&[Program], 0), tabs(&[Timeline], 0)));
    d.layout(original.clone());
    d.menu("Effects");
    d.click("panel.menu.Effects.maximize");
    assert_eq!(d.harness.state().ui.keys.maximized, Some(Effects));
    assert_eq!(d.harness.state().ui.dock, original);
    d.snapshot("maximized");
    d.menu("Effects");
    d.click("panel.menu.Effects.maximize");
    assert_eq!(d.harness.state().ui.keys.maximized, None);
    assert_eq!(d.harness.state().ui.dock, original);
    d.menu("Effects");
    d.click("panel.menu.Effects.maximize");
    d.menu("Effects");
    d.click("panel.menu.Effects.restore");
    assert_eq!(d.harness.state().ui.keys.maximized, None);
    assert_eq!(d.harness.state().ui.dock, original, "Restore Workspace while maximized must retain unsaved split sizes and tabs");
    d.menu("Effects");
    d.click("panel.menu.Effects.maximize");
    d.menu("Effects");
    d.click("panel.menu.Effects.close");
    let mut expected = original;
    expected.close(Effects);
    assert_eq!(d.harness.state().ui.keys.maximized, None);
    assert_eq!(d.harness.state().ui.dock, expected);
    assert!(d.harness.state().ui.dock.contains(Timeline));
    d.snapshot("restored-after-close");
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn duplicate_panel_tab_activation_menu_close_and_maximized_close_affect_only_clicked_group() {
    use PanelKind::*;
    let data = data_dir("duplicates");
    let mut d = Driver::launch(&data);
    let original = split(tabs(&[EffectControls, Source], 1), tabs(&[EffectControls, EssentialGraphics], 1));
    d.layout(original.clone());
    d.click("panel.tab.EffectControls.group.b");
    let active_right = split(tabs(&[EffectControls, Source], 1), tabs(&[EffectControls, EssentialGraphics], 0));
    assert_eq!(d.harness.state().ui.dock, active_right);
    d.ok("ui.click", json!({"id": "panel.tab.EffectControls.group.b", "button": "right"}));
    d.frames(3);
    d.click("panel.menu.EffectControls.close");
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[EffectControls, Source], 1), tabs(&[EssentialGraphics], 0)));
    d.layout(original.clone());
    d.ok("ui.click", json!({"id": "panel.tab.EffectControls.group.b", "button": "middle"}));
    d.frames(3);
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[EffectControls, Source], 1), tabs(&[EssentialGraphics], 0)));
    d.layout(active_right.clone());
    d.ok("ui.click", json!({"id": "panel.tab.EffectControls.group.b", "button": "right"}));
    d.frames(3);
    d.click("panel.menu.EffectControls.maximize");
    assert_eq!(d.harness.state().ui.keys.maximized, Some(EffectControls));
    assert_eq!(d.harness.state().ui.dock, active_right);
    d.menu("EffectControls");
    d.click("panel.menu.EffectControls.close");
    assert_eq!(d.harness.state().ui.keys.maximized, None);
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[EffectControls, Source], 1), tabs(&[EssentialGraphics], 0)));
    // Both copies can be visible; activating their scoped tabs keeps their independent groups.
    d.layout(original);
    d.click("panel.tab.EffectControls.group.a");
    d.click("panel.tab.EffectControls.group.b");
    assert_eq!(d.harness.state().ui.dock, split(tabs(&[EffectControls, Source], 0), tabs(&[EffectControls, EssentialGraphics], 0)));
    let elements = d.ok("ui.elements", json!({"prefix": "panel.EffectControls"}));
    let bodies: Vec<_> = elements.as_array().unwrap().iter().filter(|e| e["id"] == "panel.EffectControls").collect();
    assert_eq!(bodies.len(), 2);
    assert!(bodies[0]["rect"][0].as_f64().unwrap() + bodies[0]["rect"][2].as_f64().unwrap() < bodies[1]["rect"][0].as_f64().unwrap());
    d.snapshot("duplicate-panel-instances");
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn closing_a_maximized_panel_with_a_stale_group_path_restores_without_removing_other_copies() {
    use PanelKind::*;
    let data = data_dir("stale-maximize");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[EffectControls, Source], 1), tabs(&[EffectControls, EssentialGraphics], 0)));
    d.ok("ui.click", json!({"id": "panel.tab.EffectControls.group.b", "button": "right"}));
    d.frames(3);
    d.click("panel.menu.EffectControls.maximize");
    let changed = tabs(&[EffectControls, Program], 0);
    d.harness.state_mut().ui.dock = changed.clone();
    d.frames(3);
    d.menu("EffectControls");
    d.click("panel.menu.EffectControls.close");
    assert_eq!(d.harness.state().ui.keys.maximized, None);
    assert_eq!(d.harness.state().ui.dock, changed);
    assert!(d.harness.state().ui.status.contains("not closed"));
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

fn matching_rects(d: &mut Driver, id: &str) -> Vec<[f32; 4]> {
    let elements = d.ok("ui.elements", json!({"prefix": id}));
    let mut rects: Vec<_> = elements
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["id"] == id)
        .map(|e| {
            let r = e["rect"].as_array().unwrap();
            [r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32, r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32]
        })
        .collect();
    rects.sort_by(|a, b| a[0].total_cmp(&b[0]));
    rects
}

fn motion_scale(d: &Driver) -> f64 {
    let item = &d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0];
    item.effects.iter().find(|e| e.effect == "motion").unwrap().params["scale"].value.as_f64().unwrap()
}

#[test]
fn two_visible_effect_controls_edit_only_the_clicked_number_and_enter_commits_once() {
    use PanelKind::*;
    let data = data_dir("duplicate-numbers");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[EffectControls], 0), tabs(&[EffectControls], 0)));
    let original = motion_scale(&d);
    for (side, text, expected) in [(0, "125", 125.0), (1, "150", 150.0)] {
        let fields = matching_rects(&mut d, "effectControls.motion.scale.value");
        assert_eq!(fields.len(), 2, "both copies display the same clip parameter");
        let field = fields[side];
        let history = d.harness.state().session.history.undo.len();
        let project = d.harness.state().session.project.clone();
        let jobs = d.harness.state().session.jobs.len();
        d.ok("ui.click", json!({"x": field[0] + field[2] / 2.0, "y": field[1] + field[3] / 2.0}));
        d.frames(3);
        let editing = matching_rects(&mut d, "effectControls.motion.scale.value.editing");
        let focused = matching_rects(&mut d, "effectControls.motion.scale.value.focused");
        assert_eq!(editing.len(), 1, "clicking one panel must not open a text editor in its counterpart");
        assert_eq!(focused.len(), 1, "exactly that text editor owns focus");
        assert_eq!(editing, focused);
        assert!((editing[0][0] - field[0]).abs() < 0.1, "the focused editor belongs to the clicked panel");
        assert!(d.harness.ctx.memory(|m| m.focused()).is_some());
        d.ok("ui.key", json!({"key": "Cmd+A"}));
        d.ok("ui.type", json!({"text": text}));
        assert!(Arc::ptr_eq(&project, &d.harness.state().session.project), "typing edits a draft");
        d.ok("ui.key", json!({"key": "Enter"}));
        d.frames(3);
        assert_eq!(motion_scale(&d), expected);
        assert_eq!(d.harness.state().session.history.undo.len(), history + 1, "Enter commits one parameter edit");
        assert_eq!(d.harness.state().session.jobs.len(), jobs, "Enter cannot render previews behind the field");
        assert!(matching_rects(&mut d, "effectControls.motion.scale.value.editing").is_empty());
        assert!(matching_rects(&mut d, "effectControls.motion.scale.value.focused").is_empty());
        d.ok("engine.execute", json!({"command": "edit.undo"}));
        assert_eq!(motion_scale(&d), original);
    }
    d.snapshot("duplicate-number-focus");
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn two_visible_effect_controls_buttons_edit_the_shared_clip_only_once() {
    use PanelKind::*;
    let data = data_dir("duplicate-buttons");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[EffectControls], 0), tabs(&[EffectControls], 0)));
    for side in [0, 1] {
        let headers = matching_rects(&mut d, "effectControls.effect.motion");
        assert_eq!(headers.len(), 2);
        let header = headers[side];
        let history = d.harness.state().session.history.undo.len();
        d.ok("ui.click", json!({"x": header[0] + 28.0, "y": header[1] + header[3] / 2.0}));
        d.frames(3);
        let item = &d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0];
        assert!(!item.effects.iter().find(|e| e.effect == "motion").unwrap().enabled);
        assert_eq!(d.harness.state().session.history.undo.len(), history + 1);
        d.ok("engine.execute", json!({"command": "edit.undo"}));
        let watches = matching_rects(&mut d, "effectControls.motion.scale.stopwatch");
        assert_eq!(watches.len(), 2);
        let watch = watches[side];
        d.ok("ui.click", json!({"x": watch[0] + watch[2] / 2.0, "y": watch[1] + watch[3] / 2.0}));
        d.frames(3);
        let item = &d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0];
        let param = &item.effects.iter().find(|e| e.effect == "motion").unwrap().params["scale"];
        assert!(param.is_animated());
        assert_eq!(param.keyframes.len(), 1);
        assert_eq!(d.harness.state().session.history.undo.len(), history + 1, "one click must not toggle twice across copies");
        d.ok("engine.execute", json!({"command": "edit.undo"}));
    }
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn two_visible_effect_controls_mask_buttons_add_once_and_share_collapse_state() {
    use PanelKind::*;
    let data = data_dir("duplicate-masks");
    let mut d = Driver::launch(&data);
    d.layout(split(tabs(&[EffectControls], 0), tabs(&[EffectControls], 0)));
    for side in [0, 1] {
        let buttons = matching_rects(&mut d, "effectControls.opacity.mask.ellipse");
        assert_eq!(buttons.len(), 2);
        let button = buttons[side];
        let history = d.harness.state().session.history.undo.len();
        d.ok("ui.click", json!({"x": button[0] + button[2] / 2.0, "y": button[1] + button[3] / 2.0}));
        d.frames(3);
        let item = &d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0];
        let index = item.effects.iter().position(|e| e.effect == "opacity").unwrap();
        assert_eq!(item.effects[index].masks.len(), 1, "one copy must add exactly one mask");
        assert_eq!(d.harness.state().session.history.undo.len(), history + 1);
        let collapse_key = format!("mask:{}:{index}:0", item.id.0);
        let headers = matching_rects(&mut d, "effectControls.opacity.mask0");
        assert_eq!(headers.len(), 2);
        let header = headers[side];
        d.ok("ui.click", json!({"x": header[0] + 26.0, "y": header[1] + header[3] / 2.0}));
        d.frames(3);
        assert!(d.harness.state().ui.collapsed_fx.contains(&collapse_key), "one scoped twirl click collapses both intentionally shared views");
        assert_eq!(d.harness.state().session.history.undo.len(), history + 1, "collapse is UI state");
        let headers = matching_rects(&mut d, "effectControls.opacity.mask0");
        let other = headers[1 - side];
        d.ok("ui.click", json!({"x": other[0] + 26.0, "y": other[1] + other[3] / 2.0}));
        d.frames(3);
        assert!(!d.harness.state().ui.collapsed_fx.contains(&collapse_key));
        d.ok("engine.execute", json!({"command": "edit.undo"}));
        let item = &d.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0];
        assert!(item.effects[index].masks.is_empty());
    }
    drop(d);
    std::fs::remove_dir_all(data).unwrap();
}
