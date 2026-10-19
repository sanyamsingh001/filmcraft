//! `.fcproj` save/open through the command interface: schema envelope, legacy upgrade, Save a Copy,
//! Revert.

use super::*;
use serde_json::json;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("fc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn saved_file_is_versioned_and_save_copy_keeps_state() {
    let d = temp_dir("schema");
    let path = d.join("p.fcproj").to_string_lossy().to_string();
    let mut s = demo();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(v["schema_version"], filmcraft_format::SCHEMA_VERSION);
    assert_eq!(v["format"], "filmcraft.project");
    s.execute("sequence.addEdit", json!({"seconds": 1.0})).unwrap();
    assert!(s.is_dirty());
    let copy = d.join("copy.fcproj").to_string_lossy().to_string();
    s.execute("file.saveCopy", json!({"path": copy})).unwrap();
    assert!(s.is_dirty(), "Save a Copy leaves the project unsaved");
    assert_eq!(s.path.as_deref(), Some(path.as_str()));
    let mut t = Session::default();
    t.execute("file.open", json!({"path": copy})).unwrap();
    assert_eq!(*t.project, *s.project);
    // Revert discards the edit.
    s.execute("file.revert", json!({})).unwrap();
    assert!(!s.is_dirty());
    assert_ne!(*t.project, *s.project);
    assert!(!Session::default().is_enabled("file.revert"), "nothing to revert to before a save");
    // No temp files left next to the project.
    let stray: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
    assert!(stray.is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn legacy_project_opens_and_first_save_keeps_a_backup() {
    let d = temp_dir("legacy");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../format/tests/fixtures/v1-edit.fcproj");
    let path = d.join("old.fcproj");
    std::fs::copy(&fixture, &path).unwrap();
    let p = path.to_string_lossy().to_string();
    let mut s = Session::default();
    let r = s.execute("file.open", json!({"path": p})).unwrap();
    assert_eq!(r["schemaVersion"], 1);
    assert_eq!(r["migrated"], true);
    assert_eq!(s.project.name, "Legacy Edit");
    assert!(s.drain_events().iter().any(|e| matches!(e, Event::Toast { message, .. } if message.contains("Upgraded"))));
    let r = s.execute("file.save", json!({})).unwrap();
    let backup = r["backup"].as_str().unwrap().to_string();
    assert!(backup.ends_with("old (schema v1 backup).fcproj"), "{backup}");
    assert_eq!(std::fs::read(&backup).unwrap(), std::fs::read(&fixture).unwrap());
    let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(v["schema_version"], filmcraft_format::SCHEMA_VERSION);
    // Second save: no new backup.
    assert!(s.execute("file.save", json!({})).unwrap()["backup"].is_null());
    // A file from the future is refused clearly.
    let mut v = v;
    v["schema_version"] = json!(filmcraft_format::SCHEMA_VERSION + 1);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let e = Session::default().execute("file.open", json!({"path": p})).unwrap_err().to_string();
    assert!(e.contains("newer version of FilmCraft"), "{e}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn project_is_named_after_its_file() {
    let d = temp_dir("name");
    let mut s = demo();
    let a = d.join("Apollo 11 - Tranquility.fcproj").to_string_lossy().to_string();
    s.execute("file.saveAs", json!({"path": a})).unwrap();
    assert_eq!(s.project.name, "Apollo 11 - Tranquility");
    assert_eq!(s.project.root.name, "Apollo 11 - Tranquility");
    assert!(!s.is_dirty(), "renaming on Save As is not an edit");
    // Save a Copy keeps the current name.
    let c = d.join("Backup copy.fcproj").to_string_lossy().to_string();
    s.execute("file.saveCopy", json!({"path": c})).unwrap();
    assert_eq!(s.project.name, "Apollo 11 - Tranquility");
    s.execute("file.open", json!({"path": c})).unwrap();
    assert_eq!(s.project.name, "Apollo 11 - Tranquility");
    let _ = std::fs::remove_dir_all(&d);
}

/// Project files in memory so lifecycle checks never overwrite a contributor's project.
#[derive(Default)]
struct MemoryFiles {
    files: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl Services for MemoryFiles {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.files.lock().unwrap().get(path).cloned().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path.to_string()))
    }

    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        self.files.lock().unwrap().insert(path.to_string(), data.to_vec());
        Ok(())
    }
}

