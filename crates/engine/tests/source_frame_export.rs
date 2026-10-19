//! Monitor frame export uses the selected item's exact frame and refuses deferred stand-ins.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use filmcraft_engine::Session;
use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::{FrameRequest, Generator, MediaInfo, MediaSource, generators::GeneratorSource};
use filmcraft_project::{ItemId, ItemKind, Label, Project, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use serde_json::json;

fn warp_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn enable_source_stabilizer(f: &mut Fixture) {
    f.source(f.sequence, 12);
    Arc::make_mut(&mut f.s.project).sequence_mut(f.sequence).unwrap().video_tracks[0].items[0]
        .effects
        .insert(0, effect("warp_stabilizer", &[("framing", filmcraft_project::ParamValue::Choice(0))]));
}

struct ExactFrames {
    info: MediaInfo,
    pending: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    defer_frame: Arc<AtomicI64>,
    reads: Arc<AtomicU64>,
}
impl MediaSource for ExactFrames {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, req: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let frame_index = self.info.frame_rate().frame_at(req.time);
        if self.defer_frame.compare_exchange(frame_index, i64::MIN, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            filmcraft_media::pending::mark();
            return Err(filmcraft_media::MediaError::Io("synthetic deferred analysis read".into()));
        }
        if self.pending.swap(false, Ordering::SeqCst) {
            filmcraft_media::pending::mark();
            return Err(filmcraft_media::MediaError::Io("synthetic deferred read".into()));
        }
        if self.failed.load(Ordering::SeqCst) {
            return Err(filmcraft_media::MediaError::Io("synthetic decode failure".into()));
        }
        let frame = self.info.frame_rate().frame_at(req.time).rem_euclid(16) as u8;
        let rgba = [frame * 16, 96, 192, 255];
        Ok(Arc::new(VideoFrame::rgba8(16, 8, rgba.repeat(16 * 8))))
    }
    fn audio(&self, _start: i64, frames: usize, sr: u32) -> filmcraft_media::Result<AudioBuffer> {
        Ok(AudioBuffer::silence(sr, 2, frames))
    }
}

struct Fixture {
    s: Session,
    video: ItemId,
    subclip: ItemId,
    sequence: ItemId,
    program: ItemId,
    pending: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    defer_frame: Arc<AtomicI64>,
    reads: Arc<AtomicU64>,
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/source-export-tests").join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::default();
        let mut p = Project::new("Source export tests");
        let rate = FrameRate::FPS_25;
        let duration = Tick::from_units(2, 1);
        let generator = GeneratorSource::new(Generator::BarsAndTone, 16, 8, rate, duration);
        let info = generator.info().clone();
        let video = filmcraft_engine::demo::add_generator(&mut p, &s.media, generator, "Source / frame", Label::Iris, None);
        let pending = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let defer_frame = Arc::new(AtomicI64::new(i64::MIN));
        let reads = Arc::new(AtomicU64::new(0));
        s.media.insert(
            video,
            Arc::new(ExactFrames { info, pending: pending.clone(), failed: failed.clone(), defer_frame: defer_frame.clone(), reads: reads.clone() }),
        );
        let subclip = p.add_item(
            "Subclip",
            Label::Iris,
            ItemKind::Subclip { parent: video, range: TimeRange::new(rate.tick_of(5), rate.tick_of(20)), restrict_trims: true },
            None,
        );
        let sequence = |p: &mut Project, name, length| {
            let id = p.new_sequence(name, SequenceSettings { width: 16, height: 8, frame_rate: rate, ..Default::default() }, 1, 1, None);
            let item = p.make_track_item(video, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, length), rate).unwrap();
            p.sequence_mut(id).unwrap().video_tracks[0].items.push(item);
            id
        };
        let program = sequence(&mut p, "Program", duration);
        let sequence = sequence(&mut p, "Sequence source", Tick::from_units(1, 1));
        s.project = Arc::new(p);
        s.path = Some(dir.join("project.fcproj").to_string_lossy().to_string());
        s.state.active_sequence = Some(program);
        s.state.open_sequences = vec![program];
        s.set_playhead(rate.tick_of(1));
        Self { s, video, subclip, sequence, program, pending, failed, defer_frame, reads, dir }
    }
    fn source(&mut self, item: ItemId, frame: i64) {
        self.s.execute("source.open", json!({"item": item.0})).unwrap();
        self.s.execute("source.setPlayhead", json!({"frame": frame})).unwrap();
    }
    fn rgba(&self, path: &Path) -> Vec<u8> {
        let bytes = Arc::from(std::fs::read(path).unwrap());
        filmcraft_media::open_bytes(path.to_str().unwrap(), bytes, &[]).unwrap().video_frame(FrameRequest::full(Tick::ZERO)).unwrap().to_rgba8()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn source_frame_export_matches_media_subclip_and_sequence_picture_in_all_still_formats() {
    for target in 0..3 {
        for format in ["png", "tiff", "bmp"] {
            let mut f = Fixture::new();
            let item = [f.video, f.subclip, f.sequence][target];
            f.source(item, 12);
            let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
            let expected = filmcraft_render::render_item(&f.s.project, item, f.s.state.source_playhead, 1.0, &provider).unwrap().to_rgba8();
            let program = f.s.playhead();
            let path = f.dir.join(format!("frame.{format}"));
            let r = f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy(), "format": format})).unwrap();
            assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(16), Some(8)));
            assert_eq!(f.rgba(&path), expected, "{target} {format} must save the Source picture at its own time");
            assert_eq!(f.s.playhead(), program);
        }
    }
}

