//! Nonblocking host pickers, with UI-thread continuations and legacy host fallbacks.

use crate::FilmcraftApp;
use std::sync::{Arc, Weak, mpsc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Files,
    File,
    Folder,
    Save,
}

/// Identity of a draft whose picker continuation must not survive opening a new instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerDraft {
    Settings,
    LinkMedia,
    Proxies,
    ProjectManager,
    Extras,
    Shortcuts,
    Export,
}

#[derive(Clone, Debug)]
pub struct PickerRequest {
    pub kind: PickerKind,
    pub filter: String,
    pub extensions: Vec<String>,
    pub suggested: String,
}

impl PickerRequest {
    pub fn files(filter: &str, extensions: &[&str]) -> Self {
        Self::new(PickerKind::Files, filter, extensions, "")
    }
    pub fn file(filter: &str, extensions: &[&str]) -> Self {
        Self::new(PickerKind::File, filter, extensions, "")
    }
    pub fn folder() -> Self {
        Self::new(PickerKind::Folder, "", &[], "")
    }
    pub fn save(filter: &str, extensions: &[&str], suggested: &str) -> Self {
        Self::new(PickerKind::Save, filter, extensions, suggested)
    }
    fn new(kind: PickerKind, filter: &str, extensions: &[&str], suggested: &str) -> Self {
        Self { kind, filter: filter.into(), extensions: extensions.iter().map(|s| (*s).into()).collect(), suggested: suggested.into() }
    }
}

pub type PickerOutcome = Result<Vec<String>, String>;
pub type PickerReply = Box<dyn FnOnce(PickerOutcome) + Send>;
pub type PickerHook = Box<dyn FnMut(PickerRequest, PickerReply) -> Result<(), String>>;
type Continuation = Box<dyn FnOnce(&mut FilmcraftApp, Vec<String>) -> Result<(), String>>;
struct Pending {
    rx: mpsc::Receiver<PickerOutcome>,
    pool: Weak<filmcraft_engine::MediaPool>,
    continuation: Continuation,
    valid: Box<dyn Fn(&FilmcraftApp) -> bool>,
    stale: bool,
    draft: Option<PickerDraft>,
    project_changed: bool,
}
#[derive(Default)]
pub(crate) struct PickerState {
    pending: Option<Pending>,
    ctx: Option<egui::Context>,
    error: Option<String>,
}

impl FilmcraftApp {
    /// True when an asynchronous host handled the request, including a pending-dialog rejection.
    /// False means the caller must use its existing synchronous hook (also used by browser hosts).
    pub fn defer_picker(&mut self, request: PickerRequest, continuation: impl FnOnce(&mut Self, Vec<String>) -> Result<(), String> + 'static) -> bool {
        self.start_picker(request, None, |_| true, continuation)
    }

    /// Remember a closed/replaced draft even if another draft opens before the reply arrives.
    pub fn defer_picker_guarded(
        &mut self,
        draft: PickerDraft,
        request: PickerRequest,
        valid: impl Fn(&Self) -> bool + 'static,
        continuation: impl FnOnce(&mut Self, Vec<String>) -> Result<(), String> + 'static,
    ) -> bool {
        self.start_picker(request, Some(draft), valid, continuation)
    }

