//! Start Skylimit when the user logs in (Nastavení › Widget).
//!
//! The system's own startup list is the only record, so the switch also
//! shows what the user changed there since:
//!
//! ```text
//!  Windows  HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//!             "Skylimit" = "\"…\skylimit.exe\""
//!           Task Manager › Startup apps turns it off in …\Explorer\StartupApproved\Run
//!  macOS    ~/Library/LaunchAgents/cz.myspulin.skylimit.plist   (RunAtLoad)
//!  Linux    ~/.config/autostart/cz.myspulin.skylimit.desktop    (the AppImage file itself)
//! ```
//!
//! The entry changes only on the user's click. The Windows uninstaller
//! removes the "Skylimit" value itself (Tauri's NSIS template, not on
//! updates); on macOS and Linux an entry left behind points to a missing
//! file and the system skips it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Runtime};
#[cfg(not(windows))]
use tauri::Manager;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    /// "on"; "off" (also when turned off in the system's own list);
    /// "moved": the system starts a Skylimit at another path.
    pub state: &'static str,
    /// Why it cannot be turned on from here, else empty.
    pub problem: String,
}

/// What the system starts at login under Skylimit's name.
#[derive(Debug, PartialEq)]
enum Found {
    Nothing,
    /// Listed, but turned off in Task Manager or the desktop's startup list.
    TurnedOff,
    /// Exactly what this copy of Skylimit writes.
    Ours,
    /// Another path (the app moved) or an entry edited by hand.
    Elsewhere,
}

/// One startup entry: where it lives and what Skylimit writes there.
trait Entry {
    fn find(&self) -> io::Result<Found>;
    fn add(&self) -> io::Result<()>;
    fn remove(&self) -> io::Result<()>;
}

fn status(entry: &dyn Entry, problem: &str) -> AutostartStatus {
    let mut problem = problem.to_owned();
    let state = match entry.find() {
        Ok(Found::Ours) => "on",
        Ok(Found::Elsewhere) => "moved",
        Ok(Found::Nothing | Found::TurnedOff) => "off",
        Err(e) => {
            problem = format!("Spouštění po přihlášení nejde zjistit: {e}");
            "off"
        }
    };
    AutostartStatus { state, problem }
}

fn set(entry: &dyn Entry, on: bool, problem: &str) -> Result<(), String> {
    if !on {
        return entry
            .remove()
            .map_err(|e| format!("Spouštění po přihlášení se nepodařilo vypnout: {e}"));
    }
    if !problem.is_empty() {
        return Err(problem.to_owned());
    }
    entry
        .add()
        .map_err(|e| format!("Spouštění po přihlášení se nepodařilo zapnout: {e}"))
}

// ---------------------------------------------------------------- this copy

/// The file the system should start: the AppImage itself (it runs from a
/// temporary mount), else this executable.
fn program() -> Result<PathBuf, String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(image) = std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(image);
    }
    let exe = std::env::current_exe().map_err(|e| format!("Cesta ke Skylimitu není známá: {e}"))?;
    #[cfg(target_os = "macos")]
    {
        // Gatekeeper's random copy, or the disk image it came on: both are gone later.
        let text = exe.to_string_lossy();
        if text.contains("/AppTranslocation/") || text.starts_with("/Volumes/") {
            return Err(
                "Skylimit běží z dočasného místa nebo z instalačního disku. Přesuň ho do složky Aplikace, spusť ho odtud a pak spouštění zapni."
                    .into(),
            );
        }
    }
    Ok(exe)
}

/// This copy's entry and why it cannot be added, if so.
fn platform_entry<R: Runtime>(app: &AppHandle<R>) -> (Box<dyn Entry>, String) {
    let program = program();
    let problem = program.as_ref().err().cloned().unwrap_or_default();
    let program = program.ok();

    #[cfg(windows)]
    {
        let entry = RunValue {
            run: RUN_KEY.into(),
            approved: APPROVED_KEY.into(),
            // The uninstaller removes exactly this name (${PRODUCTNAME}).
            name: app.package_info().name.clone(),
            command: program.as_deref().map(run_command),
        };
        (Box::new(entry), problem)
    }

    #[cfg(not(windows))]
    {
        let mut problem = problem;
        let identifier = app.config().identifier.clone();
        let place = if cfg!(target_os = "macos") {
            app.path().home_dir().map(|home| home.join("Library").join("LaunchAgents").join(format!("{identifier}.plist")))
        } else {
            // XDG_CONFIG_HOME or ~/.config.
            app.path().config_dir().map(|config| config.join("autostart").join(format!("{identifier}.desktop")))
        };
        let path = place.unwrap_or_else(|e| {
            if problem.is_empty() {
                problem = format!("Domovská složka není známá: {e}");
            }
            PathBuf::new()
        });
        let content = program.as_deref().map(|program| {
            if cfg!(target_os = "macos") {
                launch_agent(&identifier, program)
            } else {
                desktop_entry(&app.package_info().name, program)
            }
        });
        (Box::new(StartupFile { path, content }), problem)
    }
}