#[test]
fn source_export_without_program_uses_free_source_filenames_and_can_import_the_still() {
    let mut f = Fixture::new();
    f.source(f.video, 12);
    f.s.state.active_sequence = None;
    f.s.state.open_sequences.clear();
    assert!(f.s.is_enabled("source.exportFrame"));
    let count = f.s.project.items.len();
    let first = f.s.execute("source.exportFrame", json!({"import": true})).unwrap();
    assert_eq!(Path::new(first["path"].as_str().unwrap()).file_name().unwrap(), "Source _ frame.Still001.png");
    assert_eq!(f.s.project.items.len(), count + 1);
    let original = std::fs::read(first["path"].as_str().unwrap()).unwrap();
    let second = f.s.execute("source.exportFrame", json!({"format": "tif"})).unwrap();
    assert_eq!(Path::new(second["path"].as_str().unwrap()).extension().unwrap(), "tiff");
    let third = f.s.execute("source.exportFrame", json!({})).unwrap();
    assert_eq!(Path::new(third["path"].as_str().unwrap()).file_name().unwrap(), "Source _ frame.Still002.png");
    assert_eq!(std::fs::read(first["path"].as_str().unwrap()).unwrap(), original);
}

#[test]
fn source_and_program_deferred_frames_do_not_write_or_import_stand_ins_and_retry_exactly() {
    for source in [true, false] {
        let mut f = Fixture::new();
        f.source(f.video, 12);
        let command = if source { "source.exportFrame" } else { "file.exportFrame" };
        let path = f.dir.join("pending.png");
        let before = serde_json::to_value(&f.s.project).unwrap();
        f.pending.store(true, Ordering::SeqCst);
        let error = f.s.execute(command, json!({"path": path.to_string_lossy(), "import": true})).unwrap_err();
        assert!(error.to_string().contains("still loading"), "{error}");
        assert!(!path.exists());
        assert_eq!(serde_json::to_value(&f.s.project).unwrap(), before);
        // An unrelated old pending flag must not turn a ready export into an error.
        filmcraft_media::pending::mark();
        f.s.execute(command, json!({"path": path.to_string_lossy()})).unwrap();
        let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
        let expected = if source {
            filmcraft_render::render_item(&f.s.project, f.video, f.s.state.source_playhead, 1.0, &provider).unwrap()
        } else {
            filmcraft_render::render_sequence(&f.s.project, f.program, f.s.playhead(), Default::default(), &provider)
        };
        assert_eq!(f.rgba(&path), expected.to_rgba8());
    }
}

#[test]
fn source_export_rejects_missing_audio_only_offline_invalid_format_and_write_errors() {
    let mut f = Fixture::new();
    assert!(!f.s.is_enabled("source.exportFrame"));
    assert!(f.s.execute("source.exportFrame", json!({"path": f.dir.join("missing.png").to_string_lossy()})).is_err());
    f.source(f.video, 12);
    for params in [json!({"format": "jpg"}), json!({"path": ""})] {
        assert!(f.s.execute("source.exportFrame", params).is_err());
    }
    let impossible = f.dir.join("missing directory/frame.png");
    assert!(f.s.execute("source.exportFrame", json!({"path": impossible.to_string_lossy()})).unwrap_err().to_string().contains("frame.png"));
    if let ItemKind::Media(m) = &mut Arc::make_mut(&mut f.s.project).item_mut(f.video).unwrap().kind {
        m.offline = true;
    }
    assert!(!f.s.is_enabled("source.exportFrame"));
    assert!(f.s.execute("source.exportFrame", json!({})).is_err());
    if let ItemKind::Media(m) = &mut Arc::make_mut(&mut f.s.project).item_mut(f.video).unwrap().kind {
        m.offline = false;
        m.info.video = None;
        m.info.kind = filmcraft_media::MediaKind::AudioOnly;
    }
    assert!(!f.s.is_enabled("source.exportFrame"));
    assert!(f.s.execute("source.exportFrame", json!({})).is_err());
}

#[test]
fn source_markers_use_source_media_time_and_subclip_parent_and_do_not_edit_program() {
    let mut f = Fixture::new();
    for item in [f.video, f.subclip, f.sequence] {
        f.source(item, 12);
        let before = f.s.project.sequence(f.program).unwrap().markers.clone();
        let r = f.s.execute("source.addMarker", json!({"name": "Source", "durationFrames": 2})).unwrap();
        let view = filmcraft_engine::clip_ops::source_view(&f.s, item).unwrap();
        let marker = view.markers.iter().find(|m| m.id.0 == r["marker"].as_u64().unwrap()).unwrap();
        assert_eq!(marker.start, f.s.state.source_playhead);
        assert_eq!(marker.duration, view.rate.tick_of(2));
        assert_eq!(f.s.project.sequence(f.program).unwrap().markers, before);
    }
    assert!(f.s.execute("source.addMarker", json!({"frame": i64::MAX})).is_err());
    assert!(f.s.execute("source.addMarker", json!({"durationFrames": -1})).is_err());
    let before = filmcraft_engine::clip_ops::source_view(&f.s, f.sequence).unwrap().markers;
    f.s.execute("markers.add", json!({"name": "Program"})).unwrap();
    assert_eq!(filmcraft_engine::clip_ops::source_view(&f.s, f.sequence).unwrap().markers, before);
    assert_eq!(f.s.project.sequence(f.program).unwrap().markers.len(), 1);
}

