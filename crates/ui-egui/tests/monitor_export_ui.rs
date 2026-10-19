//! Existing monitor camera/marker buttons act on the picture displayed by that monitor.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::{FrameRequest, Generator, MediaInfo, MediaSource, generators::GeneratorSource};
use filmcraft_project::{ItemId, Label, Project, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use filmcraft_ui_egui::{
    FilmcraftApp,
    control::ControlRequest,
    dock::PanelKind,
    pickers::{PickerKind, PickerOutcome, PickerReply},
};
use serde_json::{Value, json};

struct Driver {
    h: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    picks: Arc<Mutex<VecDeque<Option<String>>>>,
    suggestions: Arc<Mutex<Vec<String>>>,
    source: ItemId,
    dir: PathBuf,
}
impl Driver {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/monitor-export-tests").join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::default();
        let mut p = Project::new("Monitor export UI tests");
        let rate = FrameRate::FPS_25;
        let duration = Tick::from_units(2, 1);
        let source = filmcraft_engine::demo::add_generator(
            &mut p,
            &s.media,
            GeneratorSource::new(Generator::ColorMatte { color: [0.0, 0.6, 0.0, 1.0] }, 16, 8, rate, duration),
            "Source clip",
            Label::Iris,
            None,
        );
        let program_media = filmcraft_engine::demo::add_generator(
            &mut p,
            &s.media,
            GeneratorSource::new(Generator::ColorMatte { color: [0.7, 0.0, 0.0, 1.0] }, 16, 8, rate, duration),
            "Program clip",
            Label::Iris,
            None,
        );
        let program = p.new_sequence("Program", SequenceSettings { width: 16, height: 8, frame_rate: rate, ..Default::default() }, 1, 1, None);
        let item = p.make_track_item(program_media, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, duration), rate).unwrap();
        p.sequence_mut(program).unwrap().video_tracks[0].items.push(item);
        s.project = Arc::new(p);
        s.path = Some(dir.join("project.fcproj").to_string_lossy().to_string());
        s.state.active_sequence = Some(program);
        s.state.open_sequences = vec![program];
        s.set_playhead(rate.tick_of(3));
        s.execute("source.open", json!({"item": source.0})).unwrap();
        s.execute("source.setPlayhead", json!({"frame": 12})).unwrap();
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(s).with_control(rx);
        let picks = Arc::new(Mutex::new(VecDeque::new()));
        let suggestions = Arc::new(Mutex::new(Vec::new()));
        let (queue, names) = (picks.clone(), suggestions.clone());
        app.hooks.pick_save_as = Some(Box::new(move |filter, exts, suggested| {
            assert!(filter.contains("Still image"));
            assert_eq!(exts, &["png", "tif", "tiff", "bmp"]);
            names.lock().unwrap().push(suggested.to_string());
            queue.lock().unwrap().pop_front().flatten()
        }));
        let h = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { h, tx, picks, suggestions, source, dir };
        d.frames(4);
        d
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
        let result = self.call(method, params.clone());
        assert_eq!(result["ok"], true, "{method} {params}: {result}");
        result["result"].clone()
    }
    fn pick(&self, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        self.picks.lock().unwrap().push_back(Some(path.to_string_lossy().to_string()));
        path
    }
    fn delayed_picker(&mut self) -> Arc<Mutex<Option<PickerReply>>> {
        let held = Arc::new(Mutex::new(None));
        let output = held.clone();
        self.h.state_mut().hooks.async_picker = Some(Box::new(move |request, reply| {
            assert_eq!(request.kind, PickerKind::Save);
            assert_eq!(request.extensions, ["png", "tif", "tiff", "bmp"]);
            assert!(output.lock().unwrap().replace(reply).is_none());
            Ok(())
        }));
        held
    }
    fn finish_picker(&mut self, held: &Arc<Mutex<Option<PickerReply>>>, result: PickerOutcome) {
        held.lock().unwrap().take().unwrap()(result);
        self.frames(4);
        assert!(!self.h.state().picker_pending());
    }
    fn source_pixels(&self, item: ItemId, time: Tick) -> Vec<u8> {
        let session = &self.h.state().session;
        let provider = session.media.full_res_provider(session.project.clone(), session.services.clone());
        filmcraft_render::render_item(&session.project, item, time, 1.0, &provider).unwrap().to_rgba8()
    }
    fn time_coded_source(&mut self, item: ItemId) {
        let info = self.h.state().session.media.cached(item).unwrap().info().clone();
        self.h.state().session.media.insert(item, Arc::new(TimeFrames(info)));
    }
    fn rgba(path: &Path) -> Vec<u8> {
        filmcraft_media::open_bytes(path.to_str().unwrap(), Arc::from(std::fs::read(path).unwrap()), &[])
            .unwrap()
            .video_frame(FrameRequest::full(Tick::ZERO))
            .unwrap()
            .to_rgba8()
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn monitor_camera_buttons_choose_a_path_and_export_their_own_picture() {
    let mut d = Driver::new();
    let program_time = d.h.state().session.playhead();
    let source_time = d.h.state().session.state.source_playhead;
    let path = d.pick("source.png");
    d.h.state_mut().ui.focused = PanelKind::Program;
    d.ok("ui.click", json!({"id": "source.transport.exportFrame"}));
    let provider = d.h.state().session.media.full_res_provider(d.h.state().session.project.clone(), d.h.state().session.services.clone());
    let expected = filmcraft_render::render_item(&d.h.state().session.project, d.source, source_time, 1.0, &provider).unwrap().to_rgba8();
    assert_eq!(Driver::rgba(&path), expected);
    let path = d.pick("program.bmp");
    d.h.state_mut().ui.focused = PanelKind::Source;
    d.ok("ui.click", json!({"id": "program.transport.exportFrame"}));
    let app = d.h.state();
    let expected = app.session.render_program(1.0).unwrap().to_rgba8();
    assert_eq!(Driver::rgba(&path), expected);
    assert_eq!(app.session.playhead(), program_time);
    assert_eq!(app.session.state.source_playhead, source_time);
    assert_eq!(*d.suggestions.lock().unwrap(), ["Source clip.Still001.png", "Program.Still001.png"]);
}

#[test]
fn camera_cancellation_and_missing_picker_do_not_create_a_default_file() {
    let mut d = Driver::new();
    d.picks.lock().unwrap().push_back(None);
    d.ok("ui.click", json!({"id": "source.transport.exportFrame"}));
    assert_eq!(std::fs::read_dir(&d.dir).unwrap().count(), 0);
    d.h.state_mut().hooks.pick_save_as = None;
    let result = d.call("ui.menu.invoke", json!({"id": "source.exportFrame"}));
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().contains("save dialog"));
    assert_eq!(std::fs::read_dir(&d.dir).unwrap().count(), 0);
    let path = d.dir.join("direct.png");
    d.ok("ui.menu.invoke", json!({"id": "source.exportFrame", "params": {"path": path.to_string_lossy()}}));
    assert!(path.exists(), "a direct control request can export with an explicit path");
}

