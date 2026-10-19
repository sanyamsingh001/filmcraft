//! Effect Controls: the selected clip's effects (fixed Motion/Opacity/Time Remapping first, as in
//! Premiere, then standard effects), generated from parameter schemas, with stopwatches and a
//! keyframe lane on the right. Also hosts the Lumetri Color panel body (same editor, grouped).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use filmcraft_project::{ClipId, EffectInstance, ParamKind, ParamValue, TrackItem};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::theme::Tokens;

const ROW_H: f32 = 22.0;

// A provisional keyframe offset belongs to one pool and continuously painted gesture.
// A release while this panel is hidden cannot be observed by its later Response.
#[derive(Clone)]
struct GraphDragPreview {
    offset: f32,
    last_frame: u64,
    media: std::sync::Weak<filmcraft_engine::MediaPool>,
}

fn graph_drag_preview(app: &FilmcraftApp, ui: &egui::Ui, id: egui::Id) -> Option<f32> {
    let frame = ui.ctx().cumulative_frame_nr();
    let pointer_active =
        ui.input(|i| i.focused && !i.key_pressed(egui::Key::Escape) && (i.pointer.primary_down() || i.pointer.button_released(egui::PointerButton::Primary)));
    let media = std::sync::Arc::downgrade(&app.session.media);
    ui.data_mut(|data| {
        let mut preview = data.get_temp::<GraphDragPreview>(id)?;
        if !pointer_active || !std::sync::Weak::ptr_eq(&preview.media, &media) || frame.checked_sub(preview.last_frame).is_none_or(|age| age > 1) {
            data.remove::<GraphDragPreview>(id);
            return None;
        }
        preview.last_frame = frame;
        let offset = preview.offset;
        data.insert_temp(id, preview);
        Some(offset)
    })
}

fn store_graph_drag_preview(app: &FilmcraftApp, ui: &egui::Ui, id: egui::Id, offset: f32) {
    let preview = GraphDragPreview { offset, last_frame: ui.ctx().cumulative_frame_nr(), media: std::sync::Arc::downgrade(&app.session.media) };
    ui.data_mut(|data| data.insert_temp(id, preview));
}

// The press frame can include movement before the button went down. Start from the press
// origin so that only gesture motion enters the preview, including same-frame movement.
fn graph_drag_offset(ui: &egui::Ui, response: &egui::Response, previous: egui::Vec2) -> egui::Vec2 {
    if response.drag_started() {
        response
            .interact_pointer_pos()
            .zip(ui.input(|i| i.pointer.press_origin()))
            .map_or_else(|| response.drag_delta(), |(position, origin)| position - origin)
    } else {
        previous + response.drag_delta()
    }
}

fn selected_clip(app: &FilmcraftApp) -> Option<(ClipId, TrackItem, filmcraft_project::TrackKind)> {
    let seq = app.session.active_sequence()?;
    let mut best = None;
    for c in &app.session.state.selection {
        if let Some((tid, it)) = seq.find_item(*c) {
            let kind = seq.track(tid)?.kind;
            if kind == filmcraft_project::TrackKind::Video {
                return Some((*c, it.clone(), kind));
            }
            best.get_or_insert((*c, it.clone(), kind));
        }
    }
    best
}

