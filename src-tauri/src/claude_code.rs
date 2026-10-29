//! Claude Code in a terminal (iTerm2, Terminal, Windows Terminal, …) as a
//! source of Claude's limits, through its status line.
//!
//! Connecting sets one key in Claude Code's user settings
//! (`~/.claude/settings.json`):
//!
//! ```text
//!  "statusLine": { "type": "command", "command": "'<this app>' statusline" }
//! ```
//!
//! Claude Code then starts Skylimit after every response with the session as
//! JSON on stdin. [`run_cli`] keeps only the limits
//! (`usage_core::claude_code`) and prints the status line. A status line the
//! user already had is saved in Skylimit's data folder and keeps running:
//! Skylimit hands it the same input and prints its output instead of its own.
//! Disconnecting puts it back.
//!
//! The settings file changes only on the user's click and only in the
//! `statusLine` key. It is written through a temporary file and a rename (a
//! symlinked file is written where it points), and the previous version is
//! copied to Skylimit's data folder first.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};
use usage_core::claude_code;

use crate::settings::DataPaths;
use crate::shell::Shell;

const KEY: &str = "statusLine";
const PREVIOUS: &str = "previous-statusline.json";
const BACKUP: &str = "settings-backup.json";
/// A status line that takes longer is cut off (Claude Code would cancel it
/// on the next update anyway).
const PREVIOUS_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeStatus {
    /// "missing": no Claude Code folder; "off"; "on"; "moved": connected to
    /// a Skylimit at another path (the app moved, connect again).
    pub state: &'static str,
    pub settings_path: String,
    /// The user has a status line of their own that Skylimit runs ("on") or
    /// would keep running ("off").
    pub own_status_line: bool,
    /// When Claude Code last delivered limits (epoch s).
    pub last_reading_at: Option<i64>,
    /// Why connecting cannot work from here, else empty.
    pub problem: String,
}

// ---------------------------------------------------------------- CLI

/// `skylimit statusline`: Claude Code's status line command. Never fails
/// loudly: whatever goes wrong, Claude Code only sees a shorter status line.
pub fn run_cli() -> i32 {
    let mut input = Vec::new();
    let _ = io::stdin().take(claude_code::MAX_INPUT as u64).read_to_end(&mut input);
    let paths = DataPaths::from_env();
    let dir = paths.claude_code();
    let recorded = claude_code::parse(&String::from_utf8_lossy(&input))
        .and_then(|sample| claude_code::record(&dir, &sample, claude_code::now()).ok());

    let mut out = io::stdout().lock();
    if let Some(command) = previous_command(&dir) {
        if let Some(output) = run_previous(&command, &input) {
            let _ = out.write_all(&output);
            let _ = out.flush();
            return 0;
        }
    }
    if let Some(r) = recorded {
        let text = claude_code::status_text(r.five_hour.as_ref(), r.seven_day.as_ref());
        if !text.is_empty() {
            let _ = writeln!(out, "{text}");
        }
    }
    let _ = out.flush();
    0
}

fn previous_command(dir: &Path) -> Option<String> {
    let saved: Value = serde_json::from_slice(&fs::read(dir.join(PREVIOUS)).ok()?).ok()?;
    let command = saved.get("command")?.as_str()?.trim().to_owned();
    (!command.is_empty() && !is_ours(&command)).then_some(command)
}

/// Runs the user's own status line the way Claude Code would: `sh` on macOS
/// and Linux, Git Bash on Windows when installed, PowerShell otherwise.
fn run_previous(command: &str, input: &[u8]) -> Option<Vec<u8>> {
    let mut cmd = shell_for(command);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Skylimit has no console; the shell must not open one either.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < PREVIOUS_TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    reader.join().ok()
}

#[cfg(not(windows))]
fn shell_for(command: &str) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(command);
    cmd
}

#[cfg(windows)]
fn shell_for(command: &str) -> Command {
    match git_bash() {
        Some(bash) => {
            let mut cmd = Command::new(bash);
            cmd.arg("-c").arg(command);
            cmd
        }
        None => {
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-NonInteractive", "-Command", command]);
            cmd
        }
    }
}

/// Where Claude Code finds Git Bash: `CLAUDE_CODE_GIT_BASH_PATH`, next to
/// `git.exe` on PATH, or the default install.
#[cfg(windows)]
fn git_bash() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    let from_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("git.exe"))
            .find(|git| git.is_file())
            .and_then(|git| Some(git.parent()?.parent()?.join("bin").join("bash.exe")))
            .filter(|bash| bash.is_file())
    });
    from_path.or_else(|| {
        let default = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
        default.is_file().then_some(default)
    })
}