    fn start_picker(
        &mut self,
        request: PickerRequest,
        draft: Option<PickerDraft>,
        valid: impl Fn(&Self) -> bool + 'static,
        continuation: impl FnOnce(&mut Self, Vec<String>) -> Result<(), String> + 'static,
    ) -> bool {
        let Some(hook) = self.hooks.async_picker.as_mut() else { return false };
        self.picker.error = None;
        if self.picker.pending.is_some() {
            self.ui.status = "File dialog already open".into();
            self.picker.error = Some(self.ui.status.clone());
            return true;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let ctx = self.picker.ctx.clone();
        let reply = Box::new(move |result| {
            let _ = tx.try_send(result);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        match hook(request, reply) {
            Ok(()) => {
                self.picker.pending = Some(Pending {
                    rx,
                    pool: Arc::downgrade(&self.session.media),
                    continuation: Box::new(continuation),
                    valid: Box::new(valid),
                    stale: false,
                    draft,
                    project_changed: false,
                })
            }
            Err(error) => {
                self.picker.error = Some(error.clone());
                self.ui.status = error;
            }
        }
        true
    }

    /// Capture command parameters now; the selected paths are added when the picker completes.
    pub fn defer_picker_command(&mut self, request: PickerRequest, command: &str, mut params: serde_json::Value, multiple: bool) -> bool {
        let command = command.to_string();
        if !params.is_object() {
            params = serde_json::json!({});
        }
        if matches!(command.as_str(), "file.exportSelectionProject" | "file.exportAle") && params.get("items").is_none() {
            params["items"] = serde_json::json!(self.session.state.project_selection.iter().map(|i| i.0).collect::<Vec<_>>());
        }
        if command == "graphics.newFromFile" && params.get("time").is_none() {
            params["time"] = serde_json::json!(self.session.playhead().0);
        }
        if command == "captions.export"
            && params.get("track").is_none()
            && let Some(track) = filmcraft_engine::captions::track_param(&self.session, &params)
        {
            params["track"] = serde_json::json!(track.0);
        }
        let sequence = self.session.state.active_sequence;
        let export_project = matches!(command.as_str(), "source.exportFrame" | "file.exportFrame").then(|| self.session.project.clone());
        self.defer_picker(request, move |app, paths| {
            if export_project.as_ref().is_some_and(|project| !Arc::ptr_eq(project, &app.session.project)) {
                return Err("Frame export discarded because the project changed; retry Export Frame".into());
            }
            let caption_import = command == "file.import"
                && paths.iter().any(|path| {
                    std::path::Path::new(path).extension().and_then(|s| s.to_str()).is_some_and(|ext| {
                        ["srt", "vtt", "webvtt", "scc", "mcc", "stl", "ttml", "dfxp", "xml"].iter().any(|caption| ext.eq_ignore_ascii_case(caption))
                    })
                });
            if (caption_import
                || matches!(
                    command.as_str(),
                    "graphics.newFromFile"
                        | "captions.import"
                        | "captions.export"
                        | "file.exportAaf"
                        | "file.exportOmf"
                        | "lumetri.setInputLut"
                        | "lumetri.setLook"
                ))
                && app.session.state.active_sequence != sequence
            {
                return Err("File choice discarded because the active sequence changed".into());
            }
            if multiple {
                params["paths"] = serde_json::json!(paths);
            } else if let Some(path) = paths.first() {
                params["path"] = serde_json::json!(path);
                if export_project.is_some()
                    && let Some(extension) = std::path::Path::new(path).extension().and_then(|extension| extension.to_str())
                {
                    params["format"] = serde_json::json!(extension.to_ascii_lowercase());
                }
            }
            app.run_picker_command(&command, params)
        })
    }

    pub(crate) fn run_picker_command(&mut self, command: &str, params: serde_json::Value) -> Result<(), String> {
        let result = self.session.execute(command, params).map_err(|e| e.to_string())?;
        if matches!(command, "file.exportAaf" | "file.exportOmf") {
            self.ui.status = format!("Exported {}", result["path"].as_str().unwrap_or_default());
        }
        if let Some(errors) = result.get("errors").and_then(serde_json::Value::as_array).filter(|e| !e.is_empty()) {
            self.ui.status = errors.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join("; ");
        }
        Ok(())
    }

    /// Explicit replacement invalidation also covers a new draft with identical field values.
    pub(crate) fn replace_picker_draft(&mut self, draft: PickerDraft) {
        if let Some(pending) = self.picker.pending.as_mut().filter(|p| p.draft == Some(draft)) {
            pending.stale = true;
        }
    }

    pub(crate) fn replace_picker_project(&mut self) {
        if let Some(pending) = self.picker.pending.as_mut() {
            pending.project_changed = true;
        }
    }

    pub(crate) fn picker_started_result(&self, file_command: bool) -> Result<serde_json::Value, String> {
        if let Some(error) = self.picker.error.as_ref() {
            return Err(error.clone());
        }
        Ok(if file_command { serde_json::json!({"dialog": "filePicker", "pending": true}) } else { serde_json::json!({"pending": true}) })
    }

    pub fn picker_pending(&self) -> bool {
        self.picker.pending.is_some()
    }

    /// Control requests may start a dialog before the first UI frame is drawn.
    pub(crate) fn set_picker_context(&mut self, ctx: &egui::Context) {
        self.picker.ctx = Some(ctx.clone());
    }

    pub(crate) fn poll_picker(&mut self, ctx: &egui::Context) {
        self.set_picker_context(ctx);
        let invalid = self.picker.pending.as_ref().is_some_and(|pending| !(pending.valid)(self));
        if invalid && let Some(pending) = self.picker.pending.as_mut() {
            pending.stale = true;
        }
        let Some(pending) = self.picker.pending.as_ref() else { return };
        let result = match pending.rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("File dialog ended without a response".into()),
        };
        let Some(pending) = self.picker.pending.take() else { return };
        if pending.project_changed || !pending.pool.upgrade().is_some_and(|pool| Arc::ptr_eq(&pool, &self.session.media)) {
            self.ui.status = "File choice discarded because the project changed".into();
            return;
        }
        if pending.stale {
            self.ui.status = "File choice discarded because the dialog changed".into();
            return;
        }
        match result {
            Ok(paths) if paths.is_empty() => {} // Cancel leaves the draft and project untouched.
            Ok(paths) => {
                if let Err(error) = (pending.continuation)(self, paths) {
                    self.ui.status = error;
                }
            }
            Err(error) => self.ui.status = error,
        }
    }
}

#[cfg(test)]
#[path = "pickers_tests.rs"]
mod tests;