/// Effect Controls shows the selected picture and its selected linked sound together. Other
/// selected audio belongs to another clip, and selecting sound independently still shows sound.
fn selected_linked_audio(app: &FilmcraftApp, picture: &TrackItem) -> Option<(ClipId, TrackItem)> {
    let link = picture.link?;
    let seq = app.session.active_sequence()?;
    app.session.state.selection.iter().find_map(|c| {
        let (tid, it) = seq.find_item(*c)?;
        (it.link == Some(link) && seq.track(tid)?.kind == filmcraft_project::TrackKind::Audio).then(|| (*c, it.clone()))
    })
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it, kind)) = selected_clip(app) else {
        crate::dock::placeholder(ui, rect, &t, "(no clip selected)");
        return;
    };
    let Some(seq) = app.session.active_sequence().cloned() else {
        crate::dock::placeholder(ui, rect, &t, "(no sequences)");
        return;
    };
    let split = rect.min.x + (rect.width() * 0.58).max(260.0).min(rect.width() - 60.0);
    let head = Rect::from_min_size(rect.min + vec2(8.0, 4.0), vec2(split - rect.min.x - 12.0, 24.0));
    // Premiere: two pill tabs — "Source · clip" and "Sequence · clip" (active)
    let pill = |ui: &mut egui::Ui, r: Rect, text: &str, active: bool| {
        ui.painter().rect_filled(r, 4.0, if active { Color32::from_rgb(0x3a, 0x3a, 0x3a) } else { t.panel_bg });
        if !active {
            ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
        }
        let cp = ui.painter().with_clip_rect(r.shrink(2.0));
        cp.text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, text, Tokens::ui(11.5), if active { t.text } else { t.text_dim });
    };
    let half = (head.width() - 6.0) / 2.0;
    pill(ui, Rect::from_min_size(head.min, vec2(half, 24.0)), &format!("Source · {}", it.name), false);
    pill(ui, Rect::from_min_size(head.min + vec2(half + 6.0, 0.0), vec2(half, 24.0)), &format!("{} · {}", seq_name(app), it.name), true);
    // keyframe lane header: mini ruler over the clip's duration
    let lane = Rect::from_min_max(pos2(split + 4.0, rect.min.y + 4.0), pos2(rect.max.x - 6.0, rect.max.y - 26.0));
    ui.painter().rect_filled(lane, 0.0, t.tl_bg);
    let ph = app.session.playhead();
    let dur = it.duration.0.max(1) as f64;
    let lx = |tk: Tick| -> f32 { lane.min.x + (((tk - it.start).0 as f64 / dur) as f32).clamp(0.0, 1.0) * lane.width() };
    ui.painter().rect_filled(Rect::from_min_max(pos2(lane.min.x, lane.min.y + 2.0), pos2(lane.max.x, lane.min.y + 16.0)), 2.0, Color32::from_rgb(58, 58, 70));
    ui.painter().text(pos2(lane.min.x + 4.0, lane.min.y + 9.0), Align2::LEFT_CENTER, &it.name, Tokens::ui(10.0), t.text);
    let scrub = Rect::from_min_max(lane.min, pos2(lane.max.x, lane.min.y + 18.0));
    let sresp = ui.interact(scrub, ui.make_persistent_id(("ec-scrub", clip.0)), Sense::click_and_drag());
    app.auto.add("effectControls.lane", lane, "keyframe lane");
    if (sresp.dragged() || sresp.clicked())
        && let Some(pos) = sresp.interact_pointer_pos()
    {
        let f = ((pos.x - lane.min.x) / lane.width()).clamp(0.0, 1.0) as f64;
        let tk = it.start + Tick((f * it.duration.0 as f64) as i64);
        app.stop();
        app.session.set_playhead(tk);
    }
    let mut clips = vec![(clip, it.clone(), kind)];
    if kind == filmcraft_project::TrackKind::Video
        && let Some((audio, sound)) = selected_linked_audio(app, &it)
    {
        clips.push((audio, sound, filmcraft_project::TrackKind::Audio));
    }
    let primary = clip;
    let primary_start = it.start;
    let body = Rect::from_min_max(pos2(rect.min.x, head.max.y + 4.0), pos2(split, rect.max.y - 26.0));
    let mut actions: Vec<(String, Value)> = Vec::new();
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("ec-body"));
    bui.set_clip_rect(Rect::from_min_max(body.min, pos2(rect.max.x, body.max.y)));
    let heading = if kind == filmcraft_project::TrackKind::Video { "Video" } else { "Audio" };
    bui.painter().text(pos2(body.min.x + 8.0, body.min.y + 8.0), Align2::LEFT_CENTER, heading, Tokens::semibold(11.5), t.text_dim);
    bui.add_space(18.0);
    let scroll_out = egui::ScrollArea::vertical().id_salt("ec-scroll").auto_shrink([false, false]).show(&mut bui, |bui| {
        for (clip, it, _kind) in clips {
            if clip != primary {
                let (heading, _) = bui.allocate_exact_size(vec2(body.width(), 18.0), Sense::hover());
                bui.painter().text(pos2(heading.min.x + 8.0, heading.center().y), Align2::LEFT_CENTER, "Audio", Tokens::semibold(11.5), t.text_dim);
            }
            // A linked sound can have a different trim, speed and source offset. Its lane uses
            // its own sequence span while its controls evaluate its own source time.
            let clip_lane = if clip == primary {
                lane
            } else {
                let start_x = lane.min.x + (it.start.0.saturating_sub(primary_start.0) as f64 / dur) as f32 * lane.width();
                let end_x = lane.min.x + (it.end().0.saturating_sub(primary_start.0) as f64 / dur) as f32 * lane.width();
                Rect::from_min_max(pos2(start_x, lane.min.y), pos2(end_x, lane.max.y))
            };
            let lane = clip_lane;
            let timeline_now = ph.clamp(it.start, (it.end() - Tick(1)).max(it.start));
            // Fixed effects first, then standard effects, as for a clip selected independently.
            let mut order: Vec<usize> = (0..it.effects.len()).collect();
            order.sort_by_key(|i| !it.effects[*i].def().is_some_and(|d| d.intrinsic));
            for idx in order {
                let e = &it.effects[idx];
                let Some(def) = e.def() else { continue };
                let Some(timing) = ClipTimeMap::new(&it, e) else { continue };
                let Some(mt_now) = timing.media_at_timeline(timeline_now) else { continue };
                let key = format!("{}:{}", clip.0, idx);
                let open = !app.ui.collapsed_fx.contains(&key);
                let (r, resp) = bui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::click());
                if resp.hovered() {
                    bui.painter().rect_filled(r, 0.0, t.hover);
                }
                icons::paint(
                    bui.painter(),
                    Rect::from_center_size(pos2(r.min.x + 10.0, r.center().y), vec2(10.0, 10.0)),
                    if open { Icon::ChevronDown } else { Icon::ChevronRight },
                    t.text_dim,
                );
                // fx enable toggle
                let fxr = Rect::from_center_size(pos2(r.min.x + 28.0, r.center().y), vec2(18.0, 14.0));
                let fxresp = bui.interact(fxr, bui.make_persistent_id(("fxen", clip.0, idx)), Sense::click());
                bui.painter().text(fxr.center(), Align2::CENTER_CENTER, "fx", Tokens::semibold(10.5), if e.enabled { t.text } else { t.text_faint });
                if !e.enabled {
                    bui.painter().line_segment([fxr.left_bottom(), fxr.right_top()], Stroke::new(1.0, t.text_faint));
                }
                if fxresp.clicked() {
                    actions.push(("effects.toggleEnabled".into(), json!({"clip": clip.0, "index": idx})));
                }
                bui.painter().text(pos2(r.min.x + 42.0, r.center().y), Align2::LEFT_CENTER, def.name, Tokens::ui(12.0), t.text);
                // reset button
                let rr = Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(16.0, 16.0));
                let rresp = bui.interact(rr, bui.make_persistent_id(("fxreset", clip.0, idx)), Sense::click()).on_hover_text("Reset Effect");
                icons::paint(bui.painter(), rr.shrink(2.0), Icon::Reset, if rresp.hovered() { t.text } else { t.text_dim });
                if rresp.clicked() {
                    actions.push(("effects.reset".into(), json!({"clip": clip.0, "index": idx})));
                }
                app.auto.add(&format!("effectControls.effect.{}", e.effect), r, def.name);
                if resp.clicked() && !fxresp.clicked() && !rresp.clicked() {
                    if open {
                        app.ui.collapsed_fx.push(key.clone());
                    } else {
                        app.ui.collapsed_fx.retain(|k| *k != key);
                    }
                }
                let mut save_preset = false;
                let fx_id = e.effect.clone();
                resp.context_menu(|ui| {
                    let sp = ui.button("Save Preset…");
                    app.auto.add(&format!("effectControls.effect.{fx_id}.savePreset"), sp.rect, "Save Preset…");
                    if sp.clicked() {
                        save_preset = true;
                        ui.close();
                    }
                    if !def.intrinsic && ui.button("Clear").clicked() {
                        actions.push(("effects.remove".into(), json!({"clip": clip.0, "index": idx})));
                        ui.close();
                    }
                });
                if save_preset {
                    crate::panels::presets::open_save(app, clip.0, vec![idx], def.name);
                }
                if !open {
                    continue;
                }
                if crate::panels::audio_fx_editor::has_editor(&e.effect) {
                    custom_setup_row(app, bui, body, clip, idx, &e.effect);
                }
                for pd in &def.params {
                    param_row(app, bui, body, clip, idx, e, None, pd, mt_now, &mut actions, &lane, &lx, &it);
                    if app.ui.expanded_fx.contains(&graph_key(clip, idx, pd.id))
                        && let Some(param) = e.params.get(pd.id)
                        && param.is_animated()
                        && matches!(param.value, ParamValue::Float(_) | ParamValue::Vec2(_))
                    {
                        graph_rows(app, bui, body, clip, idx, None, pd, param, &lane, &it, &mut actions);
                    }
                }
                if crate::panels::masks::maskable(e) {
                    crate::panels::masks::effect_rows(app, bui, body, clip, idx, e, mt_now, &mut actions, &lane, &lx, &it);
                }
            }
        }
    });
    let _ = scroll_out;
    // playhead in lane
    let px = lx(ph);
    ui.painter().line_segment([pos2(px, lane.min.y), pos2(px, lane.max.y)], Stroke::new(1.0, t.playhead));
    // footer timecode
    let tc = filmcraft_time::format_time(ph, seq.settings.frame_rate, seq.settings.drop_frame, filmcraft_time::TimeDisplay::Timecode, 48000);
    ui.painter().text(pos2(rect.min.x + 10.0, rect.max.y - 13.0), Align2::LEFT_CENTER, tc, Tokens::mono(13.0), t.timecode);
    run(app, ui.ctx(), actions);
}

/// Premiere's "Custom Setup ▸ Edit…" row: opens the effect's Clip Fx Editor window.
fn custom_setup_row(app: &mut FilmcraftApp, ui: &mut egui::Ui, body: Rect, clip: ClipId, idx: usize, effect: &str) {
    let t = app.tokens;
    let (r, _) = ui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::hover());
    ui.painter().text(pos2(r.min.x + 42.0, r.center().y), Align2::LEFT_CENTER, "Custom Setup", Tokens::ui(12.0), t.text_dim);
    let br = Rect::from_min_size(pos2(r.min.x + 160.0, r.min.y + 2.0), vec2(60.0, ROW_H - 4.0));
    let resp = ui.interact(br, ui.make_persistent_id(("fx-custom-setup", clip.0, idx)), Sense::click());
    ui.painter().rect_filled(br, 3.0, if resp.hovered() { t.hover } else { t.field_bg });
    ui.painter().rect_stroke(br, 3.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    ui.painter().text(br.center(), Align2::CENTER_CENTER, "Edit…", Tokens::ui(11.5), t.text);
    app.auto.add(&format!("effectControls.effect.{effect}.edit"), br, "Edit…");
    if resp.clicked() {
        crate::panels::audio_fx_editor::open(app, crate::panels::audio_fx_editor::FxTarget::Clip { clip: clip.0, index: idx });
    }
}