#[test]
fn source_marker_button_and_m_key_add_source_markers_and_program_button_keeps_program_target() {
    let mut d = Driver::new();
    let program_time = d.h.state().session.playhead();
    let source_time = d.h.state().session.state.source_playhead;
    d.ok("ui.click", json!({"id": "source.transport.markers.add"}));
    d.h.state_mut().ui.focused = PanelKind::Source;
    d.ok("ui.key", json!({"key": "M"}));
    let source_markers = filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.source).unwrap().markers;
    assert_eq!(source_markers.len(), 2);
    assert!(source_markers.iter().all(|m| m.start == source_time));
    assert!(d.h.state().session.active_sequence().unwrap().markers.is_empty());
    d.ok("ui.click", json!({"id": "program.transport.markers.add"}));
    assert_eq!(d.h.state().session.active_sequence().unwrap().markers.len(), 1);
    assert_eq!(d.h.state().session.active_sequence().unwrap().markers[0].start, program_time);
    assert_eq!(filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.source).unwrap().markers, source_markers);
}

#[test]
fn export_shortcut_uses_source_with_source_focus_even_without_program() {
    let mut d = Driver::new();
    d.h.state_mut().session.shortcuts.load_preset(filmcraft_engine::shortcuts::PREMIERE_PRESET).unwrap();
    d.h.state_mut().session.state.active_sequence = None;
    d.h.state_mut().session.state.open_sequences.clear();
    d.h.state_mut().ui.focused = PanelKind::Source;
    let path = d.pick("shortcut.png");
    d.ok("ui.key", json!({"key": "Shift+E"}));
    assert!(path.exists());
    assert_eq!(*d.suggestions.lock().unwrap(), ["Source clip.Still001.png"]);
}

