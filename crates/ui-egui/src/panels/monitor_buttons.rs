//! Monitor Button Editor: a transient draft of each monitor's existing transport buttons.

use std::sync::{Arc, Weak};

use egui::{Align2, CornerRadius, Frame, Margin, Rect, Sense, Stroke, pos2, vec2};
use filmcraft_engine::media_pool::MediaPool;
use filmcraft_project::ItemId;
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::dock::PanelKind;
use crate::icons::{self, Icon};
use crate::panels::monitor::Which;
use crate::state::{Mode, MonitorView};
use crate::theme::Tokens;

#[derive(Clone, Copy)]
pub(crate) struct Button {
    pub id: &'static str,
    pub icon: Icon,
    pub label: &'static str,
}

const SOURCE: [Button; 12] = [
    Button { icon: Icon::Marker, id: "markers.add", label: "Add Marker (M)" },
    Button { icon: Icon::MarkIn, id: "src.markIn", label: "Mark In (I)" },
    Button { icon: Icon::MarkOut, id: "src.markOut", label: "Mark Out (O)" },
    Button { icon: Icon::GoToIn, id: "src.goIn", label: "Go to In (Shift+I)" },
    Button { icon: Icon::StepBack, id: "src.stepBack", label: "Step Back 1 Frame (Left)" },
    Button { icon: Icon::Play, id: "src.play", label: "Play-Stop Toggle (Space)" },
    Button { icon: Icon::StepFwd, id: "src.stepFwd", label: "Step Forward 1 Frame (Right)" },
    Button { icon: Icon::GoToOut, id: "src.goOut", label: "Go to Out (Shift+O)" },
    Button { icon: Icon::Insert, id: "source.insert", label: "Insert (,)" },
    Button { icon: Icon::Overwrite, id: "source.overwrite", label: "Overwrite (.)" },
    Button { icon: Icon::Camera, id: "exportFrame", label: "Export Frame" },
    Button { icon: Icon::Proxy, id: "media.toggleProxies", label: "Toggle Proxies" },
];
const PROGRAM: [Button; 13] = [
    Button { icon: Icon::Marker, id: "markers.add", label: "Add Marker (M)" },
    Button { icon: Icon::MarkIn, id: "markers.markIn", label: "Mark In (I)" },
    Button { icon: Icon::MarkOut, id: "markers.markOut", label: "Mark Out (O)" },
    Button { icon: Icon::GoToIn, id: "markers.goToIn", label: "Go to In (Shift+I)" },
    Button { icon: Icon::StepBack, id: "playhead.stepBack", label: "Step Back 1 Frame (Left)" },
    Button { icon: Icon::Play, id: "playback.toggle", label: "Play-Stop Toggle (Space)" },
    Button { icon: Icon::StepFwd, id: "playhead.stepForward", label: "Step Forward 1 Frame (Right)" },
    Button { icon: Icon::GoToOut, id: "markers.goToOut", label: "Go to Out (Shift+O)" },
    Button { icon: Icon::Lift, id: "sequence.lift", label: "Lift (;)" },
    Button { icon: Icon::Extract, id: "sequence.extract", label: "Extract (')" },
    Button { icon: Icon::Camera, id: "exportFrame", label: "Export Frame" },
    Button { icon: Icon::Proxy, id: "media.toggleProxies", label: "Toggle Proxies" },
    Button { icon: Icon::Fullscreen, id: "view.programFullScreen", label: "Program Monitor Full Screen (F11)" },
];

fn catalog(panel: PanelKind) -> &'static [Button] {
    match panel {
        PanelKind::Source => &SOURCE,
        PanelKind::Program => &PROGRAM,
        _ => &[],
    }
}
fn view(app: &FilmcraftApp, panel: PanelKind) -> &MonitorView {
    if panel == PanelKind::Source { &app.ui.source } else { &app.ui.program }
}
fn prefix(panel: PanelKind) -> &'static str {
    if panel == PanelKind::Source { "source" } else { "program" }
}
fn defaults(panel: PanelKind) -> Vec<String> {
    catalog(panel).iter().map(|button| button.id.into()).collect()
}