fn seq_name(app: &FilmcraftApp) -> String {
    app.session.state.active_sequence.and_then(|s| app.session.project.item(s)).map(|i| i.name.clone()).unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn param_row(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    body: Rect,
    clip: ClipId,
    idx: usize,
    e: &EffectInstance,
    mask: Option<usize>,
    pd: &filmcraft_project::ParamDef,
    mt: Tick,
    actions: &mut Vec<(String, Value)>,
    lane: &Rect,
    lx: &dyn Fn(Tick) -> f32,
    it: &TrackItem,
) {
    let _ = lx;
    let t = app.tokens;
    let param = match mask {
        Some(k) => e.masks.get(k).and_then(|m| m.param(pd.id)),
        None => e.params.get(pd.id),
    };
    let Some(param) = param else { return };
    // mask parameters: automation ids / widget ids get a `m<k>.` prefix, commands a `mask` field
    let pkey: String = match mask {
        Some(k) => format!("mask{k}.{}", pd.id),
        None => pd.id.to_string(),
    };
    let pkey = pkey.as_str();
    let with_mask = |mut v: Value| -> Value {
        if let Some(k) = mask {
            v["mask"] = json!(k);
        }
        v
    };
    let (r, _) = ui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::hover());
    let mut x = r.min.x + 26.0;
    // twirl-down for the value/velocity graphs (animated scalar or point params)
    if param.is_animated() && matches!(param.value, ParamValue::Float(_) | ParamValue::Vec2(_)) {
        let key = graph_key(clip, idx, pkey);
        let open = app.ui.expanded_fx.contains(&key);
        let tw = Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(12.0, 12.0));
        let tresp = ui.interact(tw.expand(2.0), ui.make_persistent_id(("twirl", clip.0, idx, pkey)), Sense::click()).on_hover_text("Show graphs");
        icons::paint(ui.painter(), tw, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.text_dim);
        app.auto.add(&format!("effectControls.{}.{}.graphs", e.effect, pkey), tw, "Show graphs");
        if tresp.clicked() {
            if open {
                app.ui.expanded_fx.retain(|k| *k != key);
            } else {
                app.ui.expanded_fx.push(key);
            }
        }
    }
    if pd.animatable {
        let sw = Rect::from_center_size(pos2(x, r.center().y), vec2(14.0, 14.0));
        let resp = ui.interact(sw, ui.make_persistent_id(("sw", clip.0, idx, pkey)), Sense::click()).on_hover_text("Toggle animation");
        icons::paint(ui.painter(), sw, Icon::Stopwatch, if param.is_animated() { t.accent } else { t.text_dim });
        if resp.clicked() {
            actions.push(("effects.toggleAnimation".into(), with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id}))));
        }
        app.auto.add(&format!("effectControls.{}.{}.stopwatch", e.effect, pkey), sw, "Toggle animation");
    }
    x += 14.0;
    ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, pd.label, Tokens::ui(12.0), t.text);
    let vx = r.min.x + (r.width() * 0.5).max(150.0);
    let value = param.value_at(mt);
    let mut vui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("ec-value", clip.0, idx, pkey))
            .max_rect(Rect::from_min_max(pos2(vx, r.min.y + 1.0), pos2(r.max.x - 26.0, r.max.y - 1.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let id = ui.make_persistent_id(("pv", clip.0, idx, pkey));
    let mut set: Option<Value> = None;
    match (&pd.kind, &value) {
        (ParamKind::Float { min, max, soft_min, soft_max, unit, decimals }, ParamValue::Float(v)) => {
            let speed = ((soft_max - soft_min) / 400.0).max(0.01);
            let (response, nv) = crate::widgets::hot_number(&mut vui, id, *v, speed, (*min, *max), *decimals as usize, unit, &t);
            app.auto.add(&format!("effectControls.{}.{}.value", e.effect, pkey), response.rect, pd.label);
            if vui.data(|d| d.get_temp::<String>(id.with("editing")).is_some()) {
                app.auto.add(&format!("effectControls.{}.{}.value.editing", e.effect, pkey), response.rect, pd.label);
                if response.has_focus() {
                    app.auto.add(&format!("effectControls.{}.{}.value.focused", e.effect, pkey), response.rect, pd.label);
                }
            }
            if let Some(nv) = nv {
                set = Some(json!(nv));
            }
        }
        (ParamKind::Angle, ParamValue::Float(v)) => {
            let (_, nv) = crate::widgets::hot_number(&mut vui, id, *v, 0.5, (-36000.0, 36000.0), 1, "°", &t);
            if let Some(nv) = nv {
                set = Some(json!(nv));
            }
        }
        (ParamKind::Point, ParamValue::Vec2(p)) => {
            let (_, nx) = crate::widgets::hot_number(&mut vui, id.with("x"), p.x, 1.0, (-100_000.0, 100_000.0), 1, "", &t);
            let (_, ny) = crate::widgets::hot_number(&mut vui, id.with("y"), p.y, 1.0, (-100_000.0, 100_000.0), 1, "", &t);
            if nx.is_some() || ny.is_some() {
                set = Some(json!([nx.unwrap_or(p.x), ny.unwrap_or(p.y)]));
            }
        }
        (ParamKind::Bool, ParamValue::Bool(b)) => {
            let mut v = *b;
            if vui.checkbox(&mut v, "").changed() {
                set = Some(json!(v));
            }
        }
        (ParamKind::Choice(opts), ParamValue::Choice(c)) => {
            let mut sel = *c as usize;
            egui::ComboBox::from_id_salt(id).selected_text(opts.get(sel).copied().unwrap_or("")).width(130.0).show_ui(&mut vui, |ui| {
                for (i, o) in opts.iter().enumerate() {
                    if ui.selectable_value(&mut sel, i, *o).changed() {
                        set = Some(json!(i));
                    }
                }
            });
        }
        (ParamKind::Color, ParamValue::Color(c)) => {
            let mut rgba = egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            if egui::color_picker::color_edit_button_rgba(&mut vui, &mut rgba, egui::color_picker::Alpha::Opaque).changed() {
                set = Some(json!([rgba.r(), rgba.g(), rgba.b(), rgba.a()]));
            }
        }
        (ParamKind::Path, _) => {
            crate::panels::masks::path_value(app, &mut vui, clip, idx, mask, actions);
        }
        (ParamKind::Text, ParamValue::Text(s)) if !pd.id.ends_with("_lut") => {
            // edited in a buffer; committed (one undo step) when the field loses focus
            let mut buf = crate::widgets::session_text_draft(vui.ctx(), id).unwrap_or_else(|| s.clone());
            let r = vui.add(egui::TextEdit::singleline(&mut buf).desired_width(160.0));
            app.auto.add(&format!("effectControls.text.{}.{}", idx, pd.id), r.rect, pd.label);
            if r.lost_focus() {
                if buf != *s {
                    set = Some(json!(buf));
                }
                crate::widgets::remove_session_text_draft(vui.ctx(), id);
            } else if r.has_focus() {
                crate::widgets::store_session_text_draft(vui.ctx(), id, buf);
            } else {
                crate::widgets::remove_session_text_draft(vui.ctx(), id);
            }
        }
        _ => {
            vui.label(param_text(&app.session.project, pd.id, &value));
        }
    }
    if let Some(v) = set {
        actions.push(("effects.setParam".into(), with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": v}))));
    }
    // keyframe navigator ◀ ◆ ▶ (when animated)
    let eff_json = json!(idx);
    if pd.animatable && param.is_animated() {
        let nx = r.max.x - 58.0;
        let cy = r.center().y;
        let at_key = param.keyframes.iter().any(|k| k.time == mt);
        let timing = ClipTimeMap::new(it, e);
        // A shortened responsive graphic can skip design-time keys between its protected
        // ends. Visit each direction's finite key slice once, reusing the checked inverse, so
        // an unreachable immediate key does not hide a farther reachable neighbour.
        let to_tl = |ascending: bool| {
            let timing = timing.as_ref()?;
            if ascending {
                param.keyframes.iter().filter(|k| k.time > mt).find_map(|k| timing.timeline_at_media(k.time))
            } else {
                param.keyframes.iter().rev().filter(|k| k.time < mt).find_map(|k| timing.timeline_at_media(k.time))
            }
        };
        let reversed = timing.as_ref().is_some_and(ClipTimeMap::reversed);
        let prev = to_tl(reversed);
        let next = to_tl(!reversed);
        let pr = Rect::from_center_size(pos2(nx, cy), vec2(12.0, 14.0));
        let kr = Rect::from_center_size(pos2(nx + 16.0, cy), vec2(12.0, 12.0));
        let nr = Rect::from_center_size(pos2(nx + 32.0, cy), vec2(12.0, 14.0));
        let arrow = |p: &egui::Painter, r: Rect, left: bool, on: bool| {
            let c = r.center();
            let pts = if left {
                vec![c + vec2(3.0, -4.0), c + vec2(3.0, 4.0), c + vec2(-3.0, 0.0)]
            } else {
                vec![c + vec2(-3.0, -4.0), c + vec2(-3.0, 4.0), c + vec2(3.0, 0.0)]
            };
            p.add(egui::Shape::convex_polygon(pts, if on { t.text } else { t.text_faint }, Stroke::NONE));
        };
        arrow(ui.painter(), pr, true, prev.is_some());
        arrow(ui.painter(), nr, false, next.is_some());
        icons::paint(ui.painter(), kr, Icon::Keyframe, if at_key { t.hot_text } else { t.text_dim });
        if ui.interact(pr, ui.make_persistent_id(("kprev", clip.0, idx, pkey)), Sense::click()).clicked()
            && let Some(k) = prev
        {
            actions.push(("playhead.set".into(), json!({"time": k.0})));
        }
        if ui.interact(nr, ui.make_persistent_id(("knext", clip.0, idx, pkey)), Sense::click()).clicked()
            && let Some(k) = next
        {
            actions.push(("playhead.set".into(), json!({"time": k.0})));
        }
        if ui.interact(kr, ui.make_persistent_id(("kadd", clip.0, idx, pkey)), Sense::click()).on_hover_text("Add/Remove Keyframe").clicked() {
            actions.push(("effects.addKeyframe".into(), with_mask(json!({"clip": clip.0, "effect": eff_json, "param": pd.id}))));
        }
        app.auto.add(&format!("effectControls.{}.{}.prevKeyframe", e.effect, pkey), pr, "Previous Keyframe");
        app.auto.add(&format!("effectControls.{}.{}.nextKeyframe", e.effect, pkey), nr, "Next Keyframe");
        app.auto.add(&format!("effectControls.{}.{}.addKeyframe", e.effect, pkey), kr, "Add/Remove Keyframe");
    }
    // keyframes in the lane: draggable diamonds; right-click for interpolation
    if param.is_animated() {
        let y = r.center().y;
        let Some(timing) = ClipTimeMap::new(it, e) else { return };
        let rate = app.session.sequence_rate();
        for k in &param.keyframes {
            let Some(tl) = timing.timeline_at_media(k.time) else { continue };
            let Some(f) = timing.fraction_at_media(k.time) else { continue };
            if !(-0.01..=1.01).contains(&f) {
                continue;
            }
            let id = ui.make_persistent_id(("kf", clip.0, idx, pkey, k.time.0));
            let drag_off = graph_drag_preview(app, ui, id);
            let kx = lane.min.x + f * lane.width() + drag_off.unwrap_or(0.0);
            let kr = Rect::from_center_size(pos2(kx, y), vec2(11.0, 11.0));
            let resp = ui.interact(kr.expand(2.0), id, Sense::click_and_drag());
            app.auto.add(&format!("effectControls.{}.{}.keyframe.{}", e.effect, pkey, k.time.0), kr, "keyframe");
            let sel = k.time == mt || resp.dragged();
            let col = if sel { t.hot_text } else { Color32::from_rgb(0xb0, 0xb0, 0xb0) };
            match k.interp {
                filmcraft_project::Interpolation::Hold => {
                    ui.painter().rect_filled(Rect::from_center_size(kr.center(), vec2(8.0, 8.0)), 0.0, col);
                }
                filmcraft_project::Interpolation::Linear => icons::paint(ui.painter(), kr, Icon::Keyframe, col),
                _ => {
                    ui.painter().circle_filled(kr.center(), 4.5, col);
                }
            }
            if resp.dragged() && (resp.drag_started() || drag_off.is_some()) {
                let off = drag_off.unwrap_or(0.0) + resp.drag_delta().x;
                store_graph_drag_preview(app, ui, id, off);
            }
            if resp.drag_stopped()
                && let Some(off) = drag_off
            {
                ui.data_mut(|d| d.remove::<GraphDragPreview>(id));
                let Some(new_tl) = timing.timeline_at_fraction((f + off / lane.width()) as f64) else { continue };
                let new_tl = rate.snap_nearest(new_tl).clamp(it.start, it.end() - rate.frame_duration());
                let Some(new_media) = timing.media_at_timeline(new_tl) else { continue };
                if new_media != k.time {
                    actions.push((
                        "effects.moveKeyframe".into(),
                        with_mask(json!({"clip": clip.0, "effect": eff_json, "param": pd.id, "mediaTime": k.time.0, "to": new_media.0})),
                    ));
                }
            }
            if resp.clicked() {
                actions.push(("playhead.set".into(), json!({"time": tl.0})));
            }
            resp.context_menu(|ui| {
                ui.label(egui::RichText::new("Temporal Interpolation").color(t.text_dim));
                for (label, key) in [
                    ("Linear", "linear"),
                    ("Bezier", "bezier"),
                    ("Auto Bezier", "autoBezier"),
                    ("Continuous Bezier", "continuousBezier"),
                    ("Hold", "hold"),
                    ("Ease In", "easeIn"),
                    ("Ease Out", "easeOut"),
                ] {
                    if ui.button(label).clicked() {
                        actions.push((
                            "effects.setInterpolation".into(),
                            with_mask(json!({"clip": clip.0, "effect": eff_json, "param": pd.id, "mediaTime": k.time.0, "interpolation": key})),
                        ));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Clear").clicked() {
                    actions
                        .push(("effects.deleteKeyframe".into(), with_mask(json!({"clip": clip.0, "effect": eff_json, "param": pd.id, "mediaTime": k.time.0}))));
                    ui.close();
                }
            });
        }
    }
}

/// Lumetri Color panel: edits (or adds) the Lumetri effect on the selected clip, grouped by section.
pub fn lumetri_panel(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it, _)) = selected_clip(app) else {
        crate::dock::placeholder(ui, rect, &t, "Select a clip to grade");
        return;
    };
    let idx = it.effects.iter().position(|e| e.effect == "lumetri");
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)).id_salt("lumetri"));
    bui.label(egui::RichText::new(format!("Master · {}", it.name)).color(t.text_dim));
    let Some(idx) = idx else {
        if bui.button("Add Lumetri Color to clip").clicked() {
            let _ = app.session.execute("effects.apply", json!({"clips": [clip.0], "effect": "lumetri"}));
        }
        return;
    };
    let e = it.effects[idx].clone();
    let Some(def) = e.def() else { return };
    let ph = app.session.playhead();
    let mt = it.effect_time_at(ph.clamp(it.start, it.end() - Tick(1)));
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut bui, |ui| {
        let mut groups: Vec<&str> = Vec::new();
        for p in &def.params {
            if let Some(g) = p.group
                && !groups.contains(&g)
            {
                groups.push(g);
            }
        }
        for g in groups {
            let key = format!("lumetri:{g}");
            let open = !app.ui.collapsed_fx.contains(&key);
            let (resp, now_open) = crate::widgets::section_header(ui, egui::Id::new(&key), g, open, &t, true);
            app.auto.add(&format!("lumetri.section.{g}"), resp.rect, g);
            if now_open != open {
                if now_open {
                    app.ui.collapsed_fx.retain(|k| *k != key);
                } else {
                    app.ui.collapsed_fx.push(key.clone());
                }
            }
            if !now_open {
                continue;
            }
            for pd in def.params.iter().filter(|p| p.group == Some(g)) {
                let v = e.params.get(pd.id).map(|p| p.value_at(mt)).unwrap_or(pd.default.clone());
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    ui.add_sized(vec2(110.0, 18.0), egui::Label::new(egui::RichText::new(pd.label).size(12.0)));
                    if let (ParamKind::Float { min, max, soft_min, soft_max, .. }, ParamValue::Float(x)) = (&pd.kind, &v) {
                        let mut val = *x;
                        let s = ui.add(egui::Slider::new(&mut val, *soft_min..=*soft_max).show_value(false));
                        let (_, nv) =
                            crate::widgets::hot_number(ui, egui::Id::new(("lum", pd.id)), val, (soft_max - soft_min) / 300.0, (*min, *max), 1, "", &t);
                        if s.changed() {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": val}));
                        } else if let Some(nv) = nv {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": nv}));
                        }
                        if s.double_clicked() {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": pd.default.as_f64().unwrap_or(0.0)}));
                        }
                    } else if let ParamValue::Color(c) = v {
                        let mut rgba = egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                        if egui::color_picker::color_edit_button_rgba(ui, &mut rgba, egui::color_picker::Alpha::Opaque).changed() {
                            actions.push(json!({"clip": clip.0, "effect": idx, "param": pd.id, "value": [rgba.r(), rgba.g(), rgba.b(), 1.0]}));
                        }
                    }
                });
            }
        }
    });
    run(app, ui.ctx(), actions.into_iter().map(|a| ("effects.setParam".to_string(), a)).collect());
}