#[test]
fn hostile_camera_parameters_return_errors_before_opening_a_picker_or_writing() {
    let mut d = Driver::new();
    let before = serde_json::to_value(&d.h.state().session.project).unwrap();
    for id in ["source.exportFrame", "file.exportFrame"] {
        for params in [json!(null), json!([]), json!(7), json!({"format": []}), json!({"import": "true"})] {
            let result = d.call("ui.menu.invoke", json!({"id": id, "params": params}));
            assert_eq!(result["ok"], false, "{result}");
            assert!(result["error"].as_str().unwrap().contains("must be"), "{result}");
        }
    }
    assert!(d.suggestions.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&d.dir).unwrap().count(), 0);
    assert_eq!(serde_json::to_value(&d.h.state().session.project).unwrap(), before);
}

#[test]
fn picker_filename_extension_selects_encoding_even_when_another_format_was_suggested() {
    let mut d = Driver::new();
    let path = d.pick("chosen.bmp");
    d.ok("ui.menu.invoke", json!({"id": "source.exportFrame", "params": {"format": "png"}}));
    assert_eq!(&std::fs::read(&path).unwrap()[..2], b"BM");
    assert!(!Driver::rgba(&path).is_empty());
}

#[test]
fn source_marker_menu_native_inbox_and_navigation_shortcuts_work_without_program() {
    let mut d = Driver::new();
    d.h.state_mut().session.state.active_sequence = None;
    d.h.state_mut().session.state.open_sequences.clear();
    d.h.state_mut().ui.focused = PanelKind::Source;
    d.ok("ui.menu.invoke", json!({"id": "markers.add", "params": {"frame": 7}}));
    let (tx, rx) = channel();
    d.h.state_mut().command_inbox = Some(rx);
    tx.send("markers.add".into()).unwrap();
    d.frames(2);
    let markers = filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.source).unwrap().markers;
    assert_eq!(markers.len(), 2);
    d.ok("engine.execute", json!({"command": "source.setPlayhead", "params": {"frame": 7}}));
    d.ok("ui.key", json!({"key": "Shift+M"}));
    assert_eq!(d.h.state().session.state.source_playhead, FrameRate::FPS_25.tick_of(12));
    d.ok("ui.menu.invoke", json!({"id": "markers.goPrev"}));
    assert_eq!(d.h.state().session.state.source_playhead, FrameRate::FPS_25.tick_of(7));
    let items = d.ok("ui.menu.list", json!({}));
    for id in ["markers.add", "markers.goNext", "markers.goPrev", "markers.clearCurrent", "markers.clearAll"] {
        assert_eq!(items.as_array().unwrap().iter().find(|it| it["id"] == id).unwrap()["enabled"], true);
    }
    d.ok("ui.menu.invoke", json!({"id": "markers.clearCurrent"}));
    assert_eq!(filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.source).unwrap().markers.len(), 1);
    d.ok("ui.menu.invoke", json!({"id": "markers.clearAll"}));
    assert!(filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.source).unwrap().markers.is_empty());
    d.ok("ui.menu.invoke", json!({"id": "edit.undo"}));
    assert_eq!(filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.source).unwrap().markers.len(), 1);
}

#[test]
fn malformed_explicit_marker_targets_cannot_modify_either_monitor() {
    let mut d = Driver::new();
    d.ok("ui.menu.invoke", json!({"id": "markers.add", "params": {"target": "source"}}));
    d.ok("ui.menu.invoke", json!({"id": "markers.add", "params": {"target": "program"}}));
    let before = serde_json::to_value(&d.h.state().session.project).unwrap();
    d.h.state_mut().ui.focused = PanelKind::Source;
    for id in ["markers.add", "markers.goNext", "markers.goPrev", "markers.clearCurrent", "markers.clearAll"] {
        for target in [json!("bogus"), json!(7), json!(null), json!([])] {
            let result = d.call("ui.menu.invoke", json!({"id": id, "params": {"target": target}}));
            assert_eq!(result["ok"], false, "{result}");
            assert!(result["error"].as_str().unwrap().contains("marker target"), "{result}");
            assert_eq!(serde_json::to_value(&d.h.state().session.project).unwrap(), before);
        }
    }
}