/// Saved UI settings are untrusted. Only a bounded, unique subset of this monitor's catalog is
/// rendered; None retains the exact existing default order, and an empty custom list retains +.
pub(crate) fn buttons(app: &FilmcraftApp, which: Which) -> Vec<Button> {
    let panel = if which == Which::Source { PanelKind::Source } else { PanelKind::Program };
    let all = catalog(panel);
    let Some(saved) = &view(app, panel).toolbar else { return all.to_vec() };
    let mut chosen: Vec<Button> = Vec::with_capacity(all.len());
    for id in saved.iter().take(all.len()) {
        if let Some(button) = all.iter().find(|button| button.id == id)
            && !chosen.iter().any(|previous| previous.id == button.id)
        {
            chosen.push(*button);
        }
    }
    chosen
}
fn current(app: &FilmcraftApp, panel: PanelKind) -> Vec<String> {
    buttons(app, if panel == PanelKind::Source { Which::Source } else { Which::Program }).iter().map(|button| button.id.into()).collect()
}

#[derive(Clone)]
pub struct EditorDraft {
    panel: PanelKind,
    item: Option<ItemId>,
    pool: Weak<MediaPool>,
    buttons: Vec<String>,
}
fn visible(app: &FilmcraftApp, panel: PanelKind) -> bool {
    if app.ui.mode != Mode::Edit || app.ui.program_fullscreen.is_some() {
        return false;
    }
    let shown = match app.ui.keys.maximized.filter(|p| app.ui.dock.contains(*p)) {
        Some(maximized) => maximized == panel,
        None => app.ui.dock.is_visible(panel),
    };
    shown
        && match panel {
            PanelKind::Source => app.session.state.source_item.is_some_and(|item| filmcraft_engine::clip_ops::source_view(&app.session, item).is_some()),
            PanelKind::Program => app.session.active_sequence().is_some(),
            _ => false,
        }
}
fn target_item(app: &FilmcraftApp, panel: PanelKind) -> Option<ItemId> {
    if panel == PanelKind::Source { app.session.state.source_item } else { app.session.state.active_sequence }
}
fn other_dialog(app: &FilmcraftApp) -> bool {
    app.dialog.is_some()
        || app.ui_error.is_some()
        || app.ui.clip_dialog.is_some()
        || app.ui.settings.is_some()
        || app.ui.color_dialog.is_some()
        || app.ui.save_preset.is_some()
        || app.ui.sync_dialog.is_some()
        || app.ui.edit_cameras.is_some()
        || app.ui.link_media.is_some()
        || app.ui.create_proxies.is_some()
        || app.ui.project_manager.is_some()
        || app.ui.make_offline.is_some()
        || app.ui.text_props_dialog.is_some()
        || app.ui.guide_dialog.is_some()
        || app.ui.workspace_dialog.is_some()
        || app.ui.extras.dialog.is_some()
        || app.ui.gfx_templates.export.is_some()
        || app.ui.gfx_templates.replace.is_some()
        || app.ui.project_panel.dialog.is_some()
        || app.ui.media_browser.edit_columns.is_some()
        || app.ui.export.manager.is_some()
        || app.ui.export.quick_open
        || !app.ui.audio_fx_editors.is_empty()
        || app.picker_pending()
}
fn context_dialog(ctx: &egui::Context) -> bool {
    crate::panels::remix::is_open(ctx) || crate::panels::interchange_export::is_open(ctx)
}
fn live(app: &FilmcraftApp, draft: &EditorDraft) -> bool {
    visible(app, draft.panel)
        && !other_dialog(app)
        && target_item(app, draft.panel) == draft.item
        && draft.pool.upgrade().is_some_and(|pool| Arc::ptr_eq(&pool, &app.session.media))
}
/// Close drafts before keyboard handling, including a project replacement with reused item IDs.
pub(crate) fn clear_stale(app: &mut FilmcraftApp) {
    if app.monitor_button_editor.as_ref().is_some_and(|draft| !live(app, draft)) {
        app.monitor_button_editor = None;
    }
}
pub(crate) fn can_open(app: &FilmcraftApp) -> bool {
    let panel = if app.ui.focused == PanelKind::Source { PanelKind::Source } else { PanelKind::Program };
    visible(app, panel) && !other_dialog(app)
}
pub(crate) fn inspect(app: &FilmcraftApp) -> Value {
    app.monitor_button_editor
        .as_ref()
        .filter(|draft| live(app, draft))
        .map_or(Value::Null, |draft| json!({"monitor": prefix(draft.panel), "buttons": draft.buttons}))
}
fn validate(panel: PanelKind, buttons: &Value) -> Result<Vec<String>, String> {
    let buttons = buttons.as_array().ok_or("buttons must be an array of button IDs")?;
    let all = catalog(panel);
    if buttons.len() > all.len() {
        return Err(format!("{} supports at most {} buttons", panel.title(), all.len()));
    }
    let mut result = Vec::with_capacity(buttons.len());
    for value in buttons {
        let id = value.as_str().ok_or("each button ID must be a string")?;
        if !all.iter().any(|button| button.id == id) {
            return Err(format!("unknown {} button ID", prefix(panel)));
        }
        if result.iter().any(|previous| previous == id) {
            return Err(format!("duplicate button `{id}`"));
        }
        result.push(id.into());
    }
    Ok(result)
}

