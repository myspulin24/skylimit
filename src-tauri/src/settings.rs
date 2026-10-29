//! Where Skylimit keeps its data and how the shell persists settings.
//!
//! Everything lives in `%LOCALAPPDATA%\skylimit-data` (macOS: Application
//! Support, Linux: `~/.local/share`). The folder is named differently from
//! the bundle identifier on purpose: the NSIS uninstaller's "delete app data"
//! option removes `%LOCALAPPDATA%\<identifier>` and must never take the usage
//! history with it (decision D4).
//!
//! Up to 0.4.0 the folder was `ai-usage-data`. The first start of a newer
//! version moves it over; nothing is copied or deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};
use usage_core::Settings;

const DATA_DIR_NAME: &str = "skylimit-data";
/// Name before the rename to Skylimit (0.1.0–0.4.0).
const LEGACY_DATA_DIR_NAME: &str = "ai-usage-data";

/// The data folder to use under `local`, moving the pre-0.5.0 one there
/// first. If the move fails (folder in use, permissions), the old folder
/// keeps working as it is: history is never left behind or split.
fn data_root(local: &Path) -> PathBuf {
    let root = local.join(DATA_DIR_NAME);
    let legacy = local.join(LEGACY_DATA_DIR_NAME);
    if root.exists() || !legacy.is_dir() {
        return root;
    }
    match fs::rename(&legacy, &root) {
        Ok(()) => root,
        Err(e) => {
            eprintln!("cannot move {} to {}: {e}; using the old folder", legacy.display(), root.display());
            legacy
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataPaths {
    pub root: PathBuf,
    /// Claude Code's folder: `CLAUDE_CONFIG_DIR`, usually `~/.claude`.
    pub claude_config: PathBuf,
    /// Usually `%USERPROFILE%\.claude\projects`.
    pub claude_projects: PathBuf,
    /// Claude Code's global config `.claude.json`: the signed-in account and
    /// the limits Claude Code caches (in the home folder, or in
    /// `CLAUDE_CONFIG_DIR` when set).
    pub claude_global: PathBuf,
    /// T3 Code's provider logs, one source of Claude's limits
    /// (`%USERPROFILE%\.t3\userdata\logs\provider`).
    pub t3_logs: PathBuf,
}

impl DataPaths {
    pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> Self {
        let local = env_dir("LOCALAPPDATA")
            .or_else(|| app.path().local_data_dir().ok())
            .unwrap_or_else(std::env::temp_dir);
        let home = env_dir("USERPROFILE")
            .or_else(|| app.path().home_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        Self::with(data_root(&local), &home)
    }

    /// The same folders without a running app: `skylimit statusline`, which
    /// Claude Code starts after every response. Mirrors Tauri's
    /// `local_data_dir` and never moves the pre-0.5.0 folder (the app has
    /// done that before anyone could connect Claude Code).
    pub fn from_env() -> Self {
        let home = env_dir("USERPROFILE")
            .or_else(|| env_dir("HOME"))
            .unwrap_or_else(|| PathBuf::from("."));
        let local = env_dir("LOCALAPPDATA").unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                home.join("Library").join("Application Support")
            } else {
                env_dir("XDG_DATA_HOME")
                    .filter(|d| d.is_absolute())
                    .unwrap_or_else(|| home.join(".local").join("share"))
            }
        });
        Self::with(local.join(DATA_DIR_NAME), &home)
    }

    fn with(root: PathBuf, home: &Path) -> Self {
        let config_dir = env_dir("CLAUDE_CONFIG_DIR");
        let claude_config = config_dir.clone().unwrap_or_else(|| home.join(".claude"));
        DataPaths {
            root,
            claude_projects: claude_config.join("projects"),
            claude_global: usage_core::claude_config::path(config_dir.as_deref(), home),
            claude_config,
            t3_logs: home.join(".t3").join("userdata").join("logs").join("provider"),
        }
    }

    /// Claude Code's limits from its status line; exists while connected.
    pub fn claude_code(&self) -> PathBuf {
        usage_core::claude_code::session_dir(&self.root)
    }

    pub fn db(&self) -> PathBuf {
        self.root.join("history.sqlite")
    }

    pub fn diag(&self) -> PathBuf {
        self.root.join("diag")
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn widget_placement(&self) -> PathBuf {
        self.root.join("widget-window.json")
    }

    pub fn app_placement(&self) -> PathBuf {
        self.root.join("app-window.json")
    }
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Settings from disk; anything missing or invalid falls back to the default.
pub fn load(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Lenient parse: every known key that holds a valid value is kept, so one
/// bad or missing field (older or newer file) never resets the others.
fn parse(text: &str) -> Settings {
    let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(text) else {
        return Settings::default();
    };
    let Ok(Value::Object(mut merged)) = serde_json::to_value(Settings::default()) else {
        return Settings::default();
    };
    for (key, value) in saved {
        if !merged.contains_key(&key) {
            continue;
        }
        let mut candidate = merged.clone();
        candidate.insert(key, value);
        if serde_json::from_value::<Settings>(Value::Object(candidate.clone())).is_ok() {
            merged = candidate;
        }
    }
    serde_json::from_value(Value::Object(merged)).unwrap_or_default()
}

/// Atomic write (temp file + rename) so a crash never leaves half a file.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use usage_core::{Currency, PlanCurrency, PlanPeriod, PlanSetting, Plans};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("skylimit-settings-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = temp_dir("roundtrip");
        let file = dir.join("settings.json");
        let settings = Settings {
            currency: Currency::Czk,
            click_through: true,
            plans: Plans {
                claude: None,
                codex: Some(PlanSetting {
                    amount: 540.0,
                    currency: PlanCurrency::Usd,
                    period: PlanPeriod::Year,
                }),
            },
            ..Settings::default()
        };
        save(&file, &settings).unwrap();
        assert_eq!(load(&file), settings);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        let dir = temp_dir("corrupt");
        assert_eq!(load(&dir.join("missing.json")), Settings::default());
        assert_eq!(parse("{ not json"), Settings::default());
        assert_eq!(parse("[1, 2]"), Settings::default());
    }

    #[test]
    fn older_files_without_transparency_load_with_an_opaque_widget() {
        let s = parse(r#"{"currency":"CZK","clickThrough":true}"#);
        assert_eq!(s.widget_transparency, 0);
        assert!(s.click_through);
        assert_eq!(parse(r#"{"widgetTransparency":40}"#).widget_transparency, 40);
        assert_eq!(parse(r#"{"widgetTransparency":-5}"#).widget_transparency, 0);
    }

    #[test]
    fn keeps_valid_fields_when_others_are_bad_or_missing() {
        let s = parse(r#"{"currency":"CZK","clickThrough":"yes","futureField":1}"#);
        assert_eq!(s.currency, Currency::Czk);
        assert!(!s.click_through);
        assert!(s.codex_estimate_in_score);
    }

    #[test]
    fn the_pre_skylimit_data_folder_is_moved_once() {
        let local = temp_dir("migrate");
        let legacy = local.join(LEGACY_DATA_DIR_NAME);
        fs::create_dir_all(legacy.join("diag")).unwrap();
        fs::write(legacy.join("history.sqlite"), b"history").unwrap();
        fs::write(legacy.join("settings.json"), br#"{"currency":"CZK"}"#).unwrap();

        let root = data_root(&local);
        assert_eq!(root, local.join(DATA_DIR_NAME));
        assert_eq!(fs::read(root.join("history.sqlite")).unwrap(), b"history");
        assert_eq!(load(&root.join("settings.json")).currency, Currency::Czk);
        assert!(root.join("diag").is_dir());
        assert!(!legacy.exists(), "moved, not copied");

        // Later starts (or an old folder that reappears) leave the new one alone.
        fs::create_dir_all(&legacy).unwrap();
        assert_eq!(data_root(&local), local.join(DATA_DIR_NAME));
        assert_eq!(fs::read(root.join("history.sqlite")).unwrap(), b"history");
        fs::remove_dir_all(&local).unwrap();
    }

    #[test]
    fn a_fresh_install_gets_the_new_folder_name() {
        let local = temp_dir("fresh");
        fs::create_dir_all(&local).unwrap();
        assert_eq!(data_root(&local), local.join("skylimit-data"));
        assert!(!local.join("skylimit-data").exists(), "created later by whoever writes");
        fs::remove_dir_all(&local).unwrap();
    }
}