#[test]
fn opening_demo_never_adopts_the_previous_projects_save_path() {
    let files = Arc::new(MemoryFiles::default());
    let original = filmcraft_format::encode(&Project::new("Original"), false);
    files.write_file("original.fcproj", &original).unwrap();
    let mut s = Session::new(files.clone());
    s.execute("file.open", json!({"path": "original.fcproj"})).unwrap();
    assert_eq!(s.path.as_deref(), Some("original.fcproj"));

    let opened = s.execute("file.openDemoProject", json!({})).unwrap();
    let seq = ItemId(opened["sequence"].as_u64().unwrap());
    assert_eq!(s.project.name, "FilmCraft Demo");
    assert_eq!(s.state.active_sequence, Some(seq));
    assert_eq!(s.state.open_sequences, vec![seq]);
    assert_eq!(s.playhead(), FrameRate::FPS_23_976.tick_of(4 * 24 + 6));
    assert!(s.path.is_none(), "the demo needs its own Save As destination");
    assert!(!s.is_enabled("file.revert"));
    assert!(matches!(s.execute("file.save", json!({})), Err(EngineError::BadParams { .. })));
    assert_eq!(files.read_file("original.fcproj").unwrap(), original, "Save must preserve the original project");
    assert!(!s.is_dirty(), "the unchanged demo keeps its existing clean-state behavior");
}

#[test]
fn new_and_demo_release_previous_sources_and_reset_project_state() {
    use filmcraft_media::Generator;
    use filmcraft_media::generators::GeneratorSource;

    for command in ["file.newProject", "file.openDemoProject"] {
        for saved in [false, true] {
            let mut s = Session::new(Arc::new(MemoryFiles::default()));
            let old_item = ItemId(1_000_001);
            let source =
                Arc::new(GeneratorSource::new(Generator::ColorMatte { color: [0.2, 0.3, 0.4, 1.0] }, 64, 36, FrameRate::FPS_24, Tick::from_seconds_f64(1.0)));
            let old_source = Arc::downgrade(&source);
            s.media.insert(old_item, source);
            let old_pool = Arc::downgrade(&s.media);
            s.media.set_use_proxies(true);
            s.offline.missing = vec![old_item];
            s.offline.prompt = true;
            s.loaded_schema = 1;
            s.history.undo.push(("Previous edit".into(), s.project.clone()));
            s.history.redo.push(("Previous redo".into(), s.project.clone()));
            s.state.source_item = Some(old_item);
            if saved {
                s.path = Some("original.fcproj".into());
            }
            let cache_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/lifecycle-previews");
            let previous_previews = cache_root.join("previous-project");
            s.previews.set_temp_root(Some(cache_root.clone()));
            s.previews.set_dir(Some(previous_previews.clone()));
            s.previews.add("0123456789abcdef0123456789abcdef");
            s.previews.add_audio("fedcba9876543210fedcba9876543210");
            assert_eq!(s.previews.count(), 2);

            s.execute(command, json!({"name": "New Project"})).unwrap();

            assert!(old_pool.upgrade().is_none(), "{command}: the previous media pool was released");
            assert!(old_source.upgrade().is_none(), "{command}: the previous cached source was released");
            assert!(s.media.use_proxies(), "{command}: the proxy preference is retained");
            assert!(s.media.cached(old_item).is_none());
            assert!(s.offline.missing.is_empty() && !s.offline.prompt);
            assert_eq!(s.loaded_schema, filmcraft_format::SCHEMA_VERSION);
            assert!(s.path.is_none());
            assert!(s.state.source_item.is_none());
            assert!(!s.history.can_undo() && !s.history.can_redo());
            assert!(!s.is_dirty());
            assert_eq!(s.previews.count(), 0, "{command}: preview indexes belong to the previous project");
            let previews = s.previews.dir().unwrap();
            assert_ne!(previews, previous_previews, "{command}: saved and unsaved projects both get fresh previews");
            assert!(previews.starts_with(&cache_root), "the configured preview-cache root is retained");

            if command == "file.openDemoProject" {
                let generators = s
                    .project
                    .items
                    .values()
                    .filter(|i| matches!(&i.kind, filmcraft_project::ItemKind::Media(m) if matches!(&m.media, filmcraft_project::MediaRef::Generator(_))))
                    .count();
                assert!(generators > 0);
                assert_eq!(s.media.open_sources(), generators, "the demo retains every source prepared during construction");
                let item = s.active_sequence().unwrap().video_tracks[0].items[0].item;
                let source = s.source(item).unwrap();
                let frame = source.video_frame(filmcraft_media::FrameRequest { time: Tick::ZERO, scale: 1.0 / 32.0 }).unwrap();
                assert!(frame.width > 0 && frame.height > 0, "the demo still renders its generated footage");
            } else {
                assert_eq!(s.media.open_sources(), 0);
                assert_eq!(s.project.name, "New Project");
                assert!(s.state.active_sequence.is_none());
                assert!(s.state.open_sequences.is_empty());
            }
        }
    }
}