/// Properties panel (Premiere 26): a compact inspector for the selected clip.
pub fn properties_panel(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some((clip, it, kind)) = selected_clip(app) else {
        crate::dock::placeholder(ui, rect, &t, "Select a clip to see its properties");
        return;
    };
    let ph = app.session.playhead();
    let mt = it.effect_time_at(ph.clamp(it.start, it.end() - Tick(1)));
    let mut actions: Vec<(String, Value)> = Vec::new();
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 8.0))).id_salt("props"));
    // header: clip name + menu
    let (hr, _) = bui.allocate_exact_size(vec2(bui.available_width(), 30.0), Sense::hover());
    let sw = Rect::from_center_size(pos2(hr.min.x + 8.0, hr.center().y), vec2(12.0, 12.0));
    let lc = app.session.prefs.labels.rgb(it.label);
    bui.painter().rect_filled(sw, 2.0, Color32::from_rgb(lc[0], lc[1], lc[2]));
    bui.painter().text(pos2(hr.min.x + 22.0, hr.center().y), Align2::LEFT_CENTER, &it.name, Tokens::semibold(12.5), t.text);
    let row = |ui: &mut egui::Ui, label: &str| -> Rect {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        ui.painter().text(pos2(r.min.x + 18.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::ui(12.0), t.text_dim);
        r
    };
    let section = |ui: &mut egui::Ui, app: &mut FilmcraftApp, name: &str, reset: Option<(usize, u64)>, actions: &mut Vec<(String, Value)>| -> bool {
        ui.add_space(4.0);
        let key = format!("props:{name}");
        let open = !app.ui.collapsed_fx.contains(&key);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.min.x + 6.0, r.center().y), vec2(10.0, 10.0)),
            if open { Icon::ChevronDown } else { Icon::ChevronRight },
            t.text_dim,
        );
        ui.painter().text(pos2(r.min.x + 18.0, r.center().y), Align2::LEFT_CENTER, name, Tokens::semibold(13.0), t.text);
        if let Some((idx, c)) = reset {
            let rr = Rect::from_center_size(pos2(r.max.x - 10.0, r.center().y), vec2(14.0, 14.0));
            icons::paint(ui.painter(), rr, Icon::Reset, t.text_dim);
            if ui.interact(rr, egui::Id::new(("props-reset", name, c)), Sense::click()).clicked() {
                actions.push(("effects.reset".into(), json!({"clip": c, "index": idx})));
            }
        }
        if resp.clicked() {
            if open {
                app.ui.collapsed_fx.push(key);
            } else {
                app.ui.collapsed_fx.retain(|k| *k != key);
            }
        }
        open
    };
    let diamond = |ui: &mut egui::Ui, r: Rect, animated: bool, id: egui::Id| -> bool {
        let dr = Rect::from_center_size(pos2(r.max.x - 10.0, r.center().y), vec2(10.0, 10.0));
        let resp = ui.interact(dr.expand(3.0), id, Sense::click());
        if animated {
            icons::paint(ui.painter(), dr, Icon::Keyframe, t.hot_text);
        } else {
            let c = dr.center();
            let s = 4.5;
            ui.painter()
                .add(egui::Shape::closed_line(vec![c + vec2(0.0, -s), c + vec2(s, 0.0), c + vec2(0.0, s), c + vec2(-s, 0.0)], Stroke::new(1.2, t.text_dim)));
        }
        resp.clicked()
    };
    let eff_idx = |id: &str| it.effects.iter().position(|e| e.effect == id);
    let val = |eid: &str, p: &str| it.effect(eid).and_then(|e| e.param(p)).map(|p| p.value_at(mt));
    let anim = |eid: &str, p: &str| it.effect(eid).and_then(|e| e.param(p)).is_some_and(|p| p.is_animated());
    if kind == filmcraft_project::TrackKind::Video {
        if section(&mut bui, app, "Transform", eff_idx("motion").map(|i| (i, clip.0)), &mut actions) {
            for (label, p, unit, speed, range) in [
                ("Position", "position", "", 1.0, (-100_000.0, 100_000.0)),
                ("Anchor point", "anchor", "", 1.0, (-100_000.0, 100_000.0)),
                ("Scale", "scale", " %", 0.5, (0.0, 10000.0)),
                ("Rotation", "rotation", " °", 0.5, (-36000.0, 36000.0)),
            ] {
                let r = row(&mut bui, label);
                let mut vui = bui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                match val("motion", p) {
                    Some(ParamValue::Vec2(v)) => {
                        let (_, nx) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp", p, "x", clip.0)), v.x, speed, range, 0, " X", &t);
                        vui.add_space(10.0);
                        let (_, ny) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp", p, "y", clip.0)), v.y, speed, range, 0, " Y", &t);
                        if nx.is_some() || ny.is_some() {
                            actions.push((
                                "effects.setParam".into(),
                                json!({"clip": clip.0, "effect": "motion", "param": p, "value": [nx.unwrap_or(v.x), ny.unwrap_or(v.y)]}),
                            ));
                        }
                    }
                    Some(ParamValue::Float(v)) => {
                        let (_, nv) =
                            crate::widgets::hot_number(&mut vui, egui::Id::new(("pp", p, clip.0)), v, speed, range, if p == "scale" { 0 } else { 1 }, unit, &t);
                        if let Some(nv) = nv {
                            actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "motion", "param": p, "value": nv})));
                        }
                    }
                    _ => {}
                }
                if diamond(&mut bui, r, anim("motion", p), egui::Id::new(("pd", p, clip.0))) {
                    actions.push(("effects.toggleAnimation".into(), json!({"clip": clip.0, "effect": "motion", "param": p})));
                }
            }
            let r = row(&mut bui, "Opacity");
            let mut vui = bui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            if let Some(ParamValue::Float(v)) = val("opacity", "opacity") {
                let (_, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-op", clip.0)), v, 0.5, (0.0, 100.0), 0, " %", &t);
                if let Some(nv) = nv {
                    actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "opacity", "param": "opacity", "value": nv})));
                }
            }
            if diamond(&mut bui, r, anim("opacity", "opacity"), egui::Id::new(("pd-op", clip.0))) {
                actions.push(("effects.toggleAnimation".into(), json!({"clip": clip.0, "effect": "opacity", "param": "opacity"})));
            }
        }
        let crop = eff_idx("crop");
        if section(&mut bui, app, "Crop", crop.map(|i| (i, clip.0)), &mut actions) {
            for (label, p) in [("Left", "left"), ("Top", "top"), ("Right", "right"), ("Bottom", "bottom")] {
                let r = row(&mut bui, label);
                let mut vui = bui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                let v = val("crop", p).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let (_, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-crop", p, clip.0)), v, 0.2, (0.0, 100.0), 1, " %", &t);
                if let Some(nv) = nv {
                    if crop.is_none() {
                        actions.push(("effects.apply".into(), json!({"clips": [clip.0], "effect": "crop"})));
                    }
                    actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "crop", "param": p, "value": nv})));
                }
                diamond(&mut bui, r, anim("crop", p), egui::Id::new(("pd-crop", p, clip.0)));
            }
        }
    } else if section(&mut bui, app, "Audio", eff_idx("volume").map(|i| (i, clip.0)), &mut actions) {
        let r = row(&mut bui, "Level");
        let mut vui = bui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let v = val("volume", "level").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let (_, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-vol", clip.0)), v, 0.1, (-96.0, 15.0), 1, " dB", &t);
        if let Some(nv) = nv {
            actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "volume", "param": "level", "value": nv})));
        }
        if diamond(&mut bui, r, anim("volume", "level"), egui::Id::new(("pd-vol", clip.0))) {
            actions.push(("effects.toggleAnimation".into(), json!({"clip": clip.0, "effect": "volume", "param": "level"})));
        }
        let r = row(&mut bui, "Pan");
        let mut vui = bui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(r.min.x + 150.0, r.min.y + 4.0), pos2(r.max.x - 24.0, r.max.y - 4.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let v = val("panner", "balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let (_, nv) = crate::widgets::hot_number(&mut vui, egui::Id::new(("pp-pan", clip.0)), v, 0.5, (-100.0, 100.0), 0, "", &t);
        if let Some(nv) = nv {
            actions.push(("effects.setParam".into(), json!({"clip": clip.0, "effect": "panner", "param": "balance", "value": nv})));
        }
    }
    // speed footer
    bui.add_space(12.0);
    let (br, bresp) = bui.allocate_exact_size(vec2(118.0, 26.0), Sense::click());
    bui.painter().rect_filled(br, 4.0, if bresp.hovered() { t.hover } else { t.panel_bg });
    bui.painter().rect_stroke(br, 4.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
    bui.painter().text(br.center(), Align2::CENTER_CENTER, format!("Speed {:.0}%", it.speed * 100.0), Tokens::ui(12.0), t.text);
    run(app, ui.ctx(), actions);
}

/// Run the panel's actions. Parameter changes made while the mouse button is down (a drag) share
/// one undo step, which the first change of each press begins (#201); typed values and clicks stay
/// separate steps. A drag value only changes once the mouse moves, after the press frame, so the
/// press that already began a step is remembered by its start time.
fn run(app: &mut FilmcraftApp, ctx: &egui::Context, actions: Vec<(String, Value)>) {
    let (down, press) = ctx.input(|i| (i.pointer.any_down(), i.pointer.press_start_time()));
    let key = egui::Id::new("effect-controls-drag-step");
    for (cmd, mut p) in actions {
        if cmd == "effects.setParam" && down {
            let begun = ctx.data(|d| d.get_temp::<Option<f64>>(key)).flatten();
            p["merge"] = json!(true);
            p["begin"] = json!(begun != press);
            ctx.data_mut(|d| d.insert_temp(key, press));
        }
        if let Err(e) = app.session.execute(&cmd, p) {
            app.ui.status = e.to_string();
        }
    }
}

pub(crate) fn graph_key(clip: ClipId, idx: usize, pid: &str) -> String {
    format!("graph:{}:{}:{}", clip.0, idx, pid)
}

/// Every lane uses sequence time, while each effect has its own parameter clock. Graphic
/// layers also apply Responsive Design – Time; intrinsic Motion and ordinary effects keep the
/// raw media clock. Invert the renderer's actual mapping, including reverse and held frames.
struct ClipTimeMap<'a> {
    item: &'a TrackItem,
    effect: &'a EffectInstance,
    end: Tick,
    raw_start: Tick,
    raw_end: Tick,
    responsive: bool,
}

