//! Managed state of the shell: the running core, data paths and the pause
//! flag, plus the settings operations shared by commands and the tray menu.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock, RwLockReadGuard};

use tauri::{AppHandle, Manager};
use usage_core::{CoreHandle, Settings, Snapshot, WIDGET_TRANSPARENCY_MAX};

use crate::settings::{self, DataPaths};
use crate::{tray, windows};

pub struct Shell {
    /// `None` once the app is exiting (`CoreHandle::shutdown` takes it).
    core: RwLock<Option<CoreHandle>>,
    pub paths: DataPaths,
    /// Serialises writes of `settings.json` (UI and tray can race).
    save_lock: Mutex<()>,
    paused: AtomicBool,
}

impl Shell {
    pub fn new(core: CoreHandle, paths: DataPaths) -> Self {
        Shell {
            core: RwLock::new(Some(core)),
            paths,
            save_lock: Mutex::new(()),
            paused: AtomicBool::new(false),
        }
    }

    /// Read access to the core; fails only while the app is shutting down.
    pub fn core(&self) -> Result<CoreGuard<'_>, String> {
        let guard = self.core.read().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            return Err("Aplikace se ukončuje.".into());
        }
        Ok(CoreGuard(guard))
    }

    /// Persist first, then hand the settings to the core.
    fn store(&self, core: &CoreHandle, settings: Settings) -> Result<(), String> {
        {
            let _write = self.save_lock.lock().unwrap_or_else(|e| e.into_inner());
            settings::save(&self.paths.settings(), &settings)
                .map_err(|e| format!("Nastavení se nepodařilo uložit: {e}"))?;
        }
        core.set_settings(settings);
        Ok(())
    }

    /// Stop the core (joins its threads and ends the Codex app-server).
    pub fn shutdown(&self) {
        let core = self.core.write().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(core) = core {
            core.shutdown();
        }
    }
}

pub struct CoreGuard<'a>(RwLockReadGuard<'a, Option<CoreHandle>>);

impl std::ops::Deref for CoreGuard<'_> {
    type Target = CoreHandle;

    fn deref(&self) -> &CoreHandle {
        // Constructed only when the option is `Some`.
        self.0.as_ref().expect("core present")
    }
}

/// Save new settings, pass them to the core and apply what the shell owns
/// (click-through on the widget, the tray check mark).
pub fn set_settings(app: &AppHandle, settings: Settings) -> Result<(), String> {
    let shell = app.state::<Shell>();
    let click_through = settings.click_through;
    {
        let core = shell.core()?;
        shell.store(&core, settings)?;
    }
    windows::apply_click_through(app, click_through);
    tray::set_click_through_checked(app, click_through);
    Ok(())
}

/// Toggle click-through from the app window or the tray; persisted.
pub fn set_click_through(app: &AppHandle, on: bool) -> Result<(), String> {
    let shell = app.state::<Shell>();
    {
        let core = shell.core()?;
        let mut settings = core.settings();
        settings.click_through = on;
        shell.store(&core, settings)?;
    }
    windows::apply_click_through(app, on);
    tray::set_click_through_checked(app, on);
    Ok(())
}

/// Widget background transparency from the settings slider; persisted. The
/// widget follows through the next snapshot (the core republishes on change).
pub fn set_widget_transparency(app: &AppHandle, value: u8) -> Result<(), String> {
    let shell = app.state::<Shell>();
    let core = shell.core()?;
    let mut settings = core.settings();
    settings.widget_transparency = value.min(WIDGET_TRANSPARENCY_MAX);
    shell.store(&core, settings)
}

pub fn current_snapshot(app: &AppHandle) -> Option<Snapshot> {
    let shell = app.try_state::<Shell>()?;
    let core = shell.core().ok()?;
    Some(core.snapshot())
}

pub fn is_paused(app: &AppHandle) -> bool {
    app.try_state::<Shell>()
        .map(|s| s.paused.load(Ordering::Relaxed))
        .unwrap_or(false)
}

/// Returns the previous value.
pub fn set_paused_flag(app: &AppHandle, on: bool) -> bool {
    app.try_state::<Shell>()
        .map(|s| s.paused.swap(on, Ordering::Relaxed))
        .unwrap_or(false)
}