#[test]
fn source_and_program_frame_export_use_originals_when_proxy_preview_is_enabled() {
    let mut f = Fixture::new();
    let proxy_path = f.dir.join("proxy.png");
    let proxy = filmcraft_export::encode_still(filmcraft_export::Format::PngSequence, [0, 255, 0, 255].repeat(16 * 8), 16, 8).unwrap();
    std::fs::write(&proxy_path, proxy).unwrap();
    if let ItemKind::Media(m) = &mut Arc::make_mut(&mut f.s.project).item_mut(f.video).unwrap().kind {
        m.proxy = Some(filmcraft_project::MediaRef::File { path: proxy_path.to_string_lossy().to_string() });
    }
    f.s.media.set_use_proxies(true);
    f.source(f.video, 12);
    let preview_provider = f.s.media.provider(f.s.project.clone(), f.s.services.clone());
    let proxy_picture = filmcraft_render::render_item(&f.s.project, f.video, f.s.state.source_playhead, 1.0, &preview_provider).unwrap().to_rgba8();
    for source in [true, false] {
        let path = f.dir.join(if source { "original-source.png" } else { "original-program.png" });
        f.s.execute(if source { "source.exportFrame" } else { "file.exportFrame" }, json!({"path": path.to_string_lossy()})).unwrap();
        let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
        let expected = if source {
            filmcraft_render::render_item(&f.s.project, f.video, f.s.state.source_playhead, 1.0, &provider).unwrap()
        } else {
            filmcraft_render::render_sequence(&f.s.project, f.program, f.s.playhead(), Default::default(), &provider)
        };
        assert_eq!(f.rgba(&path), expected.to_rgba8());
        assert_ne!(f.rgba(&path), proxy_picture, "Export Frame must use originals despite proxy preview");
    }
}

#[test]
fn hostile_export_and_marker_parameters_return_errors_without_writes_or_project_changes() {
    let mut f = Fixture::new();
    f.source(f.video, 12);
    let before = serde_json::to_value(&f.s.project).unwrap();
    let no_object = [json!(null), json!([]), json!(7), json!("parameters")];
    for command in ["source.exportFrame", "file.exportFrame", "source.addMarker"] {
        for params in &no_object {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.s.execute(command, params.clone())));
            assert!(result.unwrap().is_err(), "{command} must reject {params}");
        }
    }
    for command in ["source.exportFrame", "file.exportFrame"] {
        for params in [json!({"path": 1}), json!({"format": []}), json!({"import": "true"})] {
            assert!(f.s.execute(command, params.clone()).is_err(), "{command} must reject {params}");
        }
    }
    for params in [
        json!({"frame": 1e50}),
        json!({"time": "now"}),
        json!({"durationFrames": []}),
        json!({"name": 1}),
        json!({"frame": -1}),
        json!({"time": i64::MIN}),
        json!({"durationFrames": i64::MAX}),
    ] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.s.execute("source.addMarker", params.clone())));
        assert!(result.unwrap().is_err(), "Source marker must reject {params}");
    }
    assert_eq!(std::fs::read_dir(&f.dir).unwrap().count(), 0);
    assert_eq!(serde_json::to_value(&f.s.project).unwrap(), before);
}

#[test]
fn sequence_exports_reject_used_offline_or_failed_frames_and_ignore_offscreen_media() {
    for source in [true, false] {
        let mut f = Fixture::new();
        f.source(f.sequence, 12);
        let command = if source { "source.exportFrame" } else { "file.exportFrame" };
        let path = f.dir.join("sequence.png");
        let before = f.s.project.items.len();
        if let ItemKind::Media(m) = &mut Arc::make_mut(&mut f.s.project).item_mut(f.video).unwrap().kind {
            m.offline = true;
        }
        let error = f.s.execute(command, json!({"path": path.to_string_lossy(), "import": true})).unwrap_err();
        assert!(error.to_string().contains("offline"), "{error}");
        assert!(!path.exists());
        assert_eq!(f.s.project.items.len(), before);
        if let ItemKind::Media(m) = &mut Arc::make_mut(&mut f.s.project).item_mut(f.video).unwrap().kind {
            m.offline = false;
        }
        f.failed.store(true, Ordering::SeqCst);
        let error = f.s.execute(command, json!({"path": path.to_string_lossy(), "import": true})).unwrap_err();
        assert!(error.to_string().contains("decode failure"), "{error}");
        assert!(!path.exists());
        assert_eq!(f.s.project.items.len(), before);
        f.failed.store(false, Ordering::SeqCst);
        let p = Arc::make_mut(&mut f.s.project);
        let mut offline = p.item(f.video).unwrap().kind.clone();
        if let ItemKind::Media(m) = &mut offline {
            m.offline = true;
        }
        let unused = p.add_item("Unused offline clip", Label::Iris, offline, None);
        let rate = FrameRate::FPS_25;
        let clip = p.make_track_item(unused, TrackKind::Video, Tick::from_units(2, 1), TimeRange::new(Tick::ZERO, Tick::from_units(1, 1)), rate).unwrap();
        p.sequence_mut(if source { f.sequence } else { f.program }).unwrap().video_tracks[0].items.push(clip);
        f.s.execute(command, json!({"path": path.to_string_lossy()})).unwrap();
        assert!(path.exists(), "only media used by the current frame affects frame export");
    }
}

