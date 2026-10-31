//! Skylimit desktop shell (Tauri 2, Windows). All domain logic lives in
//! `usage_core`; this crate starts the core, forwards its snapshots to the
//! windows and the tray, persists settings and owns the two windows.
//!
//! ```text
//!  usage_core::Core ──watch──▶ snapshot pump thread ──emit "snapshot"──▶ widget, app
//!        ▲                              └──────────────▶ tray (icon color, tooltip)
//!        └── commands (get/set settings, clear history) ◀── UI
//! ```

mod autostart;
mod claude_code;
mod commands;
mod feedback;
mod settings;
mod shell;
mod tray;
mod updater;
mod window_state;
mod windows;

use tauri::{AppHandle, Emitter, Manager, RunEvent};
use usage_core::{watch, CodexSource, Config, Core, FxSource, Snapshot};

use crate::settings::DataPaths;
use crate::shell::Shell;
use crate::window_state::WindowTracker;
use crate::windows::Placements;

/// `skylimit statusline`, run by Claude Code after every response.
pub fn statusline() -> i32 {
    claude_code::run_cli()
}

pub fn run() {
    let built = tauri::Builder::default()
        // Must be the first plugin: a second launch only reveals the widget.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::reveal_widget(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            setup(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_settings,
            commands::set_settings,
            commands::set_click_through,
            commands::set_widget_transparency,
            commands::refresh_usage,
            feedback::feedback_status,
            feedback::feedback_send,
            commands::open_app,
            commands::open_feedback,
            commands::clear_history,
            commands::accounts,
            commands::history,
            claude_code::claude_code_status,
            claude_code::claude_code_connect,
            claude_code::claude_code_disconnect,
            autostart::autostart_status,
            autostart::autostart_set,
            updater::update_status,
            updater::update_check,
            updater::update_install,
        ])
        .build(tauri::generate_context!());

    let app = match built {
        Ok(app) => app,
        Err(e) => {
            eprintln!("Skylimit failed to start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|handle, event| match event {
        // Closing the last window must not end a tray app; only "Konec" does.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::Exit => {
            if let Some(placements) = handle.try_state::<Placements>() {
                placements.save_all();
            }
            if let Some(shell) = handle.try_state::<Shell>() {
                shell.shutdown();
            }
        }
        _ => {}
    });
}

fn setup(app: &AppHandle) -> tauri::Result<()> {
    let paths = DataPaths::resolve(app);
    let settings = settings::load(&paths.settings());

    let mut config = Config::new(paths.claude_projects.clone(), paths.db(), paths.diag());
    config.codex = CodexSource::official();
    config.t3_logs_dir = Some(paths.t3_logs.clone());
    config.t3_userdata_dir = Some(paths.t3_userdata.clone());
    config.claude_code_dir = Some(paths.claude_code());
    config.claude_config_file = Some(paths.claude_global.clone());
    config.settings = settings.clone();
    config.fx = FxSource::Cnb;
    config.pricing_refresh = false;
    let core = Core::start(config);
    let snapshots = core.subscribe();

    app.manage(Placements {
        widget: WindowTracker::new(paths.widget_placement()),
        app: WindowTracker::new(paths.app_placement()),
    });
    app.manage(Shell::new(core, paths));
    app.manage(updater::Updates::new(app.package_info().version.to_string()));
    updater::spawn_auto(app.clone());

    tray::create(app, settings.click_through)?;
    windows::create_widget(app, settings.click_through)?;
    spawn_snapshot_pump(app.clone(), snapshots);
    Ok(())
}

/// Every new snapshot goes to all windows (event "snapshot") and the tray.
/// Ends when the core shuts down and drops its sender.
fn spawn_snapshot_pump(app: AppHandle, mut rx: watch::Receiver<Snapshot>) {
    let spawned = std::thread::Builder::new()
        .name("snapshot-pump".into())
        .spawn(move || {
            let first = rx.borrow_and_update().clone();
            tray::update(&app, &first);
            while tauri::async_runtime::block_on(rx.changed()).is_ok() {
                let snapshot = rx.borrow_and_update().clone();
                if let Err(e) = app.emit("snapshot", &snapshot) {
                    eprintln!("cannot emit snapshot: {e}");
                }
                tray::update(&app, &snapshot);
            }
        });
    if let Err(e) = spawned {
        eprintln!("cannot start the snapshot thread: {e}");
    }
}
