//! Linux file choices run outside the window event loop.
use filmcraft_ui_egui::pickers::{PickerKind, PickerReply, PickerRequest};

pub fn start(request: PickerRequest, reply: PickerReply) -> Result<(), String> {
    std::thread::Builder::new()
        .name("filmcraft-file-picker".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pollster::block_on(async move {
                    let mut dialog = rfd::AsyncFileDialog::new();
                    if !request.extensions.is_empty() {
                        dialog = dialog.add_filter(request.filter, &request.extensions);
                    }
                    if !request.suggested.is_empty() {
                        dialog = dialog.set_file_name(request.suggested);
                    }
                    let files = match request.kind {
                        PickerKind::Files => dialog.pick_files().await.unwrap_or_default(),
                        PickerKind::File => dialog.pick_file().await.into_iter().collect(),
                        PickerKind::Folder => dialog.pick_folder().await.into_iter().collect(),
                        PickerKind::Save => dialog.save_file().await.into_iter().collect(),
                    };
                    files.into_iter().map(|f| f.path().to_string_lossy().into_owned()).collect()
                })
            }))
            .map_err(|_| "File dialog failed; try opening it again".to_string());
            reply(result);
        })
        .map(|_| ())
        .map_err(|e| format!("Cannot start file dialog: {e}"))
}