/// UI command: view.monitorButtonEditor {monitor?:source|program, action?:open|set|toggle|move|
/// reset|apply|cancel, buttons?:[id], button?:id, enabled?:bool, position?:nonnegative integer}.
pub fn route(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "view.monitorButtonEditor" {
        return None;
    }
    Some(edit(app, ctx, params))
}
fn edit(app: &mut FilmcraftApp, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    let params = params.as_object().ok_or("parameters must be an object")?;
    let action = params.get("action").map(|value| value.as_str().ok_or("action must be a string")).transpose()?.unwrap_or("open");
    let panel = match params.get("monitor") {
        Some(Value::String(value)) if value.eq_ignore_ascii_case("source") => PanelKind::Source,
        Some(Value::String(value)) if value.eq_ignore_ascii_case("program") => PanelKind::Program,
        Some(_) => return Err("monitor must be source or program".into()),
        None if action != "open" => app.monitor_button_editor.as_ref().map(|draft| draft.panel).unwrap_or(app.ui.focused),
        None if app.ui.focused == PanelKind::Source => PanelKind::Source,
        None => PanelKind::Program,
    };
    if !matches!(action, "open" | "set" | "toggle" | "move" | "reset" | "apply" | "cancel") {
        return Err("unknown Button Editor action".into());
    }
    let allowed: &[&str] = match action {
        "set" => &["monitor", "action", "buttons"],
        "toggle" => &["monitor", "action", "button", "enabled"],
        "move" => &["monitor", "action", "button", "position"],
        _ => &["monitor", "action"],
    };
    if params.len() > allowed.len() || params.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("unexpected Button Editor parameter".into());
    }
    if context_dialog(ctx) {
        app.monitor_button_editor = None;
        return if action == "cancel" { Ok(Value::Null) } else { Err("another dialog is open".into()) };
    }
    if action == "open" {
        if !visible(app, panel) || other_dialog(app) {
            return Err("the monitor must be visible with no other dialog open".into());
        }
        app.monitor_button_editor =
            Some(EditorDraft { panel, item: target_item(app, panel), pool: Arc::downgrade(&app.session.media), buttons: current(app, panel) });
        ctx.request_repaint();
        return Ok(inspect(app));
    }
    clear_stale(app);
    if action == "cancel" && app.monitor_button_editor.is_none() {
        return Ok(Value::Null);
    }
    let draft = app.monitor_button_editor.as_ref().ok_or("no Button Editor is open")?;
    if draft.panel != panel {
        return Err("the command monitor differs from the open Button Editor".into());
    }
    let mut next = draft.buttons.clone();
    match action {
        "set" => next = validate(panel, params.get("buttons").ok_or("set requires buttons")?)?,
        "toggle" | "move" => {
            let button = params.get("button").and_then(Value::as_str).ok_or("button must be a button ID")?;
            if !catalog(panel).iter().any(|entry| entry.id == button) {
                return Err(format!("unknown {} button ID", prefix(panel)));
            }
            let existing = next.iter().position(|id| id == button);
            if action == "toggle" {
                let enabled = params.get("enabled").and_then(Value::as_bool).ok_or("enabled must be a boolean")?;
                match (enabled, existing) {
                    (true, None) => next.push(button.into()),
                    (false, Some(index)) => {
                        next.remove(index);
                    }
                    _ => {}
                }
            } else {
                let from = existing.ok_or("only enabled buttons can be moved")?;
                let to = params
                    .get("position")
                    .and_then(Value::as_u64)
                    .and_then(|index| usize::try_from(index).ok())
                    .filter(|index| *index < next.len())
                    .ok_or("position must be inside the enabled button list")?;
                let id = next.remove(from);
                next.insert(to, id);
            }
        }
        "reset" => next = defaults(panel),
        "apply" => {
            let selected = validate(panel, &json!(next))?;
            let toolbar = (selected != defaults(panel)).then_some(selected);
            if panel == PanelKind::Source {
                app.ui.source.toolbar = toolbar
            } else {
                app.ui.program.toolbar = toolbar
            }
            app.monitor_button_editor = None;
            ctx.request_repaint();
            return Ok(json!({"monitor": prefix(panel), "buttons": current(app, panel)}));
        }
        "cancel" => {
            app.monitor_button_editor = None;
            ctx.request_repaint();
            return Ok(Value::Null);
        }
        _ => return Err("unknown Button Editor action".into()),
    }
    if let Some(draft) = app.monitor_button_editor.as_mut() {
        draft.buttons = next;
    }
    ctx.request_repaint();
    Ok(inspect(app))
}

