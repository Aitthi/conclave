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

/// Render one `panic.log` entry. Pure so the shape is unit-testable; the hook
/// supplies live values. Ends with a `---` separator line so successive panics
/// in one file stay visually distinct.
fn format_panic_entry(
    timestamp: &str,
    thread: &str,
    location: &str,
    message: &str,
    backtrace: &str,
) -> String {
    format!(
        "[{timestamp}] panic on thread '{thread}' at {location}: {message}\n\
         backtrace:\n{backtrace}\n---\n"
    )
}

/// Diagnostics beachhead: append every Rust panic to
/// `~/Library/Application Support/Conclave/panic.log` (timestamp, thread,
/// location, payload, forced backtrace), then delegate to the previous hook.
/// A Finder-launched app has no visible stderr, so without this a panic —
/// including one that aborts the process by unwinding into FFI — leaves zero
/// diagnosable output (two undiagnosed app deaths on 2026-07-10, task
/// browser-crash-fix). Uses only std + existing deps (chrono, dirs); every
/// filesystem step is best-effort so the hook itself can never panic.
fn install_panic_beachhead() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("<unnamed>");
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "<unknown location>".to_owned());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_owned());
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let entry = format_panic_entry(&timestamp, thread, &location, &message, &backtrace);
        if let Some(data_dir) = dirs::data_dir() {
            let dir = data_dir.join("Conclave");
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("panic.log"))
            {
                use std::io::Write;
                let _ = f.write_all(entry.as_bytes());
            }
        }
        prev(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_beachhead();
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

            // Spawn the app-global loopback context proxy. It remains inactive
            // until its listener binds successfully.
            let proxy_state = std::sync::Arc::clone(&state);
            tauri::async_runtime::spawn(engine::runtime::ctx_proxy::serve(proxy_state));

            // Task stall + challenge-default timer (ADR 0008 Lane B) — a
            // single app-wide background loop, same spawn idiom as the UDS
            // server above.
            let timer_state = std::sync::Arc::clone(&state);
            tauri::async_runtime::spawn(engine::runtime::task_timer::run(timer_state));

            // One-shot skill-sidecar GC (D1): every launch, delete
            // `<data_dir>/Conclave/skills/<uuid>.md` files whose UUID has no
            // live `workspace_agent` row. Retroactively cleans machines that
            // accumulated orphans before this shipped; files only pile up
            // across launches, so once per boot is enough (no timer).
            let sweep_state = std::sync::Arc::clone(&state);
            tauri::async_runtime::spawn(async move {
                match engine::repo::workspace_agent::list_all_ids(&sweep_state.db).await {
                    Ok(ids) => {
                        let live: std::collections::HashSet<String> = ids.into_iter().collect();
                        let deleted = engine::agentctx::sweep_orphan_skill_sidecars(&live);
                        if deleted > 0 {
                            eprintln!("[skill] startup sweep deleted {deleted} orphan sidecar(s)");
                        }
                    }
                    Err(e) => eprintln!("[skill] startup sweep skipped — list_all_ids failed: {e}"),
                }
            });

            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![ipc, system_accent])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            // The design-host sidecar is a plain child process (no PTY), so
            // it does not die implicitly the way a PTY-backed CLI agent's
            // child does when its master fd closes — this is the one place
            // that must reach in and kill it explicitly before the process
            // exits (see `runtime::design_host::kill_on_exit`'s doc comment
            // for why this can't just wait for the async crash-monitor task).
            if let tauri::RunEvent::Exit = event {
                engine::runtime::design_host::kill_on_exit();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::format_panic_entry;

    #[test]
    fn panic_entry_carries_every_field_and_a_separator() {
        let entry = format_panic_entry(
            "2026-07-11T03:00:00+00:00",
            "tokio-runtime-worker",
            "src/engine/runtime/browser.rs:123:9",
            "called `Option::unwrap()` on a `None` value",
            "0: frame_one\n1: frame_two",
        );
        assert!(entry.starts_with("[2026-07-11T03:00:00+00:00] panic on thread 'tokio-runtime-worker'"));
        assert!(entry.contains("at src/engine/runtime/browser.rs:123:9: "));
        assert!(entry.contains("called `Option::unwrap()` on a `None` value"));
        assert!(entry.contains("backtrace:\n0: frame_one\n1: frame_two"));
        assert!(
            entry.ends_with("\n---\n"),
            "entries must end with a separator line so successive panics stay distinct"
        );
    }
}