/// A small real media source whose pixels distinguish each requested frame.
struct TimeFrames(MediaInfo);
impl MediaSource for TimeFrames {
    fn info(&self) -> &MediaInfo {
        &self.0
    }
    fn video_frame(&self, request: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        let index = self.0.frame_rate().frame_at(request.time).rem_euclid(16) as u8;
        Ok(Arc::new(VideoFrame::rgba8(16, 8, [index * 16, 96, 192, 255].repeat(16 * 8))))
    }
    fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        Ok(AudioBuffer::silence(sample_rate, 2, frames))
    }
}

#[test]
fn delayed_source_save_keeps_clicked_item_and_exact_time_after_source_changes_and_closes() {
    let mut d = Driver::new();
    d.time_coded_source(d.source);
    let original_time = d.h.state().session.state.source_playhead;
    let expected = d.source_pixels(d.source, original_time);
    let original_items = d.h.state().session.project.items.len();
    let held = d.delayed_picker();
    let path = d.dir.join("captured-source.BMP");
    let result = d.ok("ui.menu.invoke", json!({"id": "source.exportFrame", "params": {"format": "png", "import": true}}));
    assert_eq!(result["pending"], true);
    assert!(!path.exists());
    let program = d.h.state().session.state.active_sequence.unwrap();
    d.ok("engine.execute", json!({"command": "source.open", "params": {"item": program.0}}));
    d.ok("engine.execute", json!({"command": "source.setPlayhead", "params": {"frame": 25}}));
    d.h.state_mut().session.state.source_item = None;
    d.h.state_mut().ui.focused = PanelKind::Program;
    let program_time = d.h.state().session.playhead();
    d.finish_picker(&held, Ok(vec![path.to_string_lossy().into_owned()]));
    assert_eq!(&std::fs::read(&path).unwrap()[..2], b"BM", "the chosen extension defines the encoding");
    assert_eq!(Driver::rgba(&path), expected, "export must use the clicked frame, not the new Source time");
    assert_eq!(d.h.state().session.project.items.len(), original_items + 1);
    assert_eq!(d.h.state().session.state.source_item, None);
    assert_eq!(d.h.state().session.state.source_playhead, FrameRate::FPS_25.tick_of(25));
    assert_eq!(d.h.state().session.playhead(), program_time);
}

#[test]
fn delayed_program_save_keeps_clicked_sequence_and_time_after_active_sequence_changes() {
    let mut d = Driver::new();
    let original = d.h.state().session.state.active_sequence.unwrap();
    let media = d.h.state().session.project.sequence(original).unwrap().video_tracks[0].items[0].item;
    d.time_coded_source(media);
    let other = Arc::make_mut(&mut d.h.state_mut().session.project).new_sequence(
        "Other",
        SequenceSettings { width: 16, height: 8, frame_rate: FrameRate::FPS_25, ..Default::default() },
        1,
        1,
        None,
    );
    let expected = d.h.state().session.render_program(1.0).unwrap().to_rgba8();
    let held = d.delayed_picker();
    let path = d.dir.join("captured-program.tiff");
    d.h.state_mut().ui.focused = PanelKind::Source;
    d.ok("ui.menu.invoke", json!({"id": "file.exportFrame", "params": {"target": "program"}}));
    d.ok("engine.execute", json!({"command": "sequence.open", "params": {"item": other.0}}));
    d.h.state_mut().session.set_playhead(FrameRate::FPS_25.tick_of(19));
    let source_time = d.h.state().session.state.source_playhead;
    d.finish_picker(&held, Ok(vec![path.to_string_lossy().into_owned()]));
    assert_eq!(Driver::rgba(&path), expected);
    assert_eq!(d.h.state().session.state.active_sequence, Some(other));
    assert_eq!(d.h.state().session.playhead(), FrameRate::FPS_25.tick_of(19));
    assert_eq!(d.h.state().session.state.source_playhead, source_time);
}

#[test]
fn delayed_frame_save_rejects_project_edits_or_replacement_without_writing_or_importing() {
    for replace in [false, true] {
        let mut d = Driver::new();
        let held = d.delayed_picker();
        let path = d.dir.join("discarded.png");
        d.ok("ui.menu.invoke", json!({"id": "source.exportFrame", "params": {"import": true}}));
        if replace {
            d.ok("ui.menu.invoke", json!({"id": "file.newProject", "params": {}}));
        } else {
            Arc::make_mut(&mut d.h.state_mut().session.project).name = "Edited while choosing".into();
        }
        let items = d.h.state().session.project.items.len();
        d.finish_picker(&held, Ok(vec![path.to_string_lossy().into_owned()]));
        assert!(!path.exists());
        assert_eq!(d.h.state().session.project.items.len(), items);
        assert!(d.h.state().ui.status.contains("project changed"), "{}", d.h.state().ui.status);
        if !replace {
            assert!(d.h.state().ui.status.contains("retry Export Frame"));
        }
    }
}

