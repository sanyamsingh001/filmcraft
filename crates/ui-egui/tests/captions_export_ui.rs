//! Standalone TXT export through the existing caption-track picker and native save continuation.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::native_dialogs::FileDialog;
use serde_json::{Value, json};

type Asked = Arc<Mutex<Vec<(FileDialog, Sender<Vec<String>>)>>>;

struct OutputDir(std::path::PathBuf);

impl OutputDir {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("fc-caption-txt-ui-{}-{nonce}", std::process::id()));
        // Own only a newly created directory; never reuse or remove an earlier test's output.
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for OutputDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    asked: Asked,
}

impl Driver {
    fn new(session: Session) -> Self {
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(session).with_control(rx);
        let asked: Asked = Arc::default();
        let calls = asked.clone();
        app.hooks.run_dialog = Some(Box::new(move |dialog| {
            let (tx, rx) = channel();
            calls.lock().unwrap().push((dialog, tx));
            rx
        }));
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Self { harness, tx, asked };
        d.frames(4);
        d
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
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], true, "{method} {params}: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }
}

fn snapshot(s: &Session) -> Value {
    let undo: Vec<_> = s.history.undo.iter().map(|(label, project)| json!([label, project.as_ref()])).collect();
    let redo: Vec<_> = s.history.redo.iter().map(|(label, project)| json!([label, project.as_ref()])).collect();
    json!({"project": s.project.as_ref(), "editor": s.state, "undo": undo, "redo": redo,
        "limit": s.history.limit, "mergeKey": s.history.merge_key, "revision": s.revision,
        "savedRevision": s.saved_revision, "path": s.path})
}

#[test]
fn caption_picker_exports_chosen_track_as_txt_and_keeps_cancel_unchanged() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Caption review", "fps": 25, "video": 0, "audio": 0})).unwrap();
    let review = s.execute("captions.newTrack", json!({"name": "Review"})).unwrap()["track"].as_u64().unwrap();
    s.execute("captions.add", json!({"track": review, "seconds": 0, "durationSeconds": 1, "text": "<b>Review</b>\nSecond line"})).unwrap();
    let other = s.execute("captions.newTrack", json!({"name": "Other"})).unwrap()["track"].as_u64().unwrap();
    s.execute("captions.add", json!({"track": other, "seconds": 0, "durationSeconds": 1, "text": "OTHER TRACK"})).unwrap();
    let mut d = Driver::new(s);
    d.ok("ui.set", json!({"workspace": "Captions and Graphics"}));
    d.ok("ui.click", json!({"id": "text.tab.Captions"}));
    d.frames(3);
    d.ok("ui.click", json!({"id": "text.captions.track"}));
    d.frames(2);
    let label = format!("C2 · Review ({})", filmcraft_ui_egui::i18n::t("Subtitle"));
    d.harness.get_by_label(&label).click();
    d.frames(3);
    let before = snapshot(&d.harness.state().session);
    let dir = OutputDir::new();
    let path = dir.0.join("review.TXT");
    d.ok("ui.click", json!({"id": "text.captions.export"}));
    d.frames(3);
    assert!(d.harness.state().native_dialog_open());
    let (dialog, answer) = d.asked.lock().unwrap().pop().expect("caption save picker");
    let FileDialog::SaveAs { filter, exts, name } = dialog else { panic!("wrong dialog") };
    assert!(filter.contains("TXT") && exts.iter().any(|ext| ext == "txt"));
    assert_eq!(name, "Caption_review.srt");
    assert!(!path.exists());
    answer.send(vec![path.to_string_lossy().into_owned()]).unwrap();
    d.frames(4);
    assert!(!d.harness.state().native_dialog_open(), "save answer was processed");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Review\nSecond line\n");
    assert_eq!(snapshot(&d.harness.state().session), before);

    let result = d.ok("ui.menu.invoke", json!({"id": "captions.export", "params": {"track": review, "format": ".TXT"}}));
    assert_eq!(result["dialog"], "open");
    assert!(d.harness.state().native_dialog_open());
    let (dialog, answer) = d.asked.lock().unwrap().pop().expect("explicit TXT save picker");
    let FileDialog::SaveAs { name, .. } = dialog else { panic!("wrong dialog") };
    assert_eq!(name, "Caption_review.txt");
    answer.send(Vec::new()).unwrap();
    d.frames(4);
    assert!(!d.harness.state().native_dialog_open(), "cancel answer was processed");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Review\nSecond line\n");
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1, "cancel writes no additional output");
    assert_eq!(snapshot(&d.harness.state().session), before);
}