#[test]
fn source_marker_navigation_and_clearing_respect_subclip_bounds_and_undo() {
    let mut f = Fixture::new();
    let rate = FrameRate::FPS_25;
    f.source(f.video, 1);
    for frame in [1, 7, 12, 20, 30] {
        f.s.execute("source.addMarker", json!({"frame": frame, "name": format!("Frame {frame}")})).unwrap();
    }
    let before = serde_json::to_value(&f.s.project).unwrap();
    let program_time = f.s.playhead();
    f.source(f.subclip, 7);
    f.s.execute("source.goNextMarker", json!({})).unwrap();
    assert_eq!(f.s.state.source_playhead, rate.tick_of(12));
    f.s.execute("source.goPrevMarker", json!({})).unwrap();
    assert_eq!(f.s.state.source_playhead, rate.tick_of(7));
    f.s.execute("source.goPrevMarker", json!({})).unwrap();
    assert_eq!(f.s.state.source_playhead, rate.tick_of(7), "navigation must not leave restricted subclip for parent frame 1");
    f.s.execute("source.clearCurrentMarker", json!({})).unwrap();
    assert_eq!(
        filmcraft_engine::clip_ops::source_view(&f.s, f.subclip).unwrap().markers.iter().map(|m| m.start).collect::<Vec<_>>(),
        [rate.tick_of(12), rate.tick_of(20)]
    );
    assert!(f.s.undo().is_some());
    assert_eq!(serde_json::to_value(&f.s.project).unwrap(), before);
    f.s.execute("source.clearMarkers", json!({})).unwrap();
    let media = f.s.project.item(f.video).unwrap().as_media().unwrap();
    assert_eq!(media.markers.iter().map(|m| m.start).collect::<Vec<_>>(), [rate.tick_of(1), rate.tick_of(30)]);
    assert_eq!(f.s.playhead(), program_time);
    assert!(f.s.project.sequence(f.program).unwrap().markers.is_empty());
    assert!(f.s.undo().is_some());
    assert_eq!(serde_json::to_value(&f.s.project).unwrap(), before);
    for id in ["source.goNextMarker", "source.goPrevMarker", "source.clearCurrentMarker", "source.clearMarkers"] {
        for params in [json!([]), json!(1)] {
            assert!(f.s.execute(id, params).is_err());
        }
    }
    f.s.state.source_item = None;
    for id in ["source.goNextMarker", "source.goPrevMarker", "source.clearCurrentMarker", "source.clearMarkers"] {
        assert!(!f.s.is_enabled(id));
        assert!(f.s.execute(id, json!({})).is_err());
    }
}

#[test]
fn export_rejects_dangling_or_cyclic_active_sources_and_ignores_them_offscreen() {
    for source in [true, false] {
        for broken in 0..3 {
            let mut f = Fixture::new();
            f.source(f.sequence, 12);
            let active = if source { f.sequence } else { f.program };
            let p = Arc::make_mut(&mut f.s.project);
            if broken == 0 {
                p.items.remove(&f.video);
            } else {
                let parent = if broken == 1 { ItemId(u64::MAX) } else { f.subclip };
                if let ItemKind::Subclip { parent: id, .. } = &mut p.item_mut(f.subclip).unwrap().kind {
                    *id = parent;
                }
                p.sequence_mut(active).unwrap().video_tracks[0].items[0].item = f.subclip;
            }
            let before = serde_json::to_value(&f.s.project).unwrap();
            let path = f.dir.join("damaged.png");
            let command = if source { "source.exportFrame" } else { "file.exportFrame" };
            assert!(f.s.execute(command, json!({"path": path.to_string_lossy(), "import": true})).is_err());
            assert!(!path.exists());
            assert_eq!(serde_json::to_value(&f.s.project).unwrap(), before);
            // A source not displayed at this time must not invalidate a healthy (blank) frame.
            Arc::make_mut(&mut f.s.project).sequence_mut(active).unwrap().video_tracks[0].items[0].start = Tick::from_units(2, 1);
            f.s.execute(command, json!({"path": path.to_string_lossy()})).unwrap();
            assert!(path.exists());
        }
    }
}

#[test]
fn export_bounds_nested_sequence_cycles_and_ignores_disabled_bad_layers() {
    for source in [true, false] {
        let mut f = Fixture::new();
        f.source(f.sequence, 12);
        let active = if source { f.sequence } else { f.program };
        Arc::make_mut(&mut f.s.project).sequence_mut(active).unwrap().video_tracks[0].items[0].item = active;
        let command = if source { "source.exportFrame" } else { "file.exportFrame" };
        let path = f.dir.join("cycle.png");
        let error = f.s.execute(command, json!({"path": path.to_string_lossy()})).unwrap_err();
        assert!(error.to_string().contains("nesting"), "{error}");
        assert!(!path.exists());
        Arc::make_mut(&mut f.s.project).sequence_mut(active).unwrap().video_tracks[0].enabled = false;
        f.s.execute(command, json!({"path": path.to_string_lossy()})).unwrap();
        assert!(path.exists());
    }
}

fn effect(id: &str, params: &[(&str, filmcraft_project::ParamValue)]) -> filmcraft_project::EffectInstance {
    let mut e = filmcraft_project::find_effect(id).unwrap().instance();
    for (name, value) in params {
        e.params.get_mut(*name).unwrap().value = value.clone();
    }
    e
}

