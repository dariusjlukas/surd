use base64::Engine as _;
use tauri_plugin_dialog::DialogExt;

/// Save an exported artifact to a user-chosen path via a native Save dialog.
///
/// The web build downloads exports with the usual blob/anchor dance, but the
/// desktop webview can't be relied on to surface those downloads (notably
/// WKWebView on macOS silently swallows programmatic blob downloads), so all
/// of the frontend's export sites route through this command instead. `data`
/// is the file text, or its base64 when `base64` is set — plot PNG snapshots
/// arrive as the base64 tail of a `data:` URL. Returns `false` when the user
/// cancels the dialog.
#[tauri::command]
async fn save_export(
    app: tauri::AppHandle,
    suggested_name: String,
    data: String,
    base64: bool,
) -> Result<bool, String> {
    let bytes = if base64 {
        base64::engine::general_purpose::STANDARD
            .decode(data.as_bytes())
            .map_err(|e| e.to_string())?
    } else {
        data.into_bytes()
    };

    // Runs on the async command thread, off the main/event thread — the
    // blocking dialog marshals itself to the UI thread internally, so this is
    // the supported way to await a native file picker from a command.
    let chosen = app
        .dialog()
        .file()
        .set_file_name(&suggested_name)
        .blocking_save_file();

    match chosen {
        Some(path) => {
            let path = path.into_path().map_err(|e| e.to_string())?;
            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![save_export])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