#[test]
fn switching_projects_detaches_background_edits_even_when_ids_are_reused() {
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    use crate::proxies::{OnDone, PendingJob};
    use crate::scene_detect::{ClipScenes, PendingScene, SceneApply};

    // Demo construction and reopening a saved demo reuse its ids exactly: late scene cuts
    // used to modify the replacement project's first clip despite belonging to the old edit.
    for command in ["file.newProject", "file.openDemoProject", "file.open", "file.revert", "file.newProjectFromTemplate"] {
        let files = Arc::new(MemoryFiles::default());
        let mut s = Session::new(files.clone());
        s.execute("file.openDemoProject", json!({})).unwrap();
        let seq = s.state.active_sequence.unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        let cut = clip.start + s.sequence_rate().tick_of(12);
        let media_cut = clip.source_time_at(cut);
        files.write_file("saved.fcproj", &filmcraft_format::encode(&s.project, false)).unwrap();
        s.path = Some("saved.fcproj".into());

        let editing = Job { id: 1, label: "Scene detection".into(), progress: Default::default(), result: Default::default() };
        let scene_progress = editing.progress.clone();
        let scene_results = Arc::new(Mutex::new(Some(vec![ClipScenes {
            clip: clip.id,
            name: clip.name.clone(),
            media_item: clip.item,
            media_range: filmcraft_time::TimeRange::from_bounds(clip.source_in, clip.source_out()),
            cuts: vec![(cut, media_cut)],
        }])));
        s.jobs.push(editing);
        s.scene_jobs.push(PendingScene { job: 1, seq, apply: SceneApply { cuts: true, ..Default::default() }, results: scene_results.clone() });

        let editing = Job { id: 2, label: "Proxy attachment".into(), progress: Default::default(), result: Default::default() };
        let proxy_progress = editing.progress.clone();
        s.jobs.push(editing);
        s.media_jobs.push(PendingJob { job: 2, on_done: OnDone::AttachProxies, outputs: Arc::new(Mutex::new(vec![])) });

        let standalone = Job { id: 3, label: "Export".into(), progress: Default::default(), result: Default::default() };
        let export_progress = standalone.progress.clone();
        s.jobs.push(standalone);
        let standalone = Job { id: 4, label: "Project Manager copy".into(), progress: Default::default(), result: Default::default() };
        let copy_progress = standalone.progress.clone();
        s.jobs.push(standalone);
        s.media_jobs.push(PendingJob { job: 4, on_done: OnDone::Nothing, outputs: Arc::new(Mutex::new(vec![])) });

        s.execute(command, json!({"path": "saved.fcproj", "template": "saved.fcproj", "force": true})).unwrap();
        if command != "file.newProject" {
            assert!(s.project.sequence(seq).unwrap().find_item(clip.id).is_some(), "the replacement really reuses the old ids");
        }
        assert!(scene_progress.cancel.load(Ordering::Relaxed), "{command}: scene decoding can stop");
        assert!(proxy_progress.cancel.load(Ordering::Relaxed), "{command}: proxy encoding can stop");
        assert!(!export_progress.cancel.load(Ordering::Relaxed), "{command}: exports continue");
        assert!(!copy_progress.cancel.load(Ordering::Relaxed), "{command}: standalone copies continue");
        assert!(s.scene_jobs.is_empty() && s.mask_jobs.is_empty());
        assert_eq!(s.media_jobs.len(), 1);
        assert_eq!(s.media_jobs[0].job, 4);
        assert_eq!(s.jobs.len(), 4, "the job list keeps the existing progress records");

        let before = s.project.clone();
        // Simulate the worker finishing after the switch. Its old buffer remains writable,
        // but polling must not apply its cuts to the replacement timeline.
        scene_progress.finished.store(true, Ordering::Relaxed);
        assert!(scene_results.lock().unwrap().is_some());
        s.poll_persistence();
        assert_eq!(*s.project, *before, "{command}: late cuts cannot change the replacement project");
        assert!(!s.is_dirty());
    }
}

