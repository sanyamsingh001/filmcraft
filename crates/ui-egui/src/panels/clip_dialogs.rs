//! Edit / Clip / File menu dialogs (M3.10): Paste Attributes, Remove Attributes, Offline File,
//! Make Subclip, Edit Subclip, Modify ▸ Audio Channels, Modify ▸ Timecode, Frame Hold Options,
//! Field Options, Clip Speed / Duration, Nest… (Nested Sequence Name), and the Close Project
//! "save changes?" prompt.
//!
//! The open dialog lives in `UiState::clip_dialog` (serde): the engine command it runs and the
//! parameters being edited — the same JSON the command takes — so agents can open a dialog from
//! the menu, change `params` with `ui.set`, and click OK. Menu items and shortcuts invoked without
//! params open the dialog; with params the engine command runs directly.
//!
//! Automation ids (`<p>` = the dialog prefix below, `<key>` = a parameter):
//! - every dialog: `<p>.ok`, `<p>.cancel`, and `<p>.<key>` for each control;
//! - Paste Attributes `pasteAttributes.*`: `motion`, `opacity`, `timeRemapping`, `volume`,
//!   `channelVolume`, `panner`, `effects`, `effect.<id>`, `scaleTimes`;
//! - Remove Attributes `removeAttributes.*`: the same without `scaleTimes`;
//! - Offline File `offlineFile.*`: `fileName`, `name`, `tapeName`, `video`, `audio`, `width`,
//!   `height`, `fps`, `timecode`, `seconds`, `description`;
//! - Nested Sequence Name `nest.*`: `name`;
//! - Make Subclip `makeSubclip.*`: `name`, `startFrame`, `endFrame`, `restrictTrims`;
//! - Edit Subclip `editSubclip.*`: `startFrame`, `endFrame`, `restrictTrims`, `convertToMaster`;
//! - Audio Channels `audioChannels.*`: `format.<mono|stereo|5.1|adaptive>`, `count`,
//!   `clip.<n>.ch.<c>` (n, c 0-based);
//! - Timecode `timecode.*`: `timecode`, `tapeName`, `reset`;
//! - Frame Hold Options `frameHold.*`: `enabled`, `holdOn.<in|out|playhead|sourceTimecode|sequenceTime>`,
//!   `timecode`, `holdFilters`;
//! - Field Options `fieldOptions.*`: `reverseFieldDominance`, `processing.<none|alwaysDeinterlace|flickerRemoval>`;
//! - Clip Speed / Duration `speedDuration.*`: `speed`, `reverse`, `ripple`,
//!   `interpolation.<frameSampling|frameBlending|opticalFlow>`;
//! - Close Project `closeProject.save`, `closeProject.dontSave`, `closeProject.cancel`.

use egui::{Align2, RichText};
use filmcraft_project::{AudioChannelMap, AudioChannels, ItemKind, TrackKind};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::state::ClipDialogDraft;

type Elems = Vec<(String, egui::Rect, String)>;

/// A channel draft belongs to one project snapshot and its captured item/clip targets. Holding
/// both Arcs prevents identity reuse when a different project replaces the session.
pub(crate) struct AudioChannelsDraftContext {
    project: std::sync::Arc<filmcraft_project::Project>,
    media: std::sync::Arc<filmcraft_engine::MediaPool>,
    targets: Value,
}

impl AudioChannelsDraftContext {
    fn matches(&self, app: &FilmcraftApp, draft: &ClipDialogDraft) -> bool {
        std::sync::Arc::ptr_eq(&self.project, &app.session.project)
            && std::sync::Arc::ptr_eq(&self.media, &app.session.media)
            && self.targets == audio_targets(&draft.params)
    }
}

fn audio_targets(params: &Value) -> Value {
    json!({"items": params.get("items"), "clips": if params.get("items").is_none() { params.get("clips") } else { None }})
}

fn push(elems: &mut Elems, id: impl Into<String>, r: &egui::Response, label: impl Into<String>) {
    elems.push((id.into(), r.rect, label.into()));
}

/// Dialog title and automation prefix of a command's dialog.
fn meta(command: &str) -> Option<(&'static str, &'static str)> {
    Some(match command {
        "edit.pasteAttributes" => ("Paste Attributes", "pasteAttributes"),
        "edit.removeAttributes" => ("Remove Attributes", "removeAttributes"),
        "file.newOfflineFile" => ("New Offline File", "offlineFile"),
        "clip.nest" => ("Nested Sequence Name", "nest"),
        "clip.makeSubclip" => ("Make Subclip", "makeSubclip"),
        "clip.editSubclip" => ("Edit Subclip", "editSubclip"),
        "clip.audioChannels" => ("Modify Clip: Audio Channels", "audioChannels"),
        "clip.modifyTimecode" => ("Modify Clip: Timecode", "timecode"),
        "clip.frameHoldOptions" => ("Frame Hold Options", "frameHold"),
        "clip.fieldOptions" => ("Field Options", "fieldOptions"),
        "clip.speedDuration" => ("Clip Speed / Duration", "speedDuration"),
        "file.closeProject" => ("Save Project", "closeProject"),
        _ => return None,
    })
}