impl<'a> ClipTimeMap<'a> {
    fn new(item: &'a TrackItem, effect: &'a EffectInstance) -> Option<Self> {
        if item.duration.0 <= 0 || !item.speed.is_finite() || item.speed == 0.0 {
            return None;
        }
        let end = Tick(item.start.0.checked_add(item.duration.0)?);
        let consumed = (item.duration.0 as f64 * item.speed.abs()).round();
        if !consumed.is_finite() || consumed >= i64::MAX as f64 {
            return None;
        }
        // TrackItem's mapping uses these sums. Validate them before calling it on project data.
        item.source_in.0.checked_add(consumed as i64)?;
        if item.reverse {
            item.source_in.0.checked_sub(1)?;
        }
        let responsive = filmcraft_project::graphic::is_layer(effect) && item.graphic.as_ref().is_some_and(|m| m.has_responsive_time());
        if responsive {
            let m = item.graphic.as_ref()?;
            // The canonical remap is monotone for valid protected regions. Malformed project
            // metadata has no reliable inverse, so leave its keyframe gestures unavailable.
            if i128::from(m.intro.0.max(0)) + i128::from(m.outro.0.max(0)) > i128::from(m.design_duration.0) {
                return None;
            }
        }
        let mapping = Self { item, effect, end, raw_start: item.effect_time_at(item.start), raw_end: item.effect_time_at(end), responsive };
        mapping.parameter_at_raw(mapping.raw_start)?;
        mapping.parameter_at_raw(mapping.raw_end)?;
        Some(mapping)
    }

