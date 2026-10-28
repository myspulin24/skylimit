//! Where Skylimit keeps its data and how the shell persists settings.
//!
//! Everything lives in `%LOCALAPPDATA%\ai-usage-data`. The folder is named
//! differently from the bundle identifier on purpose: the NSIS uninstaller's
//! "delete app data" option removes `%LOCALAPPDATA%\<identifier>` and must
//! never take the usage history with it (decision D4).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};
use usage_core::Settings;

const DATA_DIR_NAME: &str = "ai-usage-data";

#[derive(Debug, Clone)]
pub struct DataPaths {
    pub root: PathBuf,
    /// Usually `%USERPROFILE%\.claude\projects`.
    pub claude_projects: PathBuf,
    /// T3 Code's provider logs, the source of Claude's limits
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
        DataPaths {
            root: local.join(DATA_DIR_NAME),
            claude_projects: home.join(".claude").join("projects"),
            t3_logs: home.join(".t3").join("userdata").join("logs").join("provider"),
        }
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
        let dir = std::env::temp_dir().join(format!("ai-usage-settings-{tag}-{}", std::process::id()));
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
}