#[test]
fn exports_allow_the_maximum_sequence_depth_but_report_deeper_nesting() {
    for depth in [filmcraft_render::MAX_NEST_DEPTH, filmcraft_render::MAX_NEST_DEPTH + 1] {
        let mut f = Fixture::new();
        let p = Arc::make_mut(&mut f.s.project);
        let rate = FrameRate::FPS_25;
        let mut child = f.video;
        for _ in 0..=depth {
            let sequence = p.new_sequence("Nested", SequenceSettings { width: 16, height: 8, frame_rate: rate, ..Default::default() }, 1, 0, None);
            let clip = p.make_track_item(child, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, Tick::from_units(1, 1)), rate).unwrap();
            p.sequence_mut(sequence).unwrap().video_tracks[0].items.push(clip);
            child = sequence;
        }
        f.source(child, 12);
        let path = f.dir.join("depth.png");
        let result = f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()}));
        if depth == filmcraft_render::MAX_NEST_DEPTH {
            assert!(result.is_ok(), "a media leaf under maximum supported nesting is valid: {result:?}");
            assert!(path.exists());
        } else {
            assert!(result.unwrap_err().to_string().contains("nesting"));
            assert!(!path.exists());
        }
    }
}

#[test]
fn export_checks_disabled_track_matte_dependencies_that_are_actually_rendered() {
    use filmcraft_project::ParamValue;
    let mut f = Fixture::new();
    f.source(f.sequence, 12);
    let p = Arc::make_mut(&mut f.s.project);
    let track = p.sequence(f.sequence).unwrap().video_tracks[0].clone();
    p.sequence_mut(f.sequence).unwrap().video_tracks.push(track);
    let q = p.sequence_mut(f.sequence).unwrap();
    q.video_tracks[1].enabled = false;
    q.video_tracks[1].items[0].item = ItemId(u64::MAX);
    let path = f.dir.join("matte.png");
    // Disabled invalid tracks are harmless until a visible effect actually samples them.
    f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()})).unwrap();
    std::fs::remove_file(&path).unwrap();
    Arc::make_mut(&mut f.s.project).sequence_mut(f.sequence).unwrap().video_tracks[0].items[0]
        .effects
        .insert(0, effect("track_matte", &[("matte", ParamValue::Choice(2))]));
    let count = f.s.project.items.len();
    let error = f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy(), "import": true})).unwrap_err();
    assert!(error.to_string().contains("unavailable"), "{error}");
    assert!(!path.exists());
    assert_eq!(f.s.project.items.len(), count);
}

#[test]
fn export_checks_nested_frames_requested_by_echo_and_posterize_time() {
    use filmcraft_project::ParamValue;
    for fx in ["echo", "posterize_time"] {
        let mut f = Fixture::new();
        f.s.set_playhead(FrameRate::FPS_25.tick_of(12));
        let p = Arc::make_mut(&mut f.s.project);
        let rate = FrameRate::FPS_25;
        let q = p.sequence_mut(f.sequence).unwrap();
        q.video_tracks[0].items[0].start = rate.tick_of(10);
        q.video_tracks[0].items[0].duration = rate.tick_of(15);
        let mut earlier = q.video_tracks[0].items[0].clone();
        earlier.id = filmcraft_project::ClipId(u64::MAX);
        earlier.item = ItemId(u64::MAX);
        earlier.start = Tick::ZERO;
        earlier.duration = rate.tick_of(10);
        q.video_tracks[0].items.push(earlier);
        let q = p.sequence_mut(f.program).unwrap();
        q.video_tracks[0].items[0].item = f.sequence;
        let params =
            if fx == "echo" { vec![("time", ParamValue::Float(-0.4)), ("count", ParamValue::Float(1.0))] } else { vec![("rate", ParamValue::Float(1.0))] };
        q.video_tracks[0].items[0].effects.insert(0, effect(fx, &params));
        let path = f.dir.join("temporal.png");
        let error = f.s.execute("file.exportFrame", json!({"path": path.to_string_lossy()})).unwrap_err();
        assert!(error.to_string().contains("unavailable"), "{fx}: {error}");
        assert!(!path.exists(), "an effect's missing alternate frame must not be exported");
    }
}

#[test]
fn export_collects_warp_analysis_worker_deferrals_and_retries_complete_analysis() {
    let _lock = warp_test_lock();
    use filmcraft_project::ParamValue;
    for source in [true, false] {
        filmcraft_render::vfx::clear_stabilizer_cache();
        let mut f = Fixture::new();
        f.source(f.sequence, 12);
        f.s.set_playhead(FrameRate::FPS_25.tick_of(12));
        let target = if source { f.sequence } else { f.program };
        Arc::make_mut(&mut f.s.project).sequence_mut(target).unwrap().video_tracks[0].items[0]
            .effects
            .insert(0, effect("warp_stabilizer", &[("framing", ParamValue::Choice(0)), ("result", ParamValue::Choice(1))]));
        // Current frame 12 is ready; only an analysis read for frame 1 on a worker defers.
        f.defer_frame.store(1, Ordering::SeqCst);
        let path = f.dir.join("stabilized.png");
        let command = if source { "source.exportFrame" } else { "file.exportFrame" };
        let before = serde_json::to_value(&f.s.project).unwrap();
        let error = f.s.execute(command, json!({"path": path.to_string_lossy(), "import": true})).unwrap_err();
        assert!(error.to_string().contains("still loading"), "{error}");
        assert!(!path.exists());
        assert_eq!(serde_json::to_value(&f.s.project).unwrap(), before);
        let reads = f.reads.load(Ordering::SeqCst);
        f.s.execute(command, json!({"path": path.to_string_lossy()})).unwrap();
        assert!(f.reads.load(Ordering::SeqCst) >= reads + 25, "incomplete analysis must be recomputed when all media is ready");
        let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
        let expected = filmcraft_render::render_sequence(&f.s.project, target, FrameRate::FPS_25.tick_of(12), Default::default(), &provider);
        assert_eq!(f.rgba(&path), expected.to_rgba8(), "ready retry exports the complete stabilization result");
    }
}