    fn timeline_at_fraction(&self, fraction: f64) -> Option<Tick> {
        if !fraction.is_finite() {
            return None;
        }
        let offset = (fraction.clamp(0.0, 1.0) * self.item.duration.0 as f64).round() as i64;
        Some(Tick(self.item.start.0.saturating_add(offset)).min(self.end))
    }

    fn parameter_at_raw(&self, raw: Tick) -> Option<Tick> {
        if filmcraft_project::graphic::is_layer(self.effect) {
            // graphic_time also returns clip-relative time; that subtraction must fit even
            // when evaluating an out-of-clip neighbour for a Bezier handle.
            raw.0.checked_sub(self.item.source_in.0)?;
        }
        Some(filmcraft_render::graphic_clip::parameter_time(self.item, self.effect, raw))
    }

    fn media_at_timeline(&self, timeline: Tick) -> Option<Tick> {
        if self.item.frame_hold.is_none() || !self.item.hold_filters {
            let rel = timeline.0.checked_sub(self.item.start.0)?;
            let scaled = (rel as f64 * self.item.speed.abs()).round();
            if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled >= i64::MAX as f64 {
                return None;
            }
            // Only validate arithmetic here; the clock itself remains TrackItem's mapping.
            if self.item.reverse {
                let consumed = (self.item.duration.0 as f64 * self.item.speed.abs()).round() as i64;
                self.item.source_in.0.checked_add(consumed)?.checked_sub(scaled as i64)?.checked_sub(1)?;
            } else {
                self.item.source_in.0.checked_add(scaled as i64)?;
            }
        }
        self.parameter_at_raw(self.item.effect_time_at(timeline))
    }

    fn media_at_fraction(&self, fraction: f64) -> Option<Tick> {
        self.media_at_timeline(self.timeline_at_fraction(fraction)?)
    }

    fn timeline_at_raw(&self, raw: Tick) -> Option<Tick> {
        let span = i128::from(self.raw_end.0) - i128::from(self.raw_start.0);
        let offset = i128::from(raw.0) - i128::from(self.raw_start.0);
        let numerator = offset.checked_mul(i128::from(self.item.duration.0))? * span.signum();
        let denominator = span.abs();
        let rounded = numerator.div_euclid(denominator) + i128::from(numerator.rem_euclid(denominator) * 2 >= denominator);
        let offset = i64::try_from(rounded).ok()?;
        Some(Tick(self.item.start.0.checked_add(offset)?))
    }

    fn timeline_at_media(&self, media: Tick) -> Option<Tick> {
        if self.raw_start == self.raw_end {
            return (Some(media) == self.parameter_at_raw(self.raw_start)).then_some(self.item.start);
        }
        if !self.responsive {
            let time = self.timeline_at_raw(media)?;
            self.media_at_timeline(time)?;
            return Some(time);
        }
        // Search the actual renderer clock instead of duplicating its protected-region math.
        // The raw domain includes representable out-of-clip keys used by navigation/handles.
        let source = i128::from(self.item.source_in.0);
        let mut lo = i128::from(i64::MIN).max(i128::from(i64::MIN) + source);
        let mut hi = i128::from(i64::MAX).min(i128::from(i64::MAX) + source);
        let clock = |raw: i128| self.parameter_at_raw(Tick(i64::try_from(raw).ok()?));
        if media < clock(lo)? || media > clock(hi)? {
            return None;
        }
        // At most 64 iterations cover the entire i64 domain, including a stretched plateau.
        for _ in 0..64 {
            if lo >= hi {
                break;
            }
            let mid = lo + (hi - lo) / 2;
            if clock(mid)? < media {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        // A shortened clip can skip a protected-time interval. Do not fabricate a key position
        // for timestamps that the layer clock cannot produce.
        if clock(lo)? != media {
            return None;
        }
        let time = self.timeline_at_raw(Tick(i64::try_from(lo).ok()?))?;
        // Integer stretching can map adjacent raw ticks to one design tick. Prefer a sequence
        // tick that actually produces the key's clock; reverse playback may need the neighbour
        // on the other side of the raw lower bound. Keys between source samples retain nearest
        // sequence placement, as ordinary media keys do.
        let mut best = None;
        for delta in [0, -1, 1] {
            let Some(value) = time.0.checked_add(delta) else { continue };
            let candidate = Tick(value);
            let Some(clock) = self.media_at_timeline(candidate) else { continue };
            let error = (i128::from(clock.0) - i128::from(media.0)).abs();
            if error == 0 {
                return Some(candidate);
            }
            if best.is_none_or(|(_, distance)| error < distance) {
                best = Some((candidate, error));
            }
        }
        best.map(|(time, _)| time)
    }

    fn fraction_at_media(&self, media: Tick) -> Option<f32> {
        let time = self.timeline_at_media(media)?;
        Some(((i128::from(time.0) - i128::from(self.item.start.0)) as f64 / self.item.duration.0 as f64) as f32)
    }

    fn reversed(&self) -> bool {
        self.raw_end < self.raw_start
    }
}

/// The component shown by one value/velocity graph pair. Point keyframes share temporal easing;
/// a component value edit replaces only that coordinate of the original keyframe value.
#[derive(Clone, Copy)]
enum GraphComponent {
    Scalar,
    X,
    Y,
}

impl GraphComponent {
    fn read(self, value: &ParamValue) -> Option<f64> {
        let v = match (self, value) {
            (Self::Scalar, ParamValue::Float(v)) => *v,
            (Self::X, ParamValue::Vec2(p)) => p.x,
            (Self::Y, ParamValue::Vec2(p)) => p.y,
            _ => return None,
        };
        v.is_finite().then_some(v)
    }

    fn edit(self, value: &ParamValue, v: f64) -> Option<Value> {
        if !v.is_finite() {
            return None;
        }
        match (self, value) {
            (Self::Scalar, ParamValue::Float(_)) => Some(json!(v)),
            // An unresolved auto coordinate cannot be sent through the numeric command API.
            // Leave it intact until the project resolves the point, rather than sending null.
            (Self::X, ParamValue::Vec2(p)) if p.y.is_finite() => Some(json!([v, p.y])),
            (Self::Y, ParamValue::Vec2(p)) if p.x.is_finite() => Some(json!([p.x, v])),
            _ => None,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Scalar => "",
            Self::X => ".x",
            Self::Y => ".y",
        }
    }

    fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Scalar => ("Value", "Velocity"),
            Self::X => ("Value X", "Velocity X"),
            Self::Y => ("Value Y", "Velocity Y"),
        }
    }
}