// ---------------------------------------------------------------- settings

fn settings_file(paths: &DataPaths) -> PathBuf {
    paths.claude_config.join("settings.json")
}

/// Is this Skylimit's status line command (any path, any version)?
fn is_ours(command: &str) -> bool {
    let c = command.trim();
    c.to_ascii_lowercase().contains("skylimit") && c.ends_with(" statusline")
}

fn read_settings(path: &Path) -> Result<Map<String, Value>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("Nastavení Claude Code nejde přečíst: {e}")),
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(format!(
            "{} není platný JSON, Skylimit ho proto neupraví. Oprav ho a zkus to znovu.",
            path.display()
        )),
    }
}

fn write_settings(path: &Path, settings: &Map<String, Value>) -> Result<(), String> {
    let fail = |e: io::Error| format!("Nastavení Claude Code se nepodařilo uložit: {e}");
    // A symlinked settings file (dotfiles) stays a symlink.
    let target = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path).map_err(fail)?,
        _ => path.to_path_buf(),
    };
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| fail(io::Error::other(e)))?;
    text.push('\n');
    let tmp = target.with_extension("json.skylimit-tmp");
    fs::write(&tmp, text).map_err(fail)?;
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        fail(e)
    })
}

/// Command Claude Code runs. A path with spaces is quoted for `sh`; on
/// Windows it is shortened to its 8.3 form instead, because Claude Code may
/// run the command in PowerShell, where a quoted program needs `&` and `sh`
/// does not accept that.
fn our_command() -> Result<String, String> {
    let exe = app_path().map_err(|e| format!("Cesta ke Skylimitu není známá: {e}"))?;
    let text = exe.to_string_lossy().into_owned();
    if text.contains("/AppTranslocation/") {
        return Err("Skylimit běží z dočasného umístění. Přesuň ho do složky Aplikace, spusť ho znovu a pak připoj Claude Code.".into());
    }
    Ok(format!("{} statusline", quote_program(&text)))
}

#[cfg(not(windows))]
fn app_path() -> io::Result<PathBuf> {
    // An AppImage runs from a temporary mount; the file itself stays put.
    if let Some(image) = std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(image);
    }
    std::env::current_exe()
}

#[cfg(windows)]
fn app_path() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(if exe.to_string_lossy().contains(' ') { short_path(&exe) } else { exe })
}

#[cfg(windows)]
fn short_path(path: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated; a null buffer of size 0 asks for the length.
    let needed = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        return path.to_path_buf();
    }
    let mut buf = vec![0u16; needed as usize];
    // SAFETY: `buf` holds `needed` u16s, as the call is told.
    let written = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), needed) };
    if written == 0 || written >= needed {
        return path.to_path_buf();
    }
    buf.truncate(written as usize);
    PathBuf::from(OsString::from_wide(&buf))
}

#[cfg(not(windows))]
fn quote_program(path: &str) -> String {
    let plain = path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c));
    if plain {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}