#[test]
fn delayed_camera_cancel_writes_nothing_and_unknown_format_reports_an_error() {
    let mut d = Driver::new();
    let held = d.delayed_picker();
    d.ok("ui.menu.invoke", json!({"id": "source.exportFrame"}));
    d.finish_picker(&held, Ok(vec![]));
    assert_eq!(std::fs::read_dir(&d.dir).unwrap().count(), 0);
    let path = d.dir.join("unsupported.gif");
    d.ok("ui.menu.invoke", json!({"id": "source.exportFrame"}));
    d.finish_picker(&held, Ok(vec![path.to_string_lossy().into_owned()]));
    assert!(!path.exists());
    assert!(d.h.state().ui.status.contains("format must be png, tiff or bmp"));
}

#[test]
fn source_only_file_export_menu_is_enabled_and_uses_source() {
    let mut d = Driver::new();
    d.h.state_mut().session.state.active_sequence = None;
    d.h.state_mut().session.state.open_sequences.clear();
    d.h.state_mut().ui.focused = PanelKind::Source;
    let menu = d.ok("ui.menu.list", json!({}));
    assert_eq!(menu.as_array().unwrap().iter().find(|item| item["id"] == "file.exportFrame").unwrap()["enabled"], true);
    let expected = d.source_pixels(d.source, d.h.state().session.state.source_playhead);
    let path = d.pick("source-menu.png");
    d.ok("ui.menu.invoke", json!({"id": "file.exportFrame"}));
    assert_eq!(Driver::rgba(&path), expected);
}

#[test]
fn fullscreen_marker_and_export_keys_use_program_with_source_focus_preserved() {
    let mut d = Driver::new();
    d.h.state_mut().session.shortcuts.load_preset(filmcraft_engine::shortcuts::PREMIERE_PRESET).unwrap();
    d.h.state_mut().ui.focused = PanelKind::Source;
    let source_time = d.h.state().session.state.source_playhead;
    let program_time = d.h.state().session.playhead();
    let expected = d.h.state().session.render_program(1.0).unwrap().to_rgba8();
    d.ok("ui.key", json!({"key": "F11"}));
    assert!(d.h.state().ui.program_fullscreen.is_some());
    assert_eq!(d.h.state().ui.focused, PanelKind::Source);
    d.ok("ui.key", json!({"key": "M"}));
    assert_eq!(d.h.state().session.active_sequence().unwrap().markers[0].start, program_time);
    assert!(filmcraft_engine::clip_ops::source_view(&d.h.state().session, d.source).unwrap().markers.is_empty());
    let path = d.pick("fullscreen-program.png");
    d.ok("ui.key", json!({"key": "Shift+E"}));
    assert_eq!(Driver::rgba(&path), expected);
    let menu = d.ok("ui.menu.list", json!({}));
    assert_eq!(menu.as_array().unwrap().iter().find(|item| item["id"] == "markers.goNext").unwrap()["enabled"], true);
    let path = d.pick("fullscreen-menu.png");
    d.ok("ui.menu.invoke", json!({"id": "file.exportFrame"}));
    assert_eq!(Driver::rgba(&path), expected);
    assert_eq!(d.h.state().session.state.source_playhead, source_time);
    d.ok("ui.key", json!({"key": "Escape"}));
    assert_eq!(d.h.state().ui.focused, PanelKind::Source);
}