/// Value and velocity graphs, drawn across the keyframe lane. Points use one graph pair per
/// coordinate, with the same layout and controls as floats. Keyframes drag vertically (value);
/// Bezier influence handles drag horizontally and keep the other coordinate unchanged.
#[allow(clippy::too_many_arguments)]
pub(crate) fn graph_rows(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    body: Rect,
    clip: ClipId,
    idx: usize,
    mask: Option<usize>,
    pd: &filmcraft_project::ParamDef,
    param: &filmcraft_project::Param,
    lane: &Rect,
    it: &TrackItem,
    actions: &mut Vec<(String, Value)>,
) {
    let components: &[GraphComponent] = match &param.value {
        ParamValue::Float(_) => &[GraphComponent::Scalar],
        ParamValue::Vec2(_) => &[GraphComponent::X, GraphComponent::Y],
        _ => return,
    };
    for &component in components {
        graph_component(app, ui, body, clip, idx, mask, pd, param, lane, it, actions, component);
    }
}

#[allow(clippy::too_many_arguments)]
fn graph_component(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    body: Rect,
    clip: ClipId,
    idx: usize,
    mask: Option<usize>,
    pd: &filmcraft_project::ParamDef,
    param: &filmcraft_project::Param,
    lane: &Rect,
    it: &TrackItem,
    actions: &mut Vec<(String, Value)>,
    component: GraphComponent,
) {
    let with_mask = |mut v: Value| -> Value {
        if let Some(k) = mask {
            v["mask"] = json!(k);
        }
        v
    };
    if !body.width().is_finite() || body.width() <= 0.0 || !lane.width().is_finite() || lane.width() <= 0.0 || !it.speed.is_finite() {
        return;
    }
    let t = app.tokens;
    let (vr, _) = ui.allocate_exact_size(vec2(body.width(), 110.0), Sense::hover());
    let (velr, _) = ui.allocate_exact_size(vec2(body.width(), 64.0), Sense::hover());
    let Some(effect) = it.effects.get(idx) else { return };
    let Some(timing) = ClipTimeMap::new(it, effect) else { return };
    let dur = it.duration.0 as f64;
    let x_of = |f: f32| lane.min.x + f * lane.width();
    // Bound sampling by screen resolution, including unusually large control-channel layouts.
    let n = (lane.width() / 2.0).clamp(8.0, 8192.0) as usize;
    let vals: Vec<Option<f64>> =
        (0..=n).map(|i| timing.media_at_fraction(i as f64 / n as f64).and_then(|time| component.read(&param.value_at(time)))).collect();
    let bounds = match pd.kind {
        ParamKind::Float { min, max, .. } if min.is_finite() && max.is_finite() && min < max => Some((min, max)),
        ParamKind::Point => Some((-100_000.0, 100_000.0)),
        _ => None,
    };
    let values = vals.iter().flatten().copied().chain(param.keyframes.iter().filter_map(|k| component.read(&k.value)));
    let (lo, hi) = graph_range(values, bounds);
    let area = Rect::from_min_max(pos2(lane.min.x, vr.min.y + 4.0), pos2(lane.max.x, vr.max.y - 4.0));
    let y_of = |v: f64| area.max.y - ((v - lo) / (hi - lo)) as f32 * area.height();
    let v_of = |y: f32| lo + ((area.max.y - y) / area.height()) as f64 * (hi - lo);
    let p = ui.painter();
    p.rect_filled(area, 0.0, Color32::from_rgb(0x19, 0x19, 0x19));
    for g in 1..4 {
        let y = area.min.y + area.height() * g as f32 / 4.0;
        p.line_segment([pos2(area.min.x, y), pos2(area.max.x, y)], Stroke::new(1.0, Color32::from_rgb(0x2a, 0x2a, 0x2a)));
    }
    // range labels in the property column
    let dec = if let ParamKind::Float { decimals, .. } = pd.kind { decimals as usize } else { 1 };
    p.text(pos2(vr.max.x - 30.0, area.min.y + 6.0), Align2::RIGHT_CENTER, format!("{hi:.dec$}"), Tokens::ui(10.0), t.text_dim);
    p.text(pos2(vr.max.x - 30.0, area.max.y - 6.0), Align2::RIGHT_CENTER, format!("{lo:.dec$}"), Tokens::ui(10.0), t.text_dim);
    let (value_label, velocity_label) = component.labels();
    p.text(pos2(vr.min.x + 44.0, area.center().y), Align2::LEFT_CENTER, value_label, Tokens::ui(11.0), t.text_dim);
    graph_line(p, vals.iter().enumerate().map(|(i, v)| v.map(|v| pos2(x_of(i as f32 / n as f32), y_of(v)))), Stroke::new(1.5, t.accent));
    // velocity (units per second, derivative of the sampled value)
    let secs = dur / filmcraft_time::TICKS_PER_SECOND as f64 / n as f64;
    let vel: Vec<Option<f64>> = vals.windows(2).map(|v| v[0].zip(v[1]).map(|(a, b)| (b - a) / secs.max(1e-9)).filter(|v| v.is_finite())).collect();
    let vmax = (vel.iter().flatten().fold(1e-6f64, |a, v| a.max(v.abs())) * 1.15).min(f64::MAX);
    let varea = Rect::from_min_max(pos2(lane.min.x, velr.min.y + 2.0), pos2(lane.max.x, velr.max.y - 4.0));
    p.rect_filled(varea, 0.0, Color32::from_rgb(0x19, 0x19, 0x19));
    let vy = |v: f64| varea.center().y - (v / vmax) as f32 * varea.height() / 2.0;
    p.line_segment([pos2(varea.min.x, varea.center().y), pos2(varea.max.x, varea.center().y)], Stroke::new(1.0, Color32::from_rgb(0x33, 0x33, 0x33)));
    graph_line(
        p,
        vel.iter().enumerate().map(|(i, v)| v.map(|v| pos2(x_of((i as f32 + 0.5) / n as f32), vy(v)))),
        Stroke::new(1.2, Color32::from_rgb(0xd0, 0xa0, 0x40)),
    );
    p.text(pos2(velr.min.x + 44.0, varea.center().y), Align2::LEFT_CENTER, velocity_label, Tokens::ui(11.0), t.text_dim);
    p.text(pos2(velr.max.x - 30.0, varea.min.y + 6.0), Align2::RIGHT_CENTER, format!("{vmax:.1}/s"), Tokens::ui(10.0), t.text_dim);
    let graph_id = format!("effectControls.{}.graph{}", pd.id, component.suffix());
    app.auto.add(&format!("{graph_id}.value"), area, value_label);
    app.auto.add(&format!("{graph_id}.velocity"), varea, velocity_label);
    let p = p.clone();
    // keyframes + handles
    let ks = &param.keyframes;
    for (i, k) in ks.iter().enumerate() {
        let Some(v) = component.read(&k.value) else { continue };
        // A point with an unresolved auto coordinate is read-only until both are numeric.
        if component.edit(&k.value, v).is_none() {
            continue;
        }
        let Some(f) = timing.fraction_at_media(k.time) else { continue };
        if !(-0.01..=1.01).contains(&f) {
            continue;
        }
        let mut id = ui.make_persistent_id(("kfg", clip.0, idx, mask, pd.id, k.time.0));
        if !matches!(component, GraphComponent::Scalar) {
            id = id.with(component.suffix());
        }
        let value_drag = graph_drag_preview(app, ui, id);
        let dy = value_drag.unwrap_or(0.0);
        let c = pos2(x_of(f), y_of(v) + dy);
        if !c.is_finite() {
            continue;
        }
        let r = Rect::from_center_size(c, vec2(10.0, 10.0));
        let resp = ui.interact(r.expand(2.0), id, Sense::drag());
        app.auto.add(&format!("{graph_id}.keyframe.{}", k.time.0), r, "keyframe value");
        // influence handles: flat (ease) tangents with length ∝ influence × neighbouring segment
        let eases_out =
            !matches!(k.interp, filmcraft_project::Interpolation::Linear | filmcraft_project::Interpolation::Hold | filmcraft_project::Interpolation::EaseIn);
        // Incoming easing belongs to this segment, so a preceding Bezier key also makes
        // the current key's incoming influence active, even when its own outgoing mode is Linear.
        let eases_in = i.checked_sub(1).and_then(|i| ks.get(i)).is_some_and(|previous| {
            previous.interp != filmcraft_project::Interpolation::Hold
                && (matches!(
                    k.interp,
                    filmcraft_project::Interpolation::Bezier
                        | filmcraft_project::Interpolation::AutoBezier
                        | filmcraft_project::Interpolation::ContinuousBezier
                        | filmcraft_project::Interpolation::EaseIn
                ) || matches!(previous.interp, filmcraft_project::Interpolation::Bezier | filmcraft_project::Interpolation::ContinuousBezier))
        });
        for (outgoing, on, nb) in [(true, eases_out, ks.get(i + 1)), (false, eases_in && i > 0, if i > 0 { ks.get(i - 1) } else { None })] {
            let (true, Some(nb)) = (on, nb) else { continue };
            let Some(nf) = timing.fraction_at_media(nb.time) else { continue };
            let delta = x_of(nf) - c.x;
            let side = delta.signum();
            let seg = delta.abs();
            if !seg.is_finite() || seg <= 0.0 {
                continue;
            }
            let infl = if outgoing { k.out_influence } else { k.in_influence } as f32;
            if !infl.is_finite() {
                continue;
            }
            let hid = id.with(if outgoing { "out" } else { "in" });
            let influence_drag = graph_drag_preview(app, ui, hid);
            let hdx = influence_drag.unwrap_or(0.0);
            let hx = c.x + side * (infl * seg + hdx * side).clamp(seg * 0.01, seg);
            let hp = pos2(hx, c.y);
            p.line_segment([c, hp], Stroke::new(1.0, Color32::from_rgb(0x90, 0x90, 0x90)));
            p.circle_filled(hp, 3.5, Color32::from_rgb(0xd0, 0xd0, 0xd0));
            let hr = ui.interact(Rect::from_center_size(hp, vec2(10.0, 10.0)), hid.with("h"), Sense::drag());
            let side_name = if outgoing { "out" } else { "in" };
            app.auto.add(&format!("{graph_id}.keyframe.{}.{side_name}", k.time.0), hr.rect, "keyframe influence");
            if hr.dragged() && (hr.drag_started() || influence_drag.is_some()) {
                let nx = graph_drag_offset(ui, &hr, vec2(hdx, 0.0)).x;
                store_graph_drag_preview(app, ui, hid, nx);
            }
            if hr.drag_stopped() && influence_drag.is_some() {
                ui.data_mut(|d| d.remove::<GraphDragPreview>(hid));
                let ni = ((infl * seg + hdx * side) / seg.max(1.0)).clamp(0.01, 1.0);
                let key = if outgoing { "outInfluence" } else { "inInfluence" };
                actions.push(("effects.setKeyframe".into(), with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id, "mediaTime": k.time.0, key: ni}))));
            }
        }
        p.circle_filled(c, 4.5, if resp.dragged() { t.hot_text } else { Color32::from_rgb(0xe0, 0xe0, 0xe0) });
        if resp.dragged() && (resp.drag_started() || value_drag.is_some()) {
            let ny = graph_drag_offset(ui, &resp, vec2(0.0, dy)).y;
            store_graph_drag_preview(app, ui, id, ny);
            p.text(c + vec2(8.0, -10.0), Align2::LEFT_BOTTOM, format!("{:.dec$}", v_of(c.y)), Tokens::ui(10.5), t.hot_text);
        }
        if resp.drag_stopped() && value_drag.is_some() {
            ui.data_mut(|d| d.remove::<GraphDragPreview>(id));
            let nv = v_of(c.y);
            let nv = if let Some((min, max)) = bounds { nv.clamp(min, max) } else { nv };
            if let Some(value) = component.edit(&k.value, nv) {
                actions.push((
                    "effects.setKeyframe".into(),
                    with_mask(json!({"clip": clip.0, "effect": idx, "param": pd.id, "mediaTime": k.time.0, "value": value})),
                ));
            }
        }
    }
}