#[test]
fn replacement_isolates_late_previews_and_clears_live_recording_passes() {
    for command in ["file.newProject", "file.openDemoProject", "file.open", "file.revert", "file.newProjectFromTemplate"] {
        let files = Arc::new(MemoryFiles::default());
        let mut s = Session::new(files.clone());
        s.execute("file.openDemoProject", json!({})).unwrap();
        s.execute("mixer.setStrip", json!({"strip": "A1", "mode": "Touch"})).unwrap();
        files.write_file("saved.fcproj", &filmcraft_format::encode(&s.project, false)).unwrap();
        s.path = Some("saved.fcproj".into());
        s.saved_revision = s.revision;
        s.execute("mixer.recordStart", json!({"time": 0})).unwrap();
        s.execute("mixer.touch", json!({"strip": "A1", "lane": "volume", "value": -10.0, "time": 0})).unwrap();
        s.execute("multicam.recordStart", json!({"time": 0})).unwrap();
        assert!(s.mixrec.active() && !s.mixrec.gestures.is_empty());
        assert!(s.mcrec.active());

        let cache_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/lifecycle-previews");
        s.previews.set_temp_root(Some(cache_root.clone()));
        let old_store = s.previews.clone();
        let old_dir = old_store.dir();
        assert!(old_store.live.is_active());

        s.execute(command, json!({"path": "saved.fcproj", "template": "saved.fcproj", "force": true})).unwrap();

        assert!(!Arc::ptr_eq(&s.previews, &old_store), "{command}: jobs retain a store belonging to their original project");
        assert_eq!(s.previews.temp_root(), Some(cache_root));
        assert!(!s.previews.live.is_active(), "{command}: old fader overrides cannot affect new playback");
        assert!(!s.mixrec.active() && !s.mcrec.active() && !s.trim_play.active());

        // A video and audio worker finish against the old snapshot after replacement.
        old_store.add("0123456789abcdef0123456789abcdef");
        old_store.add_audio("fedcba9876543210fedcba9876543210");
        assert_eq!(old_store.count(), 2);
        assert_eq!(old_store.dir(), old_dir, "late workers still use their original output directory");
        assert_eq!(s.previews.count(), 0, "{command}: late previews cannot mark the replacement timeline as rendered");

        let before = s.project.clone();
        if s.state.active_sequence.is_some() {
            let written = s.execute("mixer.recordStop", json!({"time": Tick::from_seconds_f64(1.0).0})).unwrap();
            assert_eq!(written["lanes"], 0, "a stopped old pass cannot write automation into reused ids");
        }
        s.execute("multicam.recordStop", json!({})).unwrap();
        assert_eq!(*s.project, *before);
        assert!(!s.is_dirty());
    }
}