/// Existing modal frame and controls keep mouse/keyboard input inside the draft. Closing the
/// modal (Escape, close icon or outside click) cancels, while Apply changes only its monitor.
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    clear_stale(app);
    if context_dialog(ctx) {
        app.monitor_button_editor = None;
    }
    let Some(draft) = app.monitor_button_editor.clone() else { return };
    let t = app.tokens;
    let pre = format!("{}.buttonEditor", prefix(draft.panel));
    let title = format!("{} Button Editor", draft.panel.title());
    let mut action = None;
    let viewport = ctx.content_rect();
    let width = (viewport.width() - 48.0).clamp(1.0, 470.0);
    // Keep the title, footer and modal margins visible while the bounded catalog scrolls.
    let rows_height = (viewport.height() - 144.0).max(1.0);
    let frame = Frame::new().fill(t.panel_bg).stroke(Stroke::new(1.0, t.separator)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(14));
    let response = egui::Modal::new(egui::Id::new("monitor-button-editor")).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        let (header, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
        ui.painter().text(header.left_center(), Align2::LEFT_CENTER, &title, Tokens::semibold(13.0), t.text);
        app.auto.add(&format!("{pre}.window"), header, &title);
        let close = Rect::from_min_size(pos2(header.max.x - 22.0, header.min.y + 3.0), vec2(22.0, 22.0));
        let r = ui.interact(close, egui::Id::new((&pre, "close")), Sense::click()).on_hover_text("Close");
        icons::paint(ui.painter(), close.shrink(5.0), Icon::Close, if r.hovered() { t.text } else { t.text_dim });
        app.auto.add(&format!("{pre}.close"), close, "Close");
        if r.clicked() {
            action = Some(json!({"action": "cancel"}));
        }
        ui.separator();
        let all = catalog(draft.panel);
        let ordered: Vec<_> = draft
            .buttons
            .iter()
            .filter_map(|id| all.iter().find(|button| button.id == id))
            .chain(all.iter().filter(|button| !draft.buttons.iter().any(|id| id == button.id)))
            .collect();
        let parent_clip = ui.clip_rect();
        let mut row_controls = Vec::with_capacity(all.len() * 3);
        let list = egui::ScrollArea::vertical().id_salt((&pre, "list")).max_height(rows_height).auto_shrink([false, true]).show(ui, |ui| {
            for button in ordered {
                let position = draft.buttons.iter().position(|id| id == button.id);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), r, button.icon, t.icon);
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    let controls_width =
                        ["Up", "Down"].iter().map(|text| ui.painter().layout_no_wrap((*text).into(), font.clone(), t.text).size().x).sum::<f32>()
                            + 4.0 * ui.spacing().button_padding.x
                            + ui.spacing().item_spacing.x;
                    let checkbox_width = (ui.available_width() - controls_width - ui.spacing().item_spacing.x).max(1.0);
                    let mut enabled = position.is_some();
                    let checkbox = ui
                        .allocate_ui_with_layout(vec2(checkbox_width, ui.spacing().interact_size.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                            ui.checkbox(&mut enabled, button.label)
                        })
                        .inner;
                    row_controls.push((format!("{pre}.{}.enabled", button.id), checkbox.rect, button.label));
                    if checkbox.changed() {
                        action = Some(json!({"action": "toggle", "button": button.id, "enabled": enabled}));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let down = ui.add_enabled(position.is_some_and(|index| index + 1 < draft.buttons.len()), egui::Button::new("Down"));
                        row_controls.push((format!("{pre}.{}.down", button.id), down.rect, "Move Down"));
                        if down.clicked()
                            && let Some(index) = position
                        {
                            action = Some(json!({"action": "move", "button": button.id, "position": index + 1}));
                        }
                        let up = ui.add_enabled(position.is_some_and(|index| index > 0), egui::Button::new("Up"));
                        row_controls.push((format!("{pre}.{}.up", button.id), up.rect, "Move Up"));
                        if up.clicked()
                            && let Some(index) = position
                        {
                            action = Some(json!({"action": "move", "button": button.id, "position": index - 1}));
                        }
                    });
                });
            }
        });
        let list_clip = list.inner_rect.intersect(parent_clip);
        app.auto.add(&format!("{pre}.list"), list_clip, "Buttons");
        for (id, rect, label) in row_controls {
            let clipped = rect.intersect(list_clip);
            if clipped.is_positive() {
                app.auto.add(&id, clipped, label);
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            let reset = ui.button("Reset");
            app.auto.add(&format!("{pre}.reset"), reset.rect, "Reset to Defaults");
            if reset.clicked() {
                action = Some(json!({"action": "reset"}));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let apply = ui.button("Apply");
                app.auto.add(&format!("{pre}.apply"), apply.rect, "Apply");
                if apply.clicked() {
                    action = Some(json!({"action": "apply"}));
                }
                let cancel = ui.button("Cancel");
                app.auto.add(&format!("{pre}.cancel"), cancel.rect, "Cancel");
                if cancel.clicked() {
                    action = Some(json!({"action": "cancel"}));
                }
            });
        });
    });
    app.auto.add(&format!("{pre}.bounds"), response.response.rect, &title);
    if response.should_close() {
        action = Some(json!({"action": "cancel"}));
    }
    if let Some(mut params) = action {
        params["monitor"] = json!(prefix(draft.panel));
        if let Some(Err(error)) = route(app, ctx, "view.monitorButtonEditor", &params) {
            app.ui.status = error;
        }
    }
}
