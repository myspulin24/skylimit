// No console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Claude Code's status line command (Settings › Claude Code): read the
    // limits from stdin and exit, before any window or single-instance check.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "statusline") {
        std::process::exit(skylimit_lib::statusline());
    }
    skylimit_lib::run();
}
