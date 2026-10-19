//! Delayed host replies exercise actual command entry points and project changes.
use super::*;
use serde_json::json;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn app() -> FilmcraftApp {
    FilmcraftApp::new(filmcraft_engine::Session::default())
}
fn host(app: &mut FilmcraftApp) -> Arc<Mutex<Option<(PickerRequest, PickerReply)>>> {
    let held = Arc::new(Mutex::new(None));
    let output = held.clone();
    app.hooks.async_picker = Some(Box::new(move |request, reply| {
        *output.lock().unwrap() = Some((request, reply));
        Ok(())
    }));
    held
}
fn finish(app: &mut FilmcraftApp, held: &Arc<Mutex<Option<(PickerRequest, PickerReply)>>>, result: PickerOutcome) -> PickerRequest {
    let (request, reply) = held.lock().unwrap().take().unwrap();
    reply(result);
    app.poll_picker(&egui::Context::default());
    request
}
fn path(name: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::path::Path::new("target/fixtures/ui/pickers");
    std::fs::create_dir_all(dir).unwrap();
    dir.join(format!("{}-{}-{name}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

#[test]
fn delayed_open_keeps_commands_responsive_cancel_does_not_open_and_dialogs_are_bounded() {
    let mut app = app();
    let held = host(&mut app);
    assert_eq!(app.file_dialog("file.open", &json!({})).unwrap()["pending"], true);
    assert!(app.picker_pending());
    assert!(app.session.execute("command.list", json!({})).is_ok(), "commands continue while the host has not replied");
    assert_eq!(app.file_dialog("file.saveAs", &json!({})).unwrap_err(), "File dialog already open");
    assert_eq!(app.ui.status, "File dialog already open");
    assert_eq!(held.lock().unwrap().as_ref().unwrap().0.kind, PickerKind::File, "second request did not replace the first");
    finish(&mut app, &held, Ok(vec![]));
    assert!(!app.picker_pending());
    assert!(app.session.path.is_none());
}

#[test]
fn delayed_caption_import_uses_single_file_continuation() {
    let file = path("captions.srt");
    std::fs::write(&file, "1\n00:00:00,000 --> 00:00:01,000\nHarbour\n").unwrap();
    let mut app = app();
    let held = host(&mut app);
    app.file_dialog("captions.import", &json!({"name": "Chosen captions"})).unwrap();
    assert!(app.session.state.active_sequence.is_none());
    let request = finish(&mut app, &held, Ok(vec![file.to_string_lossy().into_owned()]));
    assert_eq!(request.kind, PickerKind::File);
    let seq = app.session.active_sequence().unwrap();
    assert_eq!(seq.caption_tracks.len(), 1);
    assert_eq!(seq.caption_tracks[0].name, "Chosen captions");
    assert!(!app.picker_pending());
    let _ = std::fs::remove_file(file);
}

#[test]
fn delayed_save_keeps_suggested_name_and_saves_only_after_selection() {
    let file = path("Project space.fcproj");
    let mut app = app();
    let held = host(&mut app);
    app.file_dialog("file.saveCopy", &json!({})).unwrap();
    assert!(!file.exists());
    let request = finish(&mut app, &held, Ok(vec![file.to_string_lossy().into_owned()]));
    assert_eq!(request.kind, PickerKind::Save);
    assert!(request.suggested.ends_with(" copy.fcproj"));
    assert!(file.exists());
    assert!(app.session.path.is_none(), "Save Copy leaves current project path unchanged");
    let _ = std::fs::remove_file(file);
}

#[test]
fn project_replacement_discards_late_choice() {
    let mut app = app();
    let held = host(&mut app);
    let called = Arc::new(AtomicBool::new(false));
    let flag = called.clone();
    app.defer_picker(PickerRequest::folder(), move |_, _| {
        flag.store(true, Ordering::Relaxed);
        Ok(())
    });
    app.session.media = Arc::new(filmcraft_engine::MediaPool::default());
    finish(&mut app, &held, Ok(vec!["chosen".into()]));
    assert!(!called.load(Ordering::Relaxed));
    assert!(app.ui.status.contains("project changed"));
}

#[test]
fn closed_then_reopened_draft_does_not_receive_late_result() {
    let mut app = app();
    let held = host(&mut app);
    let open = Arc::new(AtomicBool::new(true));
    let valid = open.clone();
    let called = Arc::new(AtomicBool::new(false));
    let flag = called.clone();
    app.defer_picker_guarded(
        PickerDraft::Settings,
        PickerRequest::folder(),
        move |_| valid.load(Ordering::Relaxed),
        move |_, _| {
            flag.store(true, Ordering::Relaxed);
            Ok(())
        },
    );
    open.store(false, Ordering::Relaxed);
    app.poll_picker(&egui::Context::default());
    open.store(true, Ordering::Relaxed);
    finish(&mut app, &held, Ok(vec!["chosen".into()]));
    assert!(!called.load(Ordering::Relaxed));
    assert!(app.ui.status.contains("dialog changed"));
}

#[test]
fn host_errors_and_abandoned_reply_release_pending_request() {
    let mut app = app();
    app.hooks.async_picker = Some(Box::new(|_, _| Err("picker unavailable".into())));
    assert_eq!(app.file_dialog("file.open", &json!({})).unwrap_err(), "picker unavailable");
    assert_eq!(app.ui.status, "picker unavailable");
    assert!(!app.picker_pending());
    let held = host(&mut app);
    app.file_dialog("file.open", &json!({})).unwrap();
    finish(&mut app, &held, Err("backend failed".into()));
    assert_eq!(app.ui.status, "backend failed");
    assert!(!app.picker_pending());
    app.file_dialog("file.open", &json!({})).unwrap();
    drop(held.lock().unwrap().take());
    app.poll_picker(&egui::Context::default());
    assert!(app.ui.status.contains("without a response"));
    assert!(!app.picker_pending());
}

#[test]
fn legacy_save_hook_remains_available_without_async_adapter() {
    let mut app = app();
    let file = path("legacy.fcproj");
    let chosen = file.to_string_lossy().into_owned();
    app.hooks.pick_save = Some(Box::new(move |_| Some(chosen.clone())));
    app.file_dialog("file.saveCopy", &json!({})).unwrap();
    assert!(file.exists());
    assert!(!app.picker_pending());
    assert!(!app.defer_picker(PickerRequest::folder(), |_, _| panic!("legacy caller must run its own hook")));
    let _ = std::fs::remove_file(file);
}

#[test]
fn export_selection_uses_items_chosen_before_picker_opened() {
    let mut app = app();
    let mut project = (*app.session.project).clone();
    let old_item = project.new_sequence("Original", Default::default(), 1, 0, None);
    let new_item = project.new_sequence("Later", Default::default(), 1, 0, None);
    app.session.project = Arc::new(project);
    app.session.state.project_selection = vec![old_item];
    let held = host(&mut app);
    app.defer_picker_command(PickerRequest::save("FilmCraft Project", &["fcproj"], "Selected.fcproj"), "file.exportSelectionProject", json!({}), false);
    app.session.state.project_selection = vec![new_item];
    let file = path("selection.fcproj");
    finish(&mut app, &held, Ok(vec![file.to_string_lossy().into_owned()]));
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert!(saved.to_string().contains("Original"));
    assert!(!saved.to_string().contains("Later"));
    let _ = std::fs::remove_file(file);
}

#[test]
fn new_and_demo_projects_reject_old_picker_replies_while_regular_edits_allow_them() {
    for command in ["file.newProject", "file.openDemoProject", "file.newBin"] {
        let mut app = app();
        let held = host(&mut app);
        let called = Arc::new(AtomicBool::new(false));
        let flag = called.clone();
        app.defer_picker(PickerRequest::folder(), move |_, _| {
            flag.store(true, Ordering::Relaxed);
            Ok(())
        });
        crate::menus::invoke(&mut app, &egui::Context::default(), command, json!({"name": "Changed"})).unwrap();
        finish(&mut app, &held, Ok(vec!["chosen".into()]));
        assert_eq!(called.load(Ordering::Relaxed), command == "file.newBin", "{command}");
    }
}

#[test]
fn file_import_captions_does_not_follow_a_changed_sequence() {
    let file = path("changed-sequence.srt");
    std::fs::write(&file, "1\n00:00:00,000 --> 00:00:01,000\nHarbour\n").unwrap();
    let mut app = app();
    let mut project = (*app.session.project).clone();
    let first = project.new_sequence("First", Default::default(), 1, 0, None);
    let next = project.new_sequence("Next", Default::default(), 1, 0, None);
    app.session.project = Arc::new(project);
    app.session.state.active_sequence = Some(first);
    let held = host(&mut app);
    app.file_dialog("file.import", &json!({})).unwrap();
    app.session.state.active_sequence = Some(next);
    finish(&mut app, &held, Ok(vec![file.to_string_lossy().into_owned()]));
    assert!(app.session.project.sequence(next).unwrap().caption_tracks.is_empty());
    assert!(app.ui.status.contains("active sequence changed"));
    let _ = std::fs::remove_file(file);
}

#[test]
fn delayed_lut_choice_has_no_library_side_effect_after_sequence_change() {
    let file = path("identity.cube");
    std::fs::write(&file, "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").unwrap();
    for command in ["lumetri.setInputLut", "lumetri.setLook"] {
        let mut app = app();
        let mut project = (*app.session.project).clone();
        let first = project.new_sequence("First", Default::default(), 1, 0, None);
        let next = project.new_sequence("Next", Default::default(), 1, 0, None);
        app.session.project = Arc::new(project);
        app.session.state.active_sequence = Some(first);
        let held = host(&mut app);
        app.defer_picker_command(PickerRequest::file("LUT", &["cube"]), command, json!({"clip": 999}), false);
        app.session.state.active_sequence = Some(next);
        finish(&mut app, &held, Ok(vec![file.to_string_lossy().into_owned()]));
        assert!(app.session.project.luts.is_empty());
        assert!(app.ui.status.contains("active sequence changed"));
    }
    let _ = std::fs::remove_file(file);
}