const INTRINSIC_VIDEO: [(&str, &str); 3] = [("motion", "Motion"), ("opacity", "Opacity"), ("timeRemapping", "Time Remapping")];
const INTRINSIC_AUDIO: [(&str, &str); 3] = [("volume", "Volume"), ("channelVolume", "Channel Volume"), ("panner", "Panner")];
const FORMATS: [(&str, &str); 4] = [("mono", "Mono"), ("stereo", "Stereo"), ("5.1", "5.1"), ("adaptive", "Adaptive")];
const HOLD_ON: [(&str, &str); 5] =
    [("in", "In Point"), ("out", "Out Point"), ("playhead", "Playhead"), ("sourceTimecode", "Source Timecode"), ("sequenceTime", "Sequence Time")];

/// Menu / shortcut entry points without params open a dialog. Returns None for other ids.
pub fn route(app: &mut FilmcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let empty = params.as_object().is_none_or(|m| m.is_empty());
    if !empty {
        return None;
    }
    // Save All on a never-saved project asks where to save, like Save
    if id == "file.saveAll" && app.session.path.is_none() {
        return Some(app.file_dialog("file.save", params));
    }
    if id == "file.closeProject" && !app.session.is_dirty() {
        return None;
    }
    meta(id)?;
    if let Err(e) = filmcraft_engine::find_command(id).map_or(Ok(()), |c| (c.enabled)(&app.session)) {
        app.ui.status = e.clone();
        return Some(Err(e));
    }
    let (params, info) = defaults(app, id);
    if let Some(error) = info.get("error").and_then(Value::as_str) {
        app.ui.status = error.into();
        return Some(Err(error.into()));
    }
    app.audio_channels_draft = if id == "clip.audioChannels" {
        Some(AudioChannelsDraftContext { project: app.session.project.clone(), media: app.session.media.clone(), targets: audio_targets(&params) })
    } else {
        None
    };
    app.ui.clip_dialog = Some(ClipDialogDraft { command: id.into(), params, info, error: String::new() });
    Some(Ok(json!({"dialog": meta(id).map(|m| m.1)})))
}