#[test]
fn source_marker_navigation_stops_source_clock_and_preserves_program_playback() {
    let mut d = Driver::new();
    for frame in [7, 20] {
        d.ok("ui.menu.invoke", json!({"id": "markers.add", "params": {"target": "source", "frame": frame}}));
    }
    d.ok("engine.execute", json!({"command": "source.setPlayhead", "params": {"frame": 7}}));
    let program_time = d.h.state().session.playhead();
    d.ok("ui.playback", json!({"target": "source", "action": "play"}));
    assert!(d.h.state().source_is_playing());
    let rejected = d.call("ui.menu.invoke", json!({"id": "markers.goNext", "params": {"target": "bogus"}}));
    assert_eq!(rejected["ok"], false);
    assert!(d.h.state().source_is_playing(), "a rejected target preserves playback");
    d.ok("ui.menu.invoke", json!({"id": "markers.goNext", "params": {"target": "source"}}));
    assert!(!d.h.state().source_is_playing());
    assert_eq!(d.h.state().session.state.source_playhead, FrameRate::FPS_25.tick_of(20));
    d.frames(12);
    assert_eq!(d.h.state().session.state.source_playhead, FrameRate::FPS_25.tick_of(20));
    assert_eq!(d.h.state().session.playhead(), program_time);
    d.ok("ui.playback", json!({"target": "program", "action": "play"}));
    assert!(d.h.state().program_is_playing());
    d.ok("ui.menu.invoke", json!({"id": "markers.goPrev", "params": {"target": "source"}}));
    assert!(d.h.state().program_is_playing());
    assert_eq!(d.h.state().session.state.source_playhead, FrameRate::FPS_25.tick_of(7));
    d.frames(12);
    assert_eq!(d.h.state().session.state.source_playhead, FrameRate::FPS_25.tick_of(7));
}

#[test]
fn camera_tooltip_follows_the_active_export_shortcut_preset() {
    let mut d = Driver::new();
    for (preset, expected) in [(None, "Export Frame"), (Some(filmcraft_engine::shortcuts::PREMIERE_PRESET), "Export Frame (Shift+E)")] {
        if let Some(preset) = preset {
            d.h.state_mut().session.shortcuts.load_preset(preset).unwrap();
        }
        d.frames(3);
        let elements = d.ok("ui.elements", json!({}));
        for id in ["source.transport.exportFrame", "program.transport.exportFrame"] {
            let camera = elements.as_array().unwrap().iter().find(|element| element["id"] == id).unwrap();
            assert_eq!(camera["label"], expected);
        }
    }
}

#[test]
fn delayed_frame_save_survives_file_close_but_rejects_project_replacement() {
    for source in [false, true] {
        for replace in [false, true] {
            let mut d = Driver::new();
            let sequence = d.h.state().session.state.active_sequence.unwrap();
            let source_time = d.h.state().session.state.source_playhead;
            let program_time = d.h.state().session.playhead();
            let expected = if source { d.source_pixels(d.source, source_time) } else { d.h.state().session.render_program(1.0).unwrap().to_rgba8() };
            let original = d.h.state().session.project.clone();
            let media = d.h.state().session.media.clone();
            let held = d.delayed_picker();
            let path = d.dir.join("captured-after-close.png");
            d.ok(
                "ui.menu.invoke",
                json!({"id": if source { "source.exportFrame" } else { "file.exportFrame" }, "params": {"target": if source { "source" } else { "program" }}}),
            );
            d.ok("ui.menu.invoke", json!({"id": if replace { "file.newProject" } else { "file.close" }}));
            if !replace {
                assert!(Arc::ptr_eq(&original, &d.h.state().session.project), "File Close closes a sequence, not its project");
                assert!(Arc::ptr_eq(&media, &d.h.state().session.media));
                assert!(d.h.state().session.state.active_sequence.is_none());
                assert!(d.h.state().session.state.open_sequences.is_empty());
                assert_eq!(d.h.state().session.state.playheads.get(&sequence), Some(&program_time));
                assert_eq!(d.h.state().session.state.source_playhead, source_time);
            }
            let before_reply = d.h.state().session.project.clone();
            let revision = d.h.state().session.revision;
            d.finish_picker(&held, Ok(vec![path.to_string_lossy().into_owned()]));
            if replace {
                assert!(!path.exists(), "a replacement still invalidates the old captured frame");
                assert!(d.h.state().ui.status.contains("project changed"));
            } else {
                assert!(path.exists(), "a retained sequence can still supply its captured frame after File Close");
                assert_eq!(Driver::rgba(&path), expected);
                assert!(d.h.state().session.state.active_sequence.is_none());
                assert_eq!(d.h.state().session.state.source_playhead, source_time);
            }
            assert!(Arc::ptr_eq(&before_reply, &d.h.state().session.project));
            assert_eq!(d.h.state().session.revision, revision);
        }
    }
}