#[cfg(windows)]
fn quote_program(path: &str) -> String {
    // Forward slashes: Git Bash would eat backslashes (Claude Code docs).
    let path = path.trim_start_matches(r"\\?\").replace('\\', "/");
    if path.contains(' ') {
        format!("\"{path}\"")
    } else {
        path
    }
}

fn own_line(value: Option<&Value>) -> bool {
    value
        .and_then(|v| v.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|c| !c.trim().is_empty() && !is_ours(c))
}

pub fn status(paths: &DataPaths) -> ClaudeCodeStatus {
    let path = settings_file(paths);
    let dir = paths.claude_code();
    let mut status = ClaudeCodeStatus {
        state: "off",
        settings_path: path.display().to_string(),
        own_status_line: false,
        last_reading_at: claude_code::last_reading_at(&dir),
        problem: String::new(),
    };
    if !paths.claude_config.is_dir() {
        status.state = "missing";
        return status;
    }
    let ours = our_command();
    if let Err(e) = &ours {
        status.problem = e.clone();
    }
    let current = read_settings(&path).ok().and_then(|mut s| s.remove(KEY));
    let command = current.as_ref().and_then(|v| v.get("command")).and_then(Value::as_str);
    match command {
        Some(c) if is_ours(c) => {
            status.state = if ours.as_deref().is_ok_and(|o| o == c.trim()) { "on" } else { "moved" };
            status.own_status_line = previous_command(&dir).is_some();
        }
        _ => status.own_status_line = own_line(current.as_ref()),
    }
    status
}

pub fn connect(paths: &DataPaths) -> Result<ClaudeCodeStatus, String> {
    if !paths.claude_config.is_dir() {
        return Err(format!(
            "Claude Code tu chybí (není složka {}). Nainstaluj ho, jednou spusť a zkus to znovu.",
            paths.claude_config.display()
        ));
    }
    let command = our_command()?;
    let path = settings_file(paths);
    let mut settings = read_settings(&path)?;
    let dir = paths.claude_code();
    fs::create_dir_all(&dir).map_err(|e| format!("Složku {} nejde založit: {e}", dir.display()))?;

    let current = settings.get(KEY).cloned();
    let current_is_ours = current
        .as_ref()
        .and_then(|v| v.get("command"))
        .and_then(Value::as_str)
        .is_some_and(is_ours);
    if !current_is_ours {
        match &current {
            Some(previous) => fs::write(dir.join(PREVIOUS), serde_json::to_vec_pretty(previous).unwrap_or_default())
                .map_err(|e| format!("Původní stavový řádek se nepodařilo uschovat: {e}"))?,
            None => {
                let _ = fs::remove_file(dir.join(PREVIOUS));
            }
        }
    }
    // Keep the user's padding and timers; only the command changes.
    let mut line = match current {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    line.insert("type".into(), Value::from("command"));
    line.insert("command".into(), Value::from(command));
    settings.insert(KEY.into(), Value::Object(line));

    if path.is_file() {
        let _ = fs::copy(&path, dir.join(BACKUP));
    }
    write_settings(&path, &settings)?;
    Ok(status(paths))
}

pub fn disconnect(paths: &DataPaths) -> Result<ClaudeCodeStatus, String> {
    let path = settings_file(paths);
    let dir = paths.claude_code();
    let mut settings = read_settings(&path)?;
    let ours = settings
        .get(KEY)
        .and_then(|v| v.get("command"))
        .and_then(Value::as_str)
        .is_some_and(is_ours);
    // A status line the user has changed since is left alone.
    if ours {
        let previous = fs::read(dir.join(PREVIOUS))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        match previous {
            Some(previous) => settings.insert(KEY.into(), previous),
            None => settings.remove(KEY),
        };
        write_settings(&path, &settings)?;
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("Složku {} nejde smazat: {e}", dir.display()))?;
    }
    Ok(status(paths))
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub async fn claude_code_status(app: AppHandle) -> Result<ClaudeCodeStatus, String> {
    Ok(status(&app.state::<Shell>().paths))
}

#[tauri::command]
pub async fn claude_code_connect(app: AppHandle) -> Result<ClaudeCodeStatus, String> {
    let shell = app.state::<Shell>();
    let result = connect(&shell.paths)?;
    // The limits reader notices the folder now instead of in 5 s.
    if let Ok(core) = shell.core() {
        core.refresh();
    }
    Ok(result)
}

#[tauri::command]
pub async fn claude_code_disconnect(app: AppHandle) -> Result<ClaudeCodeStatus, String> {
    let shell = app.state::<Shell>();
    let result = disconnect(&shell.paths)?;
    if let Ok(core) = shell.core() {
        core.refresh();
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(tmp: &Path) -> DataPaths {
        let claude_config = tmp.join(".claude");
        fs::create_dir_all(&claude_config).unwrap();
        DataPaths {
            root: tmp.join("skylimit-data"),
            claude_projects: claude_config.join("projects"),
            claude_config,
            t3_logs: tmp.join("t3"),
        }
    }

    fn settings(p: &DataPaths) -> Value {
        serde_json::from_str(&fs::read_to_string(settings_file(p)).unwrap()).unwrap()
    }

    #[test]
    fn recognises_its_own_command_at_any_path() {
        assert!(is_ours("/Applications/Skylimit.app/Contents/MacOS/skylimit statusline"));
        assert!(is_ours("'/Users/a b/Skylimit.app/Contents/MacOS/skylimit' statusline"));
        assert!(is_ours("C:/Users/micha/AppData/Local/Skylimit/skylimit.exe statusline "));
        assert!(!is_ours("~/.claude/statusline.sh"));
        assert!(!is_ours("npx ccstatusline@latest"));
    }

    #[cfg(not(windows))]
    #[test]
    fn quotes_paths_for_sh() {
        assert_eq!(quote_program("/Applications/Skylimit.app/Contents/MacOS/skylimit"), "/Applications/Skylimit.app/Contents/MacOS/skylimit");
        assert_eq!(quote_program("/Users/a b/it's/skylimit"), r"'/Users/a b/it'\''s/skylimit'");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_use_forward_slashes() {
        assert_eq!(quote_program(r"C:\Users\micha\AppData\Local\Skylimit\skylimit.exe"), "C:/Users/micha/AppData/Local/Skylimit/skylimit.exe");
        assert_eq!(quote_program(r"\\?\C:\Program Files\Skylimit\skylimit.exe"), "\"C:/Program Files/Skylimit/skylimit.exe\"");
    }

    #[test]
    fn connect_and_disconnect_without_a_previous_status_line() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path());
        fs::write(settings_file(&p), "{\n  \"model\": \"opus\",\n  \"permissions\": { \"allow\": [\"Bash(ls)\"] }\n}\n").unwrap();
        assert_eq!(status(&p).state, "off");
        assert!(!status(&p).own_status_line);

        let on = connect(&p).unwrap();
        assert_eq!(on.state, "on");
        let s = settings(&p);
        assert_eq!(s["statusLine"]["type"], "command");
        assert!(is_ours(s["statusLine"]["command"].as_str().unwrap()));
        assert_eq!(s["model"], "opus", "other keys stay");
        let text = fs::read_to_string(settings_file(&p)).unwrap();
        assert!(text.find("\"model\"").unwrap() < text.find("\"statusLine\"").unwrap(), "key order kept");
        assert!(p.claude_code().is_dir(), "the core sees the connection by this folder");
        assert!(p.claude_code().join(BACKUP).is_file());

        let off = disconnect(&p).unwrap();
        assert_eq!(off.state, "off");
        let s = settings(&p);
        assert!(s.get("statusLine").is_none());
        assert_eq!(s["permissions"]["allow"][0], "Bash(ls)");
        assert!(!p.claude_code().exists());
    }

    #[test]
    fn an_own_status_line_is_kept_running_and_restored() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path());
        let mine = serde_json::json!({ "type": "command", "command": "~/.claude/statusline.sh", "padding": 2 });
        fs::write(settings_file(&p), serde_json::to_string(&serde_json::json!({ "statusLine": mine })).unwrap()).unwrap();
        assert!(status(&p).own_status_line);

        let on = connect(&p).unwrap();
        assert!(on.own_status_line);
        let s = settings(&p);
        assert_eq!(s["statusLine"]["padding"], 2, "padding kept");
        assert_eq!(previous_command(&p.claude_code()).as_deref(), Some("~/.claude/statusline.sh"));

        // Connecting again (the app moved) does not save Skylimit as "previous".
        connect(&p).unwrap();
        assert_eq!(previous_command(&p.claude_code()).as_deref(), Some("~/.claude/statusline.sh"));

        disconnect(&p).unwrap();
        assert_eq!(settings(&p)["statusLine"], mine);
    }

    #[test]
    fn a_status_line_changed_by_hand_is_left_alone_on_disconnect() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path());
        connect(&p).unwrap();
        let theirs = serde_json::json!({ "statusLine": { "type": "command", "command": "echo hi" } });
        fs::write(settings_file(&p), theirs.to_string()).unwrap();
        disconnect(&p).unwrap();
        assert_eq!(settings(&p), theirs);
    }

    #[test]
    fn broken_settings_are_not_touched_and_missing_claude_code_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path());
        fs::write(settings_file(&p), "{ \"model\": ").unwrap();
        assert!(connect(&p).unwrap_err().contains("není platný JSON"));
        assert_eq!(fs::read_to_string(settings_file(&p)).unwrap(), "{ \"model\": ");

        let none = DataPaths {
            claude_config: tmp.path().join("nowhere"),
            ..p.clone()
        };
        assert_eq!(status(&none).state, "missing");
        assert!(connect(&none).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_settings_file_stays_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path());
        let real = tmp.path().join("dotfiles-settings.json");
        fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, settings_file(&p)).unwrap();
        connect(&p).unwrap();
        assert!(fs::symlink_metadata(settings_file(&p)).unwrap().file_type().is_symlink());
        assert!(fs::read_to_string(&real).unwrap().contains("statusLine"));
    }

    #[cfg(unix)]
    #[test]
    fn runs_the_previous_status_line_with_the_same_input() {
        let out = run_previous("cat; printf ' ok'", b"{\"x\":1}").unwrap();
        assert_eq!(out, b"{\"x\":1} ok");
    }
}
