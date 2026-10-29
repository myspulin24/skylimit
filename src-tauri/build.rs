//! Build script: Tauri codegen plus an `allow-<command>` permission for every
//! app command. Each window gets only the commands it uses
//! (`capabilities/widget.json`, `capabilities/app.json`).

const COMMANDS: &[&str] = &[
    "get_snapshot",
    "get_settings",
    "set_settings",
    "set_click_through",
    "set_widget_transparency",
    "refresh_usage",
    "feedback_status",
    "feedback_send",
    "update_status",
    "update_check",
    "update_install",
    "open_app",
    "open_feedback",
    "clear_history",
    "claude_code_status",
    "claude_code_connect",
    "claude_code_disconnect",
    "autostart_status",
    "autostart_set",
];

fn main() {
    let attrs = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(e) = tauri_build::try_build(attrs) {
        eprintln!("tauri-build failed: {e:#}");
        std::process::exit(1);
    }
}
