mod engine;
mod menu;
mod sysaccent;

use engine::{router, AppState};
use serde_json::Value;
use tauri::Manager;

/// GUI-only host concern (the live macOS accent color), deliberately kept out of
/// the IPC router — the UDS/CLI front-door has no business reading it.
#[tauri::command]
fn system_accent() -> Option<String> {
    sysaccent::system_accent()
}

/// Single IPC entry-point for the Tauri invoke bridge.
///
/// The frontend calls `invoke("ipc", { cmd, payload })`.
/// A future Unix-Domain-Socket server will call `engine::router::dispatch`
/// directly, bypassing this thin wrapper.
#[tauri::command]
async fn ipc(
    state: tauri::State<'_, std::sync::Arc<AppState>>,
    cmd: String,
    payload: Value,
) -> Result<Value, String> {
    // `&state` coerces `&State<Arc<AppState>>` → `&AppState` via chained Deref.
    router::dispatch(&state, &cmd, payload)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .menu(menu::build)
        .on_menu_event(|app, event| menu::on_event(app, event.id().as_ref()))
        .manage(std::sync::Arc::new(AppState::new()))
        .setup(|app| {
            // Wire the AppHandle into AppState so that bus::emit helpers and
            // state.emit(...) can push events to the UI from any async context.
            let state = app.state::<std::sync::Arc<AppState>>();
            state.set_app(app.handle().clone());

            // Spawn the Unix-Domain-Socket server (unix only): a second
            // front-door onto the same command router the GUI uses. Cloning the
            // managed Arc gives the server its own owned handle to AppState.
            #[cfg(unix)]
            {
                let server_state = std::sync::Arc::clone(&state);
                tauri::async_runtime::spawn(engine::uds::serve(
                    server_state,
                    engine::uds::socket_path(),
                ));
            }

            // Task stall + challenge-default timer (ADR 0008 Lane B) — a
            // single app-wide background loop, same spawn idiom as the UDS
            // server above.
            let timer_state = std::sync::Arc::clone(&state);
            tauri::async_runtime::spawn(engine::runtime::task_timer::run(timer_state));

            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![ipc, system_accent])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