/// Initial parameters (and display info) of a command's dialog.
fn defaults(app: &FilmcraftApp, id: &str) -> (Value, Value) {
    let s = &app.session;
    match id {
        "edit.pasteAttributes" | "edit.removeAttributes" => {
            // effects offered: the copied clips' (Paste) or the selected clips' (Remove) standard effects
            let mut names: Vec<(String, String)> = Vec::new();
            let mut add = |e: &filmcraft_project::EffectInstance| {
                if let Some(d) = e.def().filter(|d| !d.intrinsic && !filmcraft_project::graphic::is_layer_id(d.id))
                    && !names.iter().any(|n| n.0 == d.id)
                {
                    names.push((d.id.to_string(), d.name.to_string()));
                }
            };
            let mut has_v = false;
            let mut has_a = false;
            if id == "edit.pasteAttributes" {
                for (k, _, it) in &s.state.clipboard {
                    has_v |= *k == TrackKind::Video;
                    has_a |= *k == TrackKind::Audio;
                    it.effects.iter().for_each(&mut add);
                }
            } else if let Some(q) = s.active_sequence() {
                for c in &s.state.selection {
                    if let Some((tid, it)) = q.find_item(*c) {
                        let v = q.video_tracks.iter().any(|t| t.id == tid);
                        has_v |= v;
                        has_a |= !v;
                        it.effects.iter().for_each(&mut add);
                    }
                }
            }
            let mut p = json!({"motion": has_v, "opacity": has_v, "timeRemapping": has_v, "volume": has_a, "channelVolume": has_a, "panner": has_a,
                "effects": names.iter().map(|n| n.0.clone()).collect::<Vec<_>>()});
            if id == "edit.pasteAttributes" {
                p["scaleTimes"] = json!(true);
            }
            (p, json!({"video": has_v, "audio": has_a, "effects": names}))
        }
        "file.newOfflineFile" => {
            let st = s.active_sequence().map(|q| q.settings.clone()).unwrap_or_default();
            (
                json!({"fileName": "Offline.mov", "name": "", "tapeName": "", "description": "", "video": true, "audio": true, "width": st.width, "height": st.height,
                    "fps": st.frame_rate.as_f64(), "timecode": "00:00:00:00", "seconds": 10.0}),
                Value::Null,
            )
        }
        "clip.nest" => (json!({"name": filmcraft_engine::commands::next_nested_name(s)}), Value::Null),
        "clip.makeSubclip" => {
            let d = filmcraft_engine::clip_ops::subclip_defaults(s, &json!({})).unwrap_or_else(|| json!({}));
            (json!({"name": d["name"], "startFrame": d["startFrame"], "endFrame": d["endFrame"], "restrictTrims": true}), json!({"fps": d["fps"]}))
        }
        "clip.editSubclip" => {
            let sub = s.state.project_selection.iter().find_map(|i| s.project.item(*i).filter(|it| matches!(it.kind, ItemKind::Subclip { .. })));
            let Some(it) = sub else { return (json!({}), Value::Null) };
            let ItemKind::Subclip { parent, range, restrict_trims } = &it.kind else { return (json!({}), Value::Null) };
            let rate = s.project.item(*parent).and_then(|p| p.as_media()).map(|m| m.frame_rate()).unwrap_or_default();
            (
                json!({"item": it.id.0, "startFrame": rate.frame_at(range.start), "endFrame": rate.frame_at(range.end()), "restrictTrims": restrict_trims, "convertToMaster": false}),
                json!({"name": it.name, "fps": rate.as_f64()}),
            )
        }
        "clip.audioChannels" => {
            let timeline_target = app.ui.focused == crate::panels::PanelKind::Timeline
                && s.active_sequence().is_some_and(|q| s.state.selection.iter().any(|clip| q.audio_tracks.iter().any(|track| track.item(*clip).is_some())));
            let items: Vec<_> = s
                .state
                .project_selection
                .iter()
                .filter(|_| !timeline_target)
                .filter_map(|&item| s.project.resolve_media(item).filter(|(_, media, _)| media.info.has_audio()).map(|(root, _, _)| root))
                .collect();
            if let Some((_, media, _)) = items.first().and_then(|&item| s.project.resolve_media(item)) {
                let map = match &media.interpret.audio_channels {
                    Some(map) => map.clone(),
                    None => match AudioChannelMap::for_streams(AudioChannels::Stereo, &media.info.audio_streams) {
                        Ok(map) => map,
                        Err(error) => return (json!({}), json!({"error": error})),
                    },
                };
                let mut streams = if map.streams.is_empty() { vec![0; map.clips.len()] } else { map.streams.clone() };
                let mut clips = map.clips;
                // Legacy channel rows describe stream zero; PR361 also places every other stream.
                if media.interpret.audio_channels.as_ref().is_some_and(|map| map.streams.is_empty()) {
                    for (stream, audio) in media.info.audio_streams.iter().enumerate().skip(1) {
                        if clips.len() >= filmcraft_project::MAX_AUDIO_CHANNEL_CLIPS {
                            return (json!({}), json!({"error": "The complete audio map exceeds the sequence track limit."}));
                        }
                        let Ok(channels) = u16::try_from(audio.channels) else {
                            return (json!({}), json!({"error": "Unsupported audio channel count."}));
                        };
                        clips.push((0..channels).collect());
                        streams.push(stream);
                    }
                }
                (
                    json!({"items": items.iter().map(|id| id.0).collect::<Vec<_>>(), "format": format_name(map.format), "clips": clips, "streams": streams}),
                    json!({"items": true, "audioStreams": media.info.audio_streams}),
                )
            } else {
                let clips: Vec<_> = s
                    .active_sequence()
                    .into_iter()
                    .flat_map(|q| s.state.selection.iter().filter_map(move |id| q.audio_tracks.iter().find_map(|track| track.item(*id))))
                    .collect();
                let first = clips.first().copied();
                let streams = first
                    .map(|clip| {
                        if let Some((_, media, _)) = s.project.resolve_media(clip.item) {
                            media.info.audio_streams.clone()
                        } else {
                            s.project
                                .sequence(clip.item)
                                .map(|sequence| {
                                    vec![filmcraft_media::AudioStreamInfo {
                                        sample_rate: sequence.settings.sample_rate.max(1),
                                        channels: 2,
                                        codec: "Sequence".into(),
                                        bits_per_sample: None,
                                    }]
                                })
                                .unwrap_or_default()
                        }
                    })
                    .unwrap_or_default();
                (
                    json!({"clips": clips.iter().map(|clip| clip.id.0).collect::<Vec<_>>(), "channels": first.map(|clip| clip.source_channels.clone()).unwrap_or_default()}),
                    json!({"items": false, "audioStreams": streams, "stream": first.map_or(0, |clip| clip.audio_stream)}),
                )
            }
        }
        "clip.modifyTimecode" => {
            let it = s.state.project_selection.iter().find_map(|i| s.project.item(*i).and_then(|it| Some((it, it.as_media()?))));
            let (tc, tape) = it
                .map(|(it, m)| {
                    let f = m.info.start_timecode.unwrap_or(0);
                    (filmcraft_time::format_timecode_frames(f, m.frame_rate(), false), it.metadata.get("Tape Name").cloned().unwrap_or_default())
                })
                .unwrap_or_else(|| ("00:00:00:00".into(), String::new()));
            (json!({"timecode": tc, "tapeName": tape}), Value::Null)
        }
        "clip.frameHoldOptions" => {
            let it = s.active_sequence().and_then(|q| s.state.selection.iter().find_map(|c| q.find_item(*c).map(|x| x.1.clone())));
            let enabled = it.as_ref().is_none_or(|i| i.frame_hold.is_some());
            let hold_filters = it.as_ref().is_some_and(|i| i.hold_filters);
            let tc = it
                .as_ref()
                .map(|i| {
                    let rate = s.project.item(i.item).and_then(|p| p.as_media()).map(|m| m.frame_rate()).unwrap_or_default();
                    filmcraft_time::format_timecode_frames(rate.frame_at(i.frame_hold.unwrap_or(i.source_in)), rate, false)
                })
                .unwrap_or_else(|| "00:00:00:00".into());
            (json!({"enabled": enabled, "holdOn": "in", "timecode": tc, "holdFilters": hold_filters}), Value::Null)
        }
        "clip.fieldOptions" => (json!({"reverseFieldDominance": false, "processing": "none"}), Value::Null),
        "clip.speedDuration" => {
            // the first selected clip's current speed; its duration at that speed for the readout
            let q = s.active_sequence();
            let it = q.and_then(|q| s.state.selection.iter().find_map(|c| q.find_item(*c).map(|x| x.1.clone())));
            let fps = q.map(|q| q.settings.frame_rate.as_f64()).unwrap_or(24.0);
            let speed = it.as_ref().map_or(1.0, |i| i.speed);
            let interp = it.as_ref().map_or(filmcraft_project::TimeInterpolation::default(), |i| i.time_interpolation);
            (
                json!({"speed": (speed.abs() * 100.0 * 100.0).round() / 100.0, "reverse": speed < 0.0, "ripple": false, "interpolation": interp.name()}),
                json!({"duration": it.as_ref().map_or(0, |i| i.duration.0), "speed": speed.abs(), "fps": fps}),
            )
        }
        _ => (json!({}), Value::Null),
    }
}