#[test]
fn export_rejects_oversized_active_adjustment_layers_before_allocation() {
    let mut f = Fixture::new();
    f.source(f.sequence, 12);
    let p = Arc::make_mut(&mut f.s.project);
    let rate = FrameRate::FPS_25;
    let layer = p.add_item(
        "Malformed adjustment",
        Label::Iris,
        ItemKind::AdjustmentLayer { width: u32::MAX, height: u32::MAX, rate, duration: Tick::from_units(1, 1) },
        None,
    );
    let clip = p.make_track_item(layer, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, Tick::from_units(1, 1)), rate).unwrap();
    p.sequence_mut(f.sequence).unwrap().video_tracks[0].items.clear();
    p.sequence_mut(f.sequence).unwrap().video_tracks[0].items.push(clip);
    let path = f.dir.join("adjustment.png");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()}))));
    assert!(result.unwrap().is_err());
    assert!(!path.exists());
    Arc::make_mut(&mut f.s.project).sequence_mut(f.sequence).unwrap().video_tracks[0].enabled = false;
    f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(path.exists());
}

#[test]
fn warp_retry_reanalyses_a_nested_frame_when_one_of_its_layers_defers() {
    let _lock = warp_test_lock();
    use filmcraft_project::ParamValue;
    filmcraft_render::vfx::clear_stabilizer_cache();
    let mut f = Fixture::new();
    let p = Arc::make_mut(&mut f.s.project);
    let rate = FrameRate::FPS_25;
    let upper = filmcraft_engine::demo::add_generator(
        p,
        &f.s.media,
        GeneratorSource::new(Generator::ColorMatte { color: [0.1, 0.4, 0.8, 0.5] }, 16, 8, rate, Tick::from_units(1, 1)),
        "Ready upper layer",
        Label::Iris,
        None,
    );
    let ready = p.make_track_item(upper, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, Tick::from_units(1, 1)), rate).unwrap();
    let mut track = p.sequence(f.sequence).unwrap().video_tracks[0].clone();
    track.items = vec![ready];
    p.sequence_mut(f.sequence).unwrap().video_tracks.push(track);
    let q = p.sequence_mut(f.program).unwrap();
    q.video_tracks[0].items[0].item = f.sequence;
    q.video_tracks[0].items[0].duration = rate.tick_of(25);
    q.video_tracks[0].items[0].effects.insert(0, effect("warp_stabilizer", &[("framing", ParamValue::Choice(0))]));
    f.s.set_playhead(rate.tick_of(12));
    f.defer_frame.store(1, Ordering::SeqCst);
    let path = f.dir.join("nested-warp.png");
    let error = f.s.execute("file.exportFrame", json!({"path": path.to_string_lossy()})).unwrap_err();
    assert!(error.to_string().contains("still loading"));
    assert!(!path.exists());
    let before = f.reads.load(Ordering::SeqCst);
    f.s.execute("file.exportFrame", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(f.reads.load(Ordering::SeqCst) >= before + 25, "a partial nested analysis frame must not be cached");
    let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
    let expected = filmcraft_render::render_sequence(&f.s.project, f.program, rate.tick_of(12), Default::default(), &provider);
    assert_eq!(f.rgba(&path), expected.to_rgba8());
}

#[test]
fn export_stops_sampling_later_layers_after_the_first_required_source_error() {
    let mut f = Fixture::new();
    f.source(f.sequence, 12);
    let p = Arc::make_mut(&mut f.s.project);
    let healthy = p.sequence(f.sequence).unwrap().video_tracks[0].clone();
    p.sequence_mut(f.sequence).unwrap().video_tracks[0].items[0].item = ItemId(u64::MAX);
    p.sequence_mut(f.sequence).unwrap().video_tracks.push(healthy);
    let path = f.dir.join("abort.png");
    let before = f.reads.load(Ordering::SeqCst);
    assert!(f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()})).is_err());
    assert_eq!(f.reads.load(Ordering::SeqCst), before, "known export failure must avoid decoding later layers");
    assert!(!path.exists());
}

