//! Tauri commands from `docs/snapshot-contract.md`. All async so none of
//! them runs on (or blocks) the main thread; `open_app` creates a window,
//! which must not happen in a synchronous command on Windows.

use tauri::{AppHandle, Manager, State};
use usage_core::{Settings, Snapshot};

use crate::shell::{self, Shell};
use crate::windows;

#[tauri::command]
pub async fn get_snapshot(state: State<'_, Shell>) -> Result<Snapshot, String> {
    Ok(state.core()?.snapshot())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, Shell>) -> Result<Settings, String> {
    Ok(state.core()?.settings())
}

#[tauri::command]
pub async fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    shell::set_settings(&app, settings)
}

#[tauri::command]
pub async fn set_click_through(app: AppHandle, on: bool) -> Result<(), String> {
    shell::set_click_through(&app, on)
}

#[tauri::command]
pub async fn set_widget_transparency(app: AppHandle, value: u8) -> Result<(), String> {
    shell::set_widget_transparency(&app, value)
}

/// "Obnovit" in the widget: every source is read now; data follows as snapshots.
#[tauri::command]
pub async fn refresh_usage(state: State<'_, Shell>) -> Result<(), String> {
    state.core()?.refresh();
    Ok(())
}

/// The widget's speech-bubble button: the app window with the feedback dialog.
#[tauri::command]
pub async fn open_feedback(app: AppHandle) -> Result<(), String> {
    windows::open_feedback(&app).map_err(|e| format!("Okno se zpětnou vazbou se nepodařilo otevřít: {e}"))
}

#[tauri::command]
pub async fn open_app(app: AppHandle) -> Result<(), String> {
    windows::open_app(&app).map_err(|e| format!("Okno aplikace se nepodařilo otevřít: {e}"))
}

/// Blocks until the core has emptied the tables (up to 10 s), so it runs on
/// the blocking pool instead of an async worker.
#[tauri::command]
pub async fn clear_history(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let shell = app.state::<Shell>();
        let core = shell.core()?;
        core.clear_history();
        Ok(())
    })
    .await
    .map_err(|e| format!("Mazání historie selhalo: {e}"))?
}