fn audio_channels_ui(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, params: &mut Value, info: &Value, error_height: f32) -> Result<(), String> {
    let descriptors: Vec<filmcraft_media::AudioStreamInfo> =
        serde_json::from_value(info["audioStreams"].clone()).map_err(|_| "audio stream metadata is unavailable")?;
    if descriptors.is_empty() || descriptors.len() > filmcraft_media::MAX_AUDIO_STREAMS {
        return Err("audio stream metadata is unavailable".into());
    }
    let count_channels = |stream: usize| {
        descriptors
            .get(stream)
            .and_then(|audio| u16::try_from(audio.channels).ok())
            .filter(|&n| n > 0)
            .ok_or_else(|| "unsupported audio channel count".to_string())
    };
    if info["items"].as_bool().unwrap_or(false) {
        let mut clips: Vec<Vec<u16>> = serde_json::from_value(params["clips"].clone()).map_err(|_| "invalid audio channel rows")?;
        let mut streams: Vec<usize> = serde_json::from_value(params["streams"].clone()).map_err(|_| "invalid audio stream rows")?;
        if streams.is_empty() {
            streams.resize(clips.len(), 0);
        }
        if clips.is_empty() || clips.len() > filmcraft_project::MAX_AUDIO_CHANNEL_CLIPS || clips.len() != streams.len() {
            return Err("invalid audio channel rows".into());
        }
        for &stream in &streams {
            count_channels(stream)?;
        }
        ui.horizontal(|ui| {
            ui.label("Clip Channel Format:");
            for (key, label) in FORMATS {
                let r = ui.radio(params["format"].as_str() == Some(key), label);
                push(elems, format!("{pre}.format.{key}"), &r, label);
                if r.clicked() {
                    let mut selected = Vec::new();
                    for &stream in &streams {
                        if !selected.contains(&stream) {
                            selected.push(stream);
                        }
                    }
                    let selected_info: Vec<_> = selected.iter().filter_map(|&stream| descriptors.get(stream).cloned()).collect();
                    if let Ok(map) = AudioChannelMap::for_streams(format_of(key), &selected_info) {
                        streams = (0..map.clips.len()).filter_map(|index| map.stream_for_clip(index).and_then(|index| selected.get(index).copied())).collect();
                        clips = map.clips;
                        params["format"] = json!(key);
                    }
                }
            }
        });
        let max_rows = descriptors
            .iter()
            .fold(0usize, |n, audio| n.saturating_add(audio.channels as usize))
            .clamp(1, filmcraft_project::MAX_AUDIO_CHANNEL_CLIPS)
            .max(clips.len());
        ui.horizontal(|ui| {
            ui.label("Number of Audio Clips:");
            let mut count = clips.len();
            let r = ui.add(egui::DragValue::new(&mut count).range(1..=max_rows));
            push(elems, format!("{pre}.count"), &r, "Number of Audio Clips");
            if r.changed() {
                let count = count.clamp(1, max_rows);
                while clips.len() < count {
                    let stream = streams.last().copied().unwrap_or(0);
                    let n = count_channels(stream).unwrap_or(1);
                    clips.push(vec![u16::try_from(clips.len()).unwrap_or(u16::MAX).min(n - 1)]);
                    streams.push(stream);
                }
                clips.truncate(count);
                streams.truncate(count);
            }
        });
        ui.label(RichText::new("Media Source Channels").strong());
        let max_channels = (0..descriptors.len()).filter_map(|stream| count_channels(stream).ok()).max().unwrap_or(1);
        if clips.len().saturating_mul(usize::from(max_channels)) > 8192 {
            return Err("This audio channel map is too large to display.".into());
        }
        // Reserve the existing footer before expanding the channel matrix in a small viewport.
        let footer_height = ui.spacing().interact_size.y + 6.0 + 2.0 * ui.spacing().item_spacing.y + error_height;
        let matrix_height = (ui.ctx().content_rect().bottom() - ui.cursor().top() - footer_height).clamp(0.0, 220.0);
        egui::ScrollArea::both().max_height(matrix_height).show(ui, |ui| {
            egui::Grid::new("audio-channel-matrix").striped(true).show(ui, |ui| {
                ui.label("");
                if descriptors.len() > 1 {
                    ui.label("Audio Stream");
                }
                for channel in 0..max_channels {
                    ui.label(format!("Ch. {}", channel + 1));
                }
                ui.end_row();
                for (index, channels) in clips.iter_mut().enumerate() {
                    ui.label(format!("Clip {}", index + 1));
                    let Some(stream) = streams.get_mut(index) else { continue };
                    if descriptors.len() > 1 {
                        let before = *stream;
                        let r = egui::ComboBox::from_id_salt(("audio-channel-stream", index))
                            .selected_text(format!("{}", stream.saturating_add(1)))
                            .width(52.0)
                            .show_ui(ui, |ui| {
                                for candidate in 0..descriptors.len() {
                                    let r = ui
                                        .add_enabled_ui(count_channels(candidate).is_ok(), |ui| {
                                            ui.selectable_value(stream, candidate, format!("{}", candidate + 1))
                                        })
                                        .inner;
                                    push(elems, format!("{pre}.clip.{index}.stream.{candidate}"), &r, format!("Audio Stream {}", candidate + 1));
                                }
                            });
                        push(elems, format!("{pre}.clip.{index}.stream"), &r.response, "Audio Stream");
                        if before != *stream {
                            *channels = (0..count_channels(*stream).unwrap_or(1))
                                .take(match format_of(params["format"].as_str().unwrap_or("stereo")) {
                                    AudioChannels::Mono => 1,
                                    AudioChannels::Stereo => 2,
                                    _ => usize::MAX,
                                })
                                .collect();
                        }
                    }
                    let n = count_channels(*stream).unwrap_or(1);
                    for channel in 0..max_channels {
                        let mut on = channels.contains(&channel);
                        let r = ui.add_enabled(channel < n, egui::Checkbox::new(&mut on, ""));
                        push(elems, format!("{pre}.clip.{index}.ch.{channel}"), &r, format!("Clip {} Ch. {}", index + 1, channel + 1));
                        if r.changed() {
                            if on {
                                channels.push(channel);
                                channels.sort_unstable();
                            } else {
                                channels.retain(|&c| c != channel);
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        });
        params["clips"] = json!(clips);
        if descriptors.len() > 1 {
            params["streams"] = json!(streams);
        } else if let Some(params) = params.as_object_mut() {
            params.remove("streams");
        }
    } else {
        let default_stream = info["stream"].as_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
        let mut stream = params.get("stream").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(default_stream);
        if descriptors.len() > 1 {
            ui.horizontal(|ui| {
                ui.label("Audio Stream:");
                let before = stream;
                let r = egui::ComboBox::from_id_salt("audio-channel-stream").selected_text(format!("{}", stream.saturating_add(1))).show_ui(ui, |ui| {
                    for candidate in 0..descriptors.len() {
                        let r = ui
                            .add_enabled_ui(count_channels(candidate).is_ok(), |ui| ui.selectable_value(&mut stream, candidate, format!("{}", candidate + 1)))
                            .inner;
                        push(elems, format!("{pre}.stream.{candidate}"), &r, format!("Audio Stream {}", candidate + 1));
                    }
                });
                push(elems, format!("{pre}.stream"), &r.response, "Audio Stream");
                if stream != before {
                    params["stream"] = json!(stream);
                    params["channels"] = json!([]);
                }
            });
        }
        let n = count_channels(stream)?;
        ui.label("Source channels the selected audio clips play:");
        let mut channels: Vec<u16> = serde_json::from_value(params["channels"].clone()).map_err(|_| "invalid source channels")?;
        ui.horizontal_wrapped(|ui| {
            for channel in 0..n {
                let mut on = channels.is_empty() || channels.contains(&channel);
                let r = ui.checkbox(&mut on, format!("Ch. {}", channel + 1));
                push(elems, format!("{pre}.clip.0.ch.{channel}"), &r, format!("Ch. {}", channel + 1));
                if r.changed() {
                    if channels.is_empty() {
                        channels = (0..n).collect();
                    }
                    if on {
                        channels.push(channel);
                        channels.sort_unstable();
                        channels.dedup();
                    } else {
                        channels.retain(|&c| c != channel);
                    }
                }
            }
        });
        params["channels"] = json!(channels);
    }
    Ok(())
}

fn format_name(f: AudioChannels) -> &'static str {
    match f {
        AudioChannels::Mono => "mono",
        AudioChannels::Stereo => "stereo",
        AudioChannels::Surround51 => "5.1",
        AudioChannels::Adaptive => "adaptive",
    }
}

fn format_of(name: &str) -> AudioChannels {
    match name {
        "mono" => AudioChannels::Mono,
        "5.1" => AudioChannels::Surround51,
        "adaptive" => AudioChannels::Adaptive,
        _ => AudioChannels::Stereo,
    }
}

fn check(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, enabled: bool) {
    let mut v = p.get(key).and_then(Value::as_bool).unwrap_or(false);
    let r = ui.add_enabled(enabled, egui::Checkbox::new(&mut v, label));
    push(elems, format!("{pre}.{key}"), &r, label);
    if r.changed() {
        p[key] = json!(v);
    }
}

fn text(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, width: f32) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut v = p.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let r = ui.add(egui::TextEdit::singleline(&mut v).desired_width(width));
        push(elems, format!("{pre}.{key}"), &r, label);
        if r.changed() {
            p[key] = json!(v);
        }
    });
}

fn number(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, range: std::ops::RangeInclusive<f64>, suffix: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut v = p.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        let r = ui.add(egui::DragValue::new(&mut v).range(range).suffix(suffix));
        push(elems, format!("{pre}.{key}"), &r, label);
        if r.changed() {
            p[key] = json!(v);
        }
    });
}