#[test]
fn warp_analysis_is_not_reused_by_another_session_with_the_same_clip_ids() {
    let _lock = warp_test_lock();
    filmcraft_render::vfx::clear_stabilizer_cache();
    let mut first = Fixture::new();
    enable_source_stabilizer(&mut first);
    first.s.execute("source.exportFrame", json!({"path": first.dir.join("first.png").to_string_lossy()})).unwrap();
    assert!(first.reads.load(Ordering::SeqCst) >= 26);
    let reads = first.reads.load(Ordering::SeqCst);
    first.s.execute("source.exportFrame", json!({"path": first.dir.join("unchanged.png").to_string_lossy()})).unwrap();
    assert_eq!(first.reads.load(Ordering::SeqCst), reads + 1, "unchanged exports reuse the complete analysis across new providers");
    let mut second = Fixture::new();
    enable_source_stabilizer(&mut second);
    assert_eq!(first.sequence, second.sequence);
    let path = second.dir.join("second.png");
    second.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(second.reads.load(Ordering::SeqCst) >= 26, "replacement projects must analyse their own media despite reused IDs");
    filmcraft_render::vfx::clear_stabilizer_cache();
    let provider = second.s.media.full_res_provider(second.s.project.clone(), second.s.services.clone());
    let expected = filmcraft_render::render_item(&second.s.project, second.sequence, second.s.state.source_playhead, 1.0, &provider).unwrap();
    assert_eq!(second.rgba(&path), expected.to_rgba8());
}

#[test]
fn warp_analysis_follows_project_snapshot_changes_and_explicit_source_replacement() {
    let _lock = warp_test_lock();
    filmcraft_render::vfx::clear_stabilizer_cache();
    let mut f = Fixture::new();
    enable_source_stabilizer(&mut f);
    f.s.execute("source.exportFrame", json!({"path": f.dir.join("original.png").to_string_lossy()})).unwrap();
    let before = f.reads.load(Ordering::SeqCst);
    Arc::make_mut(&mut f.s.project).sequence_mut(f.sequence).unwrap().video_tracks[0].items[0].frame_hold = Some(FrameRate::FPS_25.tick_of(5));
    f.s.execute("source.exportFrame", json!({"path": f.dir.join("hold.png").to_string_lossy()})).unwrap();
    assert!(f.reads.load(Ordering::SeqCst) >= before + 26, "frame holds must invalidate the previous camera analysis");
    // The project remains identical, but replacing an already-opened source also changes
    // the provider namespace. This covers relink/reopen and synthetic host sources.
    let before = f.reads.load(Ordering::SeqCst);
    let source = f.s.media.cached(f.video).unwrap();
    f.s.media.insert(f.video, source);
    f.s.execute("source.exportFrame", json!({"path": f.dir.join("reopened.png").to_string_lossy()})).unwrap();
    assert!(f.reads.load(Ordering::SeqCst) >= before + 26, "explicit source replacement must refresh cached analysis");
}

#[test]
fn warp_proxy_preview_does_not_supply_original_export_analysis_and_keeps_its_captured_mode() {
    let _lock = warp_test_lock();
    filmcraft_render::vfx::clear_stabilizer_cache();
    let mut f = Fixture::new();
    let proxy_path = f.dir.join("analysis-proxy.png");
    let proxy = filmcraft_export::encode_still(filmcraft_export::Format::PngSequence, [0, 255, 0, 255].repeat(16 * 8), 16, 8).unwrap();
    std::fs::write(&proxy_path, proxy).unwrap();
    if let ItemKind::Media(media) = &mut Arc::make_mut(&mut f.s.project).item_mut(f.video).unwrap().kind {
        media.proxy = Some(filmcraft_project::MediaRef::File { path: proxy_path.to_string_lossy().to_string() });
    }
    enable_source_stabilizer(&mut f);
    f.s.media.set_use_proxies(true);
    let preview = f.s.media.provider(f.s.project.clone(), f.s.services.clone());
    let proxy_picture = filmcraft_render::render_item(&f.s.project, f.sequence, f.s.state.source_playhead, 1.0, &preview).unwrap().to_rgba8();
    assert_eq!(f.reads.load(Ordering::SeqCst), 0, "proxy analysis must not read originals");
    f.s.media.set_use_proxies(false);
    let captured_picture = filmcraft_render::render_item(&f.s.project, f.sequence, f.s.state.source_playhead, 1.0, &preview).unwrap().to_rgba8();
    assert_eq!(captured_picture, proxy_picture, "a scheduled render retains its source mode");
    assert_eq!(f.reads.load(Ordering::SeqCst), 0);
    let path = f.dir.join("original-analysis.png");
    f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(f.reads.load(Ordering::SeqCst) >= 26, "original export must analyse original frames after proxy warm-up");
    assert_ne!(f.rgba(&path), proxy_picture);
    filmcraft_render::vfx::clear_stabilizer_cache();
    let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
    let expected = filmcraft_render::render_item(&f.s.project, f.sequence, f.s.state.source_playhead, 1.0, &provider).unwrap();
    assert_eq!(f.rgba(&path), expected.to_rgba8());
}

