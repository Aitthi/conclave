//! Native macOS menu bar + global accelerators.
//!
//! The menu is built once at startup. Custom items emit their id on the `menu`
//! event; the frontend (`AppShell`) listens and maps each id to an action
//! (open the builder, toggle the blackboard, switch theme, …). Predefined items
//! (Cut/Copy/Paste, Quit, Minimize) are handled natively by the OS — they make
//! the standard ⌘C/⌘V/⌘Q shortcuts work in every text field for free.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

/// Build the application menu. Called from the Tauri builder's `.menu(...)`.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    // ── Conclave (app menu) ────────────────────────────────────────────────
    let app_menu = Submenu::with_items(
        app,
        "Conclave",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("Conclave"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    // ── File ───────────────────────────────────────────────────────────────
    let new_agent = MenuItem::with_id(app, "new_agent", "New Agent", true, Some("CmdOrCtrl+N"))?;
    let open_library =
        MenuItem::with_id(app, "library", "Agent Library", true, Some("CmdOrCtrl+L"))?;
    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &new_agent,
            &open_library,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    // ── Edit (native — gives every input ⌘X/⌘C/⌘V/⌘A/⌘Z) ───────────────────
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    // ── View (Appearance submenu + Blackboard) ─────────────────────────────
    let theme_system = MenuItem::with_id(app, "theme_system", "Match System", true, None::<&str>)?;
    let theme_light = MenuItem::with_id(app, "theme_light", "Light", true, None::<&str>)?;
    let theme_dark = MenuItem::with_id(app, "theme_dark", "Dark", true, None::<&str>)?;
    let appearance = Submenu::with_items(
        app,
        "Appearance",
        true,
        &[&theme_system, &theme_light, &theme_dark],
    )?;
    let toggle_blackboard = MenuItem::with_id(
        app,
        "toggle_blackboard",
        "Toggle Blackboard",
        true,
        Some("CmdOrCtrl+B"),
    )?;
    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &appearance,
            &PredefinedMenuItem::separator(app)?,
            &toggle_blackboard,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;

    // ── Window ─────────────────────────────────────────────────────────────
    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    Menu::with_items(
        app,
        &[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu],
    )
}

/// Forward a custom menu-item click to the frontend as a `menu` event carrying
/// the item id (e.g. `"new_agent"`, `"theme_dark"`). Predefined items never
/// reach here — the OS handles them.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let _ = app.emit("menu", id.to_string());
}
