mod engine;

use engine::{router, AppState};
use serde_json::Value;
use tauri::Manager;

/// Single IPC entry-point for the Tauri invoke bridge.
///
/// The frontend calls `invoke("ipc", { cmd, payload })`.
/// A future Unix-Domain-Socket server will call `engine::router::dispatch`
/// directly, bypassing this thin wrapper.
#[tauri::command]
async fn ipc(
    state: tauri::State<'_, AppState>,
    cmd: String,
    payload: Value,
) -> Result<Value, String> {
    router::dispatch(&state, &cmd, payload)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .setup(|app| {
            // Wire the AppHandle into AppState so that bus::emit helpers and
            // state.emit(...) can push events to the UI from any async context.
            let state = app.state::<AppState>();
            state.set_app(app.handle().clone());
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![ipc])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