#[test]
fn warp_snapshot_namespace_tracks_nested_edits_without_changing_the_outer_clip_key() {
    let _lock = warp_test_lock();
    filmcraft_render::vfx::clear_stabilizer_cache();
    let mut f = Fixture::new();
    let q = Arc::make_mut(&mut f.s.project).sequence_mut(f.program).unwrap();
    q.video_tracks[0].items[0].item = f.sequence;
    q.video_tracks[0].items[0].duration = FrameRate::FPS_25.tick_of(25);
    q.video_tracks[0].items[0].effects.insert(0, effect("warp_stabilizer", &[("framing", filmcraft_project::ParamValue::Choice(0))]));
    f.source(f.program, 12);
    let old_provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
    let old_picture = filmcraft_render::render_item(&old_provider.project, f.program, f.s.state.source_playhead, 1.0, &old_provider).unwrap().to_rgba8();
    let reads = f.reads.load(Ordering::SeqCst);
    // Only the nested media trim changes. The outer item/range/speed/hold/multicam key stays identical.
    Arc::make_mut(&mut f.s.project).sequence_mut(f.sequence).unwrap().video_tracks[0].items[0].source_in = FrameRate::FPS_25.tick_of(5);
    let path = f.dir.join("nested-edit.png");
    f.s.execute("source.exportFrame", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(f.reads.load(Ordering::SeqCst) >= reads + 26, "a new project snapshot must analyse the edited nested picture");
    assert_ne!(f.rgba(&path), old_picture);
    let reads = f.reads.load(Ordering::SeqCst);
    let retained_picture = filmcraft_render::render_item(&old_provider.project, f.program, f.s.state.source_playhead, 1.0, &old_provider).unwrap().to_rgba8();
    assert_eq!(retained_picture, old_picture, "jobs retaining the previous snapshot keep its own analysis");
    assert_eq!(f.reads.load(Ordering::SeqCst), reads + 1);
}

#[test]
fn captured_source_item_and_time_survive_changed_and_closed_current_monitors() {
    let mut f = Fixture::new();
    f.source(f.video, 12);
    let captured = f.s.state.source_playhead;
    let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
    let expected = filmcraft_render::render_item(&f.s.project, f.video, captured, 1.0, &provider).unwrap().to_rgba8();
    f.source(f.sequence, 5);
    let changed_time = f.s.state.source_playhead;
    let changed_item = f.s.state.source_item;
    let program = f.s.playhead();
    let path = f.dir.join("captured-source.png");
    f.s.execute("source.exportFrame", json!({"item": f.video.0, "time": captured.0, "path": path.to_string_lossy()})).unwrap();
    assert_eq!(f.rgba(&path), expected);
    assert_eq!((f.s.state.source_item, f.s.state.source_playhead), (changed_item, changed_time));
    assert_eq!(f.s.playhead(), program);
    f.s.state.source_item = None;
    assert!(!f.s.is_enabled("source.exportFrame"), "the ordinary menu follows the closed monitor");
    let path = f.dir.join("closed-source.png");
    f.s.execute("source.exportFrame", json!({"item": f.video.0, "time": captured.0, "path": path.to_string_lossy()})).unwrap();
    assert_eq!(f.rgba(&path), expected);
    assert!(f.s.state.source_item.is_none());
    assert_eq!(f.s.state.source_playhead, changed_time);
    // Explicit targets use their own filename as well as their own picture.
    let result = f.s.execute("source.exportFrame", json!({"item": f.video.0, "time": captured.0})).unwrap();
    assert_eq!(Path::new(result["path"].as_str().unwrap()).file_name().unwrap(), "Source _ frame.Still001.png");
    let path = f.dir.join("bad-capture.png");
    for params in [
        json!({"item": f.subclip.0, "time": FrameRate::FPS_25.tick_of(4).0}),
        json!({"item": f.subclip.0, "time": FrameRate::FPS_25.tick_of(25).0}),
        json!({"item": u64::MAX, "time": captured.0}),
        json!({"item": f.video.0, "time": -1}),
        json!({"item": f.video.0, "time": Tick::MAX.0 + 1}),
        json!({"item": null, "time": captured.0}),
        json!({"item": -1, "time": captured.0}),
        json!({"item": f.video.0, "time": 0.25}),
        json!({"item": f.video.0, "sequence": f.program.0, "time": captured.0}),
    ] {
        let mut params = params;
        params["path"] = json!(path.to_string_lossy());
        let error = f.s.execute("source.exportFrame", params).unwrap_err();
        assert!(!path.exists(), "invalid capture must fail without writing: {error}");
    }
}

#[test]
fn captured_program_sequence_and_time_work_after_program_closes_without_moving_either_monitor() {
    let mut f = Fixture::new();
    f.source(f.video, 5);
    let captured = FrameRate::FPS_25.tick_of(12);
    let provider = f.s.media.full_res_provider(f.s.project.clone(), f.s.services.clone());
    let expected = filmcraft_render::render_sequence(&f.s.project, f.sequence, captured, Default::default(), &provider).to_rgba8();
    let playheads = f.s.state.playheads.clone();
    let source_time = f.s.state.source_playhead;
    f.s.state.active_sequence = None;
    f.s.state.open_sequences.clear();
    assert!(!f.s.is_enabled("file.exportFrame"));
    let path = f.dir.join("captured-program.png");
    f.s.execute("file.exportFrame", json!({"sequence": f.sequence.0, "time": captured.0, "path": path.to_string_lossy()})).unwrap();
    assert_eq!(f.rgba(&path), expected);
    assert!(f.s.state.active_sequence.is_none());
    assert_eq!(f.s.state.playheads, playheads);
    assert_eq!(f.s.playhead(), Tick::ZERO, "a closed Program monitor has no current playhead");
    assert_eq!(f.s.state.source_playhead, source_time);
    std::fs::remove_file(&path).unwrap();
    for target in
        [json!({"sequence": f.video.0}), json!({"sequence": u64::MAX}), json!({"sequence": "bad"}), json!({"sequence": f.sequence.0, "item": f.video.0})]
    {
        let mut args = target;
        args["path"] = json!(path.to_string_lossy());
        assert!(f.s.execute("file.exportFrame", args).is_err());
        assert!(!path.exists());
    }
}