fn frames(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, fps: f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut v = p.get(key).and_then(Value::as_i64).unwrap_or(0);
        let r = ui.add(egui::DragValue::new(&mut v).range(0..=i64::MAX / 4).suffix(" fr"));
        push(elems, format!("{pre}.{key}"), &r, label);
        if r.changed() {
            p[key] = json!(v);
        }
        let rate = filmcraft_time::FrameRate::from_f64(if fps > 0.0 { fps } else { 24.0 });
        ui.label(RichText::new(filmcraft_time::format_timecode_frames(v, rate, false)).monospace().weak());
    });
}

/// Draw the open dialog, if any.
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.clip_dialog.clone() else {
        app.audio_channels_draft = None;
        return;
    };
    let Some((title, pre)) = meta(&d.command) else {
        app.ui.clip_dialog = None;
        return;
    };
    let mut elems: Elems = Vec::new();
    let mut action: Option<&'static str> = None;
    let project_name = app.session.project.name.clone();
    let can_save = app.session.path.is_some();
    egui::Window::new(title).collapsible(false).resizable(false).default_width(360.0).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        let p = &mut d.params;
        match d.command.as_str() {
            "edit.pasteAttributes" | "edit.removeAttributes" => {
                let (v, a) = (d.info["video"].as_bool().unwrap_or(true), d.info["audio"].as_bool().unwrap_or(true));
                ui.label(RichText::new("Video Attributes").strong());
                for (k, l) in INTRINSIC_VIDEO {
                    check(ui, &mut elems, pre, p, k, l, v);
                }
                ui.label(RichText::new("Audio Attributes").strong());
                for (k, l) in INTRINSIC_AUDIO {
                    check(ui, &mut elems, pre, p, k, l, a);
                }
                let effects: Vec<(String, String)> = d.info["effects"]
                    .as_array()
                    .map(|x| x.iter().filter_map(|e| Some((e[0].as_str()?.to_string(), e[1].as_str()?.to_string()))).collect())
                    .unwrap_or_default();
                ui.label(RichText::new("Effects").strong());
                if effects.is_empty() {
                    ui.label(RichText::new("No effects").weak());
                }
                for (id, name) in &effects {
                    let mut on = p["effects"].as_array().is_some_and(|x| x.iter().any(|e| e.as_str() == Some(id)));
                    let r = ui.checkbox(&mut on, name);
                    push(&mut elems, format!("{pre}.effect.{id}"), &r, name.as_str());
                    if r.changed() {
                        let mut list: Vec<Value> = p["effects"].as_array().cloned().unwrap_or_default();
                        list.retain(|e| e.as_str() != Some(id));
                        if on {
                            list.push(json!(id));
                        }
                        p["effects"] = Value::Array(list);
                    }
                }
                if d.command == "edit.pasteAttributes" {
                    ui.separator();
                    check(ui, &mut elems, pre, p, "scaleTimes", "Scale Attribute Times", true);
                }
            }
            "file.newOfflineFile" => {
                text(ui, &mut elems, pre, p, "fileName", "File Name:", 220.0);
                text(ui, &mut elems, pre, p, "name", "Clip Name:", 220.0);
                text(ui, &mut elems, pre, p, "tapeName", "Tape Name:", 220.0);
                text(ui, &mut elems, pre, p, "description", "Description:", 220.0);
                ui.horizontal(|ui| {
                    ui.label("Contains:");
                    check(ui, &mut elems, pre, p, "video", "Video", true);
                    check(ui, &mut elems, pre, p, "audio", "Audio", true);
                });
                let has_v = p["video"].as_bool().unwrap_or(true);
                ui.add_enabled_ui(has_v, |ui| {
                    number(ui, &mut elems, pre, p, "width", "Width:", 16.0..=16384.0, " px");
                    number(ui, &mut elems, pre, p, "height", "Height:", 16.0..=16384.0, " px");
                    number(ui, &mut elems, pre, p, "fps", "Frame Rate:", 1.0..=240.0, " fps");
                });
                text(ui, &mut elems, pre, p, "timecode", "Media Start:", 120.0);
                number(ui, &mut elems, pre, p, "seconds", "Duration:", 0.05..=86_400.0, " s");
            }
            "clip.nest" => {
                // the name is ready to type over, and Enter accepts it
                let mut v = p["name"].as_str().unwrap_or_default().to_string();
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    let id = egui::Id::new("nest-name");
                    let r = ui.add(egui::TextEdit::singleline(&mut v).desired_width(240.0).id(id));
                    push(&mut elems, format!("{pre}.name"), &r, "Name:");
                    if r.changed() {
                        p["name"] = json!(v);
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        action = Some("ok");
                    }
                    let first = egui::Id::new("nest-name-focused");
                    if !ui.data(|d| d.get_temp::<bool>(first).unwrap_or(false)) {
                        ui.data_mut(|d| d.insert_temp(first, true));
                        r.request_focus();
                    }
                });
            }
            "clip.makeSubclip" | "clip.editSubclip" => {
                let fps = d.info["fps"].as_f64().unwrap_or(24.0);
                if d.command == "clip.makeSubclip" {
                    text(ui, &mut elems, pre, p, "name", "Name:", 240.0);
                } else {
                    ui.label(format!("Subclip: {}", d.info["name"].as_str().unwrap_or_default()));
                }
                frames(ui, &mut elems, pre, p, "startFrame", "Start:", fps);
                frames(ui, &mut elems, pre, p, "endFrame", "End:", fps);
                check(ui, &mut elems, pre, p, "restrictTrims", "Restrict Trims To Subclip Boundaries", true);
                if d.command == "clip.editSubclip" {
                    check(ui, &mut elems, pre, p, "convertToMaster", "Convert to Master Clip", true);
                }
            }
            "clip.audioChannels" => {
                let error_height = if d.error.is_empty() {
                    0.0
                } else {
                    let galley = ui.painter().layout(
                        d.error.clone(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        egui::Color32::from_rgb(0xe0, 0x60, 0x60),
                        ui.available_width(),
                    );
                    galley.size().y + ui.spacing().item_spacing.y
                };
                if let Err(error) = audio_channels_ui(ui, &mut elems, pre, p, &d.info, error_height) {
                    d.error = error;
                }
            }
            "clip.modifyTimecode" => {
                text(ui, &mut elems, pre, p, "timecode", "Set Timecode:", 120.0);
                text(ui, &mut elems, pre, p, "tapeName", "Tape Name:", 200.0);
                check(ui, &mut elems, pre, p, "reset", "Use the file's timecode", true);
            }
            "clip.frameHoldOptions" => {
                check(ui, &mut elems, pre, p, "enabled", "Hold On", true);
                let on = p["enabled"].as_bool().unwrap_or(true);
                ui.add_enabled_ui(on, |ui| {
                    for (k, l) in HOLD_ON {
                        let r = ui.radio(p["holdOn"].as_str() == Some(k), l);
                        push(&mut elems, format!("{pre}.holdOn.{k}"), &r, l);
                        if r.clicked() {
                            p["holdOn"] = json!(k);
                        }
                    }
                    if matches!(p["holdOn"].as_str(), Some("sourceTimecode" | "sequenceTime")) {
                        text(ui, &mut elems, pre, p, "timecode", "Timecode:", 120.0);
                    }
                    check(ui, &mut elems, pre, p, "holdFilters", "Hold Filters", true);
                });
            }
            "clip.fieldOptions" => {
                check(ui, &mut elems, pre, p, "reverseFieldDominance", "Reverse Field Dominance", true);
                ui.label(RichText::new("Processing Options").strong());
                for (k, l) in [("none", "None"), ("alwaysDeinterlace", "Always Deinterlace"), ("flickerRemoval", "Flicker Removal")] {
                    let r = ui.radio(p["processing"].as_str() == Some(k), l);
                    push(&mut elems, format!("{pre}.processing.{k}"), &r, l);
                    if r.clicked() {
                        p["processing"] = json!(k);
                    }
                }
            }
            "clip.speedDuration" => {
                number(ui, &mut elems, pre, p, "speed", "Speed:", 0.01..=100_000.0, " %");
                // Duration follows the speed: the media shown stays the same
                let (dur, was, fps) =
                    (d.info["duration"].as_i64().unwrap_or(0), d.info["speed"].as_f64().unwrap_or(1.0), d.info["fps"].as_f64().unwrap_or(24.0));
                let now = p["speed"].as_f64().unwrap_or(100.0) / 100.0;
                if dur > 0 && now > 0.0 {
                    let rate = filmcraft_time::FrameRate::from_f64(if fps > 0.0 { fps } else { 24.0 });
                    let ticks = filmcraft_time::Tick((dur as f64 * was / now).round() as i64);
                    ui.horizontal(|ui| {
                        ui.label("Duration:");
                        ui.label(RichText::new(filmcraft_time::format_timecode_frames(rate.frame_at(ticks), rate, false)).monospace());
                    });
                }
                check(ui, &mut elems, pre, p, "reverse", "Reverse Speed", true);
                check(ui, &mut elems, pre, p, "ripple", "Ripple Edit, Shifting Trailing Clips", true);
                ui.label(RichText::new("Time Interpolation").strong());
                for m in filmcraft_project::TimeInterpolation::ALL {
                    let r = ui.radio(p["interpolation"].as_str() == Some(m.name()), m.label());
                    push(&mut elems, format!("{pre}.interpolation.{}", m.name()), &r, m.label());
                    if r.clicked() {
                        p["interpolation"] = json!(m.name());
                    }
                }
            }
            "file.closeProject" => {
                ui.label(format!("Save changes to “{project_name}” before closing?"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let r = ui.button("Cancel");
                    push(&mut elems, format!("{pre}.cancel"), &r, "Cancel");
                    if r.clicked() {
                        action = Some("cancel");
                    }
                    let r = ui.button("Don't Save");
                    push(&mut elems, format!("{pre}.dontSave"), &r, "Don't Save");
                    if r.clicked() {
                        action = Some("dontSave");
                    }
                    let r = ui.add_enabled(can_save, egui::Button::new("Save"));
                    push(&mut elems, format!("{pre}.save"), &r, "Save");
                    if r.clicked() {
                        action = Some("save");
                    }
                });
                return;
            }
            _ => {}
        }
        if !d.error.is_empty() {
            ui.colored_label(egui::Color32::from_rgb(0xe0, 0x60, 0x60), &d.error);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let r = ui.button("Cancel");
            push(&mut elems, format!("{pre}.cancel"), &r, "Cancel");
            if r.clicked() {
                action = Some("cancel");
            }
            let r = ui.button("OK");
            push(&mut elems, format!("{pre}.ok"), &r, "OK");
            if r.clicked() {
                action = Some("ok");
            }
        });
    });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some("cancel");
    }
    if action.is_some() {
        ctx.data_mut(|d| d.remove::<bool>(egui::Id::new("nest-name-focused")));
    }
    match action {
        Some("cancel") => {
            app.ui.clip_dialog = None;
            return;
        }
        Some("ok") => {
            if d.command == "clip.audioChannels" && !app.audio_channels_draft.as_ref().is_some_and(|context| context.matches(app, &d)) {
                d.error = "The project or source changed. Reopen Audio Channels before applying.".into();
                app.ui.clip_dialog = Some(d);
                return;
            }
            let params = finish(&d);
            match app.session.execute(&d.command, params) {
                Ok(_) => {
                    app.ui.clip_dialog = None;
                    return;
                }
                Err(e) => d.error = e.to_string(),
            }
        }
        Some("dontSave") | Some("save") => {
            if action == Some("save")
                && let Err(e) = app.session.execute("file.save", json!({}))
            {
                app.ui.status = e.to_string();
                app.ui.clip_dialog = None;
                return;
            }
            if let Err(e) = app.session.execute("file.closeProject", json!({"force": true})) {
                app.ui.status = e.to_string();
            }
            app.ui.clip_dialog = None;
            return;
        }
        _ => {}
    }
    app.ui.clip_dialog = Some(d);
}

/// The parameters sent on OK (dialog-only fields dropped or converted).
fn finish(d: &ClipDialogDraft) -> Value {
    let mut p = d.params.clone();
    match d.command.as_str() {
        "file.newOfflineFile" => {
            if p["name"].as_str().is_some_and(str::is_empty) {
                p.as_object_mut().map(|m| m.remove("name"));
            }
        }
        "clip.modifyTimecode" if p["reset"].as_bool() == Some(true) => {
            p.as_object_mut().map(|m| m.remove("timecode"));
        }
        // the timecode field only applies to Source Timecode / Sequence Time
        "clip.frameHoldOptions" if !matches!(p["holdOn"].as_str(), Some("sourceTimecode" | "sequenceTime")) => {
            p.as_object_mut().map(|m| m.remove("timecode"));
        }
        _ => {}
    }
    p
}