// ---------------------------------------------------------------- macOS, Linux

/// A Launch Agent or an XDG autostart entry: one file that only Skylimit writes.
#[cfg_attr(windows, allow(dead_code))]
struct StartupFile {
    path: PathBuf,
    /// The file as Skylimit writes it; `None` when this copy cannot be started at login.
    content: Option<String>,
}

impl Entry for StartupFile {
    fn find(&self) -> io::Result<Found> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Found::Nothing),
            Err(e) => return Err(e),
        };
        Ok(if turned_off(&text) {
            Found::TurnedOff
        } else if self.content.as_deref() == Some(text.as_str()) {
            Found::Ours
        } else {
            Found::Elsewhere
        })
    }

    fn add(&self) -> io::Result<()> {
        let content = self.content.as_deref().ok_or_else(|| io::Error::other("cesta ke Skylimitu není známá"))?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("skylimit-tmp");
        fs::write(&tmp, content)?;
        fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }

    fn remove(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// GNOME's and KDE's startup lists switch an entry off instead of deleting it.
#[cfg_attr(windows, allow(dead_code))]
fn turned_off(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .any(|line| line == "Hidden=true" || line == "X-GNOME-Autostart-enabled=false")
}

/// macOS starts it at login (`RunAtLoad`) in the user's graphical session
/// only. `AssociatedBundleIdentifiers` lets System Settings › Login Items
/// show it as Skylimit with its icon.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launch_agent(identifier: &str, program: &Path) -> String {
    let id = xml_text(identifier);
    let program = xml_text(&program.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{id}</string>
  <key>AssociatedBundleIdentifiers</key>
  <array>
    <string>{id}</string>
  </array>
  <key>ProgramArguments</key>
  <array>
    <string>{program}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
"#
    )
}

fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// XDG autostart entry. `TryExec` makes the desktop skip it once the file is gone.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn desktop_entry(name: &str, program: &Path) -> String {
    let path = program.to_string_lossy();
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=Limity Claude a Codexu\nExec={}\nTryExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exec_quoted(&path),
        path.replace('\\', "\\\\"),
    )
}