/// A finite, nonzero plot range, including auto points and values outside the UI's edit bounds.
fn graph_range(values: impl Iterator<Item = f64>, bounds: Option<(f64, f64)>) -> (f64, f64) {
    let (mut lo, mut hi) = values.filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if !lo.is_finite() || !hi.is_finite() {
        return (-1.0, 1.0);
    }
    if hi - lo < 1e-6 {
        lo -= 1.0;
        hi += 1.0;
    }
    let pad = (hi - lo) * 0.12;
    lo -= pad;
    hi += pad;
    if let Some((min, max)) = bounds {
        lo = lo.max(min);
        hi = hi.min(max);
        if !lo.is_finite() || !hi.is_finite() || hi <= lo {
            return (min, max);
        }
    }
    if !lo.is_finite() || !hi.is_finite() || !(hi - lo).is_finite() || hi <= lo {
        return (-1.0, 1.0);
    }
    (lo, hi)
}

/// Keep invalid/auto samples out of egui geometry, without connecting across a missing segment.
fn graph_line(painter: &egui::Painter, points: impl Iterator<Item = Option<Pos2>>, stroke: Stroke) {
    let mut line = Vec::new();
    for point in points.chain(std::iter::once(None)) {
        if let Some(point) = point.filter(|p| p.is_finite()) {
            line.push(point);
        } else if line.len() >= 2 {
            painter.add(egui::Shape::line(std::mem::take(&mut line), stroke));
        } else {
            line.clear();
        }
    }
}

/// Read-only text for parameters without an inline editor (LUT references, free text, curves).
fn param_text(project: &filmcraft_project::Project, id: &str, value: &ParamValue) -> String {
    match value {
        ParamValue::Text(s) if id.ends_with("_lut") => filmcraft_render::luts::label(Some(project), s),
        ParamValue::Text(s) if s.is_empty() => "None".into(),
        ParamValue::Text(s) => s.clone(),
        ParamValue::Curve(pts) if pts.is_empty() || *pts == [[0.0, 0.0], [1.0, 1.0]] => "Default".into(),
        ParamValue::Curve(pts) => format!("Custom ({} points)", pts.len()),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod param_text_tests {
    use super::*;

    #[test]
    fn readable_values_for_text_lut_and_curve_params() {
        let p = filmcraft_project::Project::default();
        assert_eq!(param_text(&p, "input_lut", &ParamValue::Text(String::new())), "None");
        assert_eq!(param_text(&p, "look_lut", &ParamValue::Text("builtin:look-teal-orange".into())), "Teal & Orange");
        assert_eq!(param_text(&p, "label", &ParamValue::Text(String::new())), "None");
        assert_eq!(param_text(&p, "label", &ParamValue::Text("Reel 3".into())), "Reel 3");
        assert_eq!(param_text(&p, "curve_luma", &ParamValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]])), "Default");
        assert_eq!(param_text(&p, "hue_vs_sat", &ParamValue::Curve(vec![])), "Default");
        assert_eq!(param_text(&p, "curve_luma", &ParamValue::Curve(vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]])), "Custom (3 points)");
    }
}

#[cfg(test)]
mod drag_undo_tests {
    use serde_json::json;

    /// #201: a drag value changes only once the mouse moves, after the press frame; two drags of
    /// the same parameter are still two undo steps.
    #[test]
    fn each_press_begins_its_own_undo_step() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        let mut app = crate::FilmcraftApp::new(s);
        let ctx = egui::Context::default();
        let opacity = |app: &crate::FilmcraftApp| {
            let q = app.session.active_sequence().unwrap();
            let it = q.find_item(filmcraft_project::ClipId(clip)).unwrap().1;
            it.effects.iter().find(|e| e.effect == "opacity").unwrap().params["opacity"].value.as_f64().unwrap()
        };
        let start = opacity(&app);
        let pos = egui::pos2(10.0, 10.0);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut time = 0.0;
        let mut frame = |app: &mut crate::FilmcraftApp, events: Vec<egui::Event>, value: Option<f64>| {
            time += 0.1;
            let raw = egui::RawInput { time: Some(time), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let actions = value.map(|v| ("effects.setParam".to_string(), json!({"clip": clip, "effect": "opacity", "param": "opacity", "value": v})));
                super::run(app, ui.ctx(), actions.into_iter().collect());
            });
            out.textures_delta.clear();
        };
        for values in [[90.0, 70.0], [50.0, 60.0]] {
            frame(&mut app, vec![egui::Event::PointerMoved(pos), button(true)], None); // the press: nothing changes yet
            for v in values {
                frame(&mut app, vec![egui::Event::PointerMoved(pos)], Some(v));
            }
            frame(&mut app, vec![button(false)], None);
        }
        assert_eq!(opacity(&app), 60.0);
        app.session.undo();
        assert_eq!(opacity(&app), 70.0, "undo takes back only the second drag");
        app.session.undo();
        assert_eq!(opacity(&app), start, "and then the first");
    }
}

#[cfg(test)]
mod graph_tests;