#[test]
fn replacement_refuses_to_lose_an_unfinished_voice_over_take() {
    for command in ["file.newProject", "file.openDemoProject", "file.open", "file.revert", "file.newProjectFromTemplate"] {
        let files = Arc::new(MemoryFiles::default());
        let mut s = Session::new(files.clone());
        s.execute("file.openDemoProject", json!({})).unwrap();
        files.write_file("saved.fcproj", &filmcraft_format::encode(&s.project, false)).unwrap();
        s.path = Some("saved.fcproj".into());
        let seq = s.state.active_sequence.unwrap();
        s.execute("audio.voiceover.start", json!({"track": "A3", "time": 0, "preroll": 0})).unwrap();
        let before = s.project.clone();
        let pool = s.media.clone();
        let store = s.previews.clone();

        let error = s.execute(command, json!({"path": "saved.fcproj", "template": "saved.fcproj", "force": true})).unwrap_err().to_string();

        assert!(error.contains("voice-over take"), "{error}");
        assert!(Arc::ptr_eq(&s.project, &before), "{command}: the original project stays open");
        assert!(Arc::ptr_eq(&s.media, &pool) && Arc::ptr_eq(&s.previews, &store));
        assert_eq!(s.path.as_deref(), Some("saved.fcproj"));
        assert!(s.voiceover.recording());
        assert_eq!(s.voiceover.rec.as_ref().unwrap().seq, seq, "{command}: the take remains attached to its original timeline");

        s.execute("audio.voiceover.stop", json!({"discard": true})).unwrap();
        s.execute(command, json!({"path": "saved.fcproj", "template": "saved.fcproj", "force": true})).unwrap();
        assert!(!s.voiceover.recording(), "replacement succeeds after the take was explicitly discarded");
    }
}

#[test]
fn template_replacement_keeps_dirty_confirmation_and_clean_unsaved_state() {
    let files = Arc::new(MemoryFiles::default());
    let mut s = Session::new(files.clone());
    s.execute("file.openDemoProject", json!({})).unwrap();
    files.write_file("template.fcproj", &filmcraft_format::encode(&s.project, false)).unwrap();
    s.path = Some("original.fcproj".into());
    s.execute("sequence.addEdit", json!({"seconds": 1.0})).unwrap();
    assert!(s.is_dirty());
    let before = s.project.clone();
    let pool = s.media.clone();
    let previews = s.previews.clone();
    let error = s.execute("file.newProjectFromTemplate", json!({"template": "template.fcproj"})).unwrap_err().to_string();
    assert!(error.contains("unsaved changes"), "{error}");
    assert!(Arc::ptr_eq(&s.project, &before) && Arc::ptr_eq(&s.media, &pool) && Arc::ptr_eq(&s.previews, &previews));
    assert_eq!(s.path.as_deref(), Some("original.fcproj"));

    s.loaded_schema = 1;
    s.offline.missing = vec![ItemId(1_000_001)];
    s.offline.prompt = true;
    s.media.set_use_proxies(true);
    s.execute("file.newProjectFromTemplate", json!({"template": "template.fcproj", "name": "Episode 2", "force": true})).unwrap();
    assert_eq!(s.project.name, "Episode 2");
    assert_eq!(s.project.root.name, "Episode 2");
    assert!(s.path.is_none() && !s.is_dirty(), "a new template retains the existing clean state and requires its own Save As path");
    assert!(!Arc::ptr_eq(&s.media, &pool) && !Arc::ptr_eq(&s.previews, &previews));
    assert!(s.media.use_proxies());
    assert!(s.offline.missing.is_empty() && !s.offline.prompt);
    assert_eq!(s.loaded_schema, filmcraft_format::SCHEMA_VERSION);
    assert!(!s.history.can_undo() && !s.history.can_redo());
    assert_eq!(s.media.open_sources(), 0, "the replacement has no cached source from the previous project");
    let sequence = s.state.active_sequence.unwrap();
    assert_eq!(s.state.open_sequences, vec![sequence]);
    let item = s.active_sequence().unwrap().video_tracks[0].items[0].item;
    let source = s.source(item).unwrap();
    let frame = source.video_frame(filmcraft_media::FrameRequest { time: Tick::ZERO, scale: 1.0 / 32.0 }).unwrap();
    assert!(frame.width > 0 && frame.height > 0, "a generator saved in the template opens in its new media pool");
    assert!(s.drain_events().iter().any(|e| matches!(e, Event::OpenSequence(id) if *id == sequence)));
}