/// The Desktop Entry spec's quoting for `Exec`: double quotes with `"`,
/// `` ` ``, `$` and `\` escaped, then the string escape for backslashes and
/// `%%` for a literal percent sign (`%` starts a field code).
fn exec_quoted(path: &str) -> String {
    let mut quoted = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\").replace('%', "%%")
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// The command in the Run key: the program in quotes, whatever the path holds.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_command(program: &Path) -> String {
    let path = program.to_string_lossy();
    format!("\"{}\"", path.trim_start_matches(r"\\?\"))
}

/// Task Manager writes 02 (on) or 03 (off) as the first byte, then a time.
#[cfg_attr(not(windows), allow(dead_code))]
fn approved_off(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// A value under HKEY_CURRENT_USER's Run key (tests use a key of their own).
#[cfg(windows)]
struct RunValue {
    run: String,
    approved: String,
    name: String,
    /// `None` when this copy cannot be started at login.
    command: Option<String>,
}

#[cfg(windows)]
impl Entry for RunValue {
    fn find(&self) -> io::Result<Found> {
        let Some(value) = registry::get(&self.run, &self.name)? else {
            return Ok(Found::Nothing);
        };
        if registry::get(&self.approved, &self.name)?.is_some_and(|b| approved_off(&b)) {
            return Ok(Found::TurnedOff);
        }
        let value = registry::utf16_text(&value);
        Ok(if self.command.as_deref() == Some(value.as_str()) { Found::Ours } else { Found::Elsewhere })
    }

    fn add(&self) -> io::Result<()> {
        let command = self.command.as_deref().ok_or_else(|| io::Error::other("cesta ke Skylimitu není známá"))?;
        registry::set_text(&self.run, &self.name, command)?;
        // Turned off in Task Manager before: switching it on here wins.
        registry::delete(&self.approved, &self.name)
    }

    fn remove(&self) -> io::Result<()> {
        registry::delete(&self.run, &self.name)?;
        registry::delete(&self.approved, &self.name)
    }
}

#[cfg(windows)]
mod registry {
    use std::io;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_ANY,
    };

    pub fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn check(code: u32) -> io::Result<()> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        }
    }

    /// The raw value, or `None` when the key or the value does not exist.
    pub fn get(key: &str, name: &str) -> io::Result<Option<Vec<u8>>> {
        let (key, name) = (wide(key), wide(name));
        let mut size = 0u32;
        // SAFETY: both strings are NUL-terminated; a null buffer asks for the size.
        let code = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_ANY, null_mut(), null_mut(), &mut size)
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` holds `size` bytes, as the call is told.
        let code = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_ANY, null_mut(), buf.as_mut_ptr().cast(), &mut size)
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        buf.truncate(size as usize);
        Ok(Some(buf))
    }

    /// A REG_SZ value as text, without its terminating NULs.
    pub fn utf16_text(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned()
    }

    /// Writes a REG_SZ value, creating the key if needed.
    pub fn set_text(key: &str, name: &str, value: &str) -> io::Result<()> {
        let (key, name, data) = (wide(key), wide(name), wide(value));
        // SAFETY: NUL-terminated strings; `data` is `data.len() * 2` bytes long.
        check(unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
        })
    }

    /// Removes a value; one that is not there is fine.
    pub fn delete(key: &str, name: &str) -> io::Result<()> {
        let (key, name) = (wide(key), wide(name));
        // SAFETY: both strings are NUL-terminated.
        let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(code)
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub async fn autostart_status(app: AppHandle) -> Result<AutostartStatus, String> {
    let (entry, problem) = platform_entry(&app);
    Ok(status(entry.as_ref(), &problem))
}

#[tauri::command]
pub async fn autostart_set(app: AppHandle, on: bool) -> Result<AutostartStatus, String> {
    let (entry, problem) = platform_entry(&app);
    set(entry.as_ref(), on, &problem)?;
    Ok(status(entry.as_ref(), &problem))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_entry(dir: &Path, program: &str) -> StartupFile {
        StartupFile {
            path: dir.join("autostart").join("cz.myspulin.skylimit.desktop"),
            content: Some(desktop_entry("Skylimit", Path::new(program))),
        }
    }

    #[test]
    fn a_startup_file_is_added_found_and_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = file_entry(tmp.path(), "/opt/Skylimit.AppImage");
        assert_eq!(status(&entry, "").state, "off");
        assert!(set(&entry, false, "").is_ok(), "turning off what is not there is fine");

        set(&entry, true, "").unwrap();
        assert_eq!(status(&entry, "").state, "on");
        assert!(fs::read_to_string(&entry.path).unwrap().contains("Exec=\"/opt/Skylimit.AppImage\""));

        // The AppImage moved: the old entry still starts the old path.
        let moved = file_entry(tmp.path(), "/home/a/Apps/Skylimit.AppImage");
        assert_eq!(status(&moved, "").state, "moved");
        set(&moved, true, "").unwrap();
        assert_eq!(status(&moved, "").state, "on");

        set(&moved, false, "").unwrap();
        assert!(!entry.path.exists());
        assert_eq!(status(&moved, "").state, "off");
    }

    #[test]
    fn switched_off_in_the_desktop_list_counts_as_off_and_turning_on_rewrites_it() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = file_entry(tmp.path(), "/usr/bin/skylimit");
        set(&entry, true, "").unwrap();
        let off = fs::read_to_string(&entry.path).unwrap().replace("X-GNOME-Autostart-enabled=true", "X-GNOME-Autostart-enabled=false");
        fs::write(&entry.path, off).unwrap();
        assert_eq!(entry.find().unwrap(), Found::TurnedOff);
        assert_eq!(status(&entry, "").state, "off");
        set(&entry, true, "").unwrap();
        assert_eq!(status(&entry, "").state, "on");
        assert!(turned_off("[Desktop Entry]\nHidden=true\n"));
    }

    #[test]
    fn a_copy_that_cannot_start_at_login_says_why_but_can_still_turn_it_off() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = file_entry(tmp.path(), "/usr/bin/skylimit");
        set(&entry, true, "").unwrap();
        let translocated = StartupFile { path: entry.path.clone(), content: None };
        let why = "Skylimit běží z dočasného místa.";
        assert_eq!(set(&translocated, true, why).unwrap_err(), why);
        let s = status(&translocated, why);
        assert_eq!((s.state, s.problem.as_str()), ("moved", why));
        set(&translocated, false, why).unwrap();
        assert_eq!(status(&translocated, why).state, "off");
    }

    #[test]
    fn exec_paths_are_quoted_the_desktop_entry_way() {
        assert_eq!(exec_quoted("/usr/bin/skylimit"), "\"/usr/bin/skylimit\"");
        assert_eq!(exec_quoted("/home/a b/Skylimit.AppImage"), "\"/home/a b/Skylimit.AppImage\"");
        assert_eq!(exec_quoted("/opt/100%/x$y"), r#""/opt/100%%/x\\$y""#);
        assert_eq!(exec_quoted(r#"/a"b\c"#), r#""/a\\"b\\\\c""#);
        let entry = desktop_entry("Skylimit", Path::new("/home/a b/Skylimit.AppImage"));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=Skylimit\n"));
        assert!(entry.contains("\nTryExec=/home/a b/Skylimit.AppImage\n"));
    }

    #[test]
    fn the_launch_agent_starts_the_app_binary_at_login() {
        let plist = launch_agent("cz.myspulin.skylimit", Path::new("/Applications/Skylimit & co.app/Contents/MacOS/skylimit"));
        assert!(plist.contains("<key>Label</key>\n  <string>cz.myspulin.skylimit</string>"));
        assert!(plist.contains("<string>/Applications/Skylimit &amp; co.app/Contents/MacOS/skylimit</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<string>Aqua</string>"));
    }

    #[test]
    fn the_run_command_is_always_quoted() {
        assert_eq!(
            run_command(Path::new(r"C:\Users\Jan Novák\AppData\Local\Skylimit\skylimit.exe")),
            r#""C:\Users\Jan Novák\AppData\Local\Skylimit\skylimit.exe""#
        );
        assert_eq!(run_command(Path::new(r"\\?\C:\Skylimit\skylimit.exe")), r#""C:\Skylimit\skylimit.exe""#);
        assert!(approved_off(&[3, 0, 0, 0, 0x10, 0x20, 0, 0, 0, 0, 0, 0]));
        assert!(!approved_off(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    }

    /// Against the real registry, under a key of the test's own.
    #[cfg(windows)]
    #[test]
    fn the_run_value_follows_task_manager() {
        use windows_sys::Win32::System::Registry::{RegDeleteTreeW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_BINARY};

        let root = format!(r"Software\Skylimit-test-{}", std::process::id());
        let entry = |command: &str| RunValue {
            run: format!(r"{root}\Run"),
            approved: format!(r"{root}\StartupApproved\Run"),
            name: "Skylimit".into(),
            command: Some(command.into()),
        };
        let ours = entry(r#""C:\Users\Jan Novák\Skylimit\skylimit.exe""#);
        let result = std::panic::catch_unwind(|| {
            assert_eq!(status(&ours, "").state, "off");
            set(&ours, true, "").unwrap();
            assert_eq!(status(&ours, "").state, "on");
            assert_eq!(registry::utf16_text(&registry::get(&ours.run, "Skylimit").unwrap().unwrap()), r#""C:\Users\Jan Novák\Skylimit\skylimit.exe""#);
            assert_eq!(status(&entry(r#""D:\Skylimit\skylimit.exe""#), "").state, "moved");

            // Task Manager › Startup apps › Disable.
            let (key, name) = (registry::wide(&ours.approved), registry::wide("Skylimit"));
            let off = [3u8, 0, 0, 0, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x01];
            // SAFETY: NUL-terminated strings; `off` is 12 bytes.
            assert_eq!(unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_BINARY, off.as_ptr().cast(), 12) }, 0);
            assert_eq!(ours.find().unwrap(), Found::TurnedOff);
            set(&ours, true, "").unwrap();
            assert_eq!(status(&ours, "").state, "on", "turning on here clears Task Manager's switch");

            set(&ours, false, "").unwrap();
            set(&ours, false, "").unwrap();
            assert_eq!(status(&ours, "").state, "off");
            assert!(registry::get(&ours.run, "Skylimit").unwrap().is_none());
        });
        let root = registry::wide(&root);
        // SAFETY: NUL-terminated; removes only the test's own key.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, root.as_ptr()) };
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
