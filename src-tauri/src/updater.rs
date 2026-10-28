//! Automatic updates from GitHub Releases (`tauri-plugin-updater`).
//!
//! ```text
//!  20 s after start, then every 6 h (unless "Kontrolovat automaticky" is off
//!  or SKYLIMIT_AUTO_UPDATE=0):
//!    check latest.json ──▶ status "available" ──event "update"──▶ widget (dot), app (banner)
//!  "Aktualizovat a restartovat" (update_install):
//!    download (minisign check against the pubkey in tauri.conf.json)
//!      ──▶ stop the core ──▶ install ──▶ restart
//!         (Windows: the NSIS installer takes over and starts the new version)
//! ```
//!
//! The check lives in Rust, not in a webview: the app window exists only
//! while it is open, the widget is always there, and both read one status.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::shell::Shell;

const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(6 * 3600);
const STEP: Duration = Duration::from_secs(30);
const ENV_SWITCH: &str = "SKYLIMIT_AUTO_UPDATE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Nothing checked yet (or an automatic check failed quietly).
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    /// A manual check or the install failed; `error` says why.
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub phase: Phase,
    /// Version that is running.
    pub current: String,
    /// Version on offer, when there is one.
    pub version: Option<String>,
    /// Release notes from GitHub (Markdown from CHANGELOG.md).
    pub notes: Option<String>,
    /// Release date, epoch seconds.
    pub date: Option<i64>,
    /// Download progress 0..1 while downloading, when the size is known.
    pub progress: Option<f64>,
    pub error: Option<String>,
    /// Last finished check, epoch seconds.
    pub checked_at: Option<i64>,
    /// Automatic checks are on (setting and environment).
    pub auto: bool,
}

impl UpdateStatus {
    fn new(current: String) -> Self {
        UpdateStatus {
            phase: Phase::Idle,
            current,
            version: None,
            notes: None,
            date: None,
            progress: None,
            error: None,
            checked_at: None,
            auto: true,
        }
    }
}

/// Managed state: the status both windows see and the update on offer.
pub struct Updates {
    status: Mutex<UpdateStatus>,
    pending: Mutex<Option<Update>>,
}

impl Updates {
    pub fn new(current: String) -> Self {
        Updates {
            status: Mutex::new(UpdateStatus::new(current)),
            pending: Mutex::new(None),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The environment switch wins over the setting, so an admin or a test run
/// can keep the app off the network.
pub fn env_allows(value: Option<&str>) -> bool {
    !matches!(value.map(str::trim), Some("0" | "false" | "off" | "no"))
}

fn auto_enabled(app: &AppHandle) -> bool {
    let setting = app
        .try_state::<Shell>()
        .and_then(|s| s.core().ok().map(|c| c.settings().check_updates))
        .unwrap_or(true);
    setting && env_allows(std::env::var(ENV_SWITCH).ok().as_deref())
}

/// Store, stamp `auto` and tell both windows.
fn publish(app: &AppHandle, change: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
    let state = app.state::<Updates>();
    let snapshot = {
        let mut status = state.status.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut status);
        status.auto = auto_enabled(app);
        status.clone()
    };
    let _ = app.emit("update", &snapshot);
    snapshot
}

/// What a failed check tells the user. Details go to stderr.
pub fn friendly_error(detail: &str) -> String {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("signature") {
        "Aktualizace nemá platný podpis, proto se neinstaluje.".into()
    } else if lower.contains("404") || lower.contains("not found") {
        "Na GitHubu zatím není vydání s aktualizací.".into()
    } else {
        "Aktualizaci se nepodařilo zkontrolovat. Jsi připojený k internetu?".into()
    }
}

/// Ask GitHub. A manual check reports errors; an automatic one stays quiet
/// (no banner for a missing network), but keeps an offer already found.
pub async fn check(app: &AppHandle, manual: bool) -> UpdateStatus {
    {
        let state = app.state::<Updates>();
        let busy = matches!(
            state.status.lock().map(|s| s.phase).unwrap_or(Phase::Idle),
            Phase::Checking | Phase::Downloading | Phase::Installing
        );
        if busy {
            return publish(app, |_| {});
        }
    }
    publish(app, |s| {
        s.phase = Phase::Checking;
        s.error = None;
    });
    let result = match app.updater() {
        Ok(updater) => updater.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match result {
        Ok(Some(update)) => {
            let (version, notes, date) = (update.version.clone(), update.body.clone(), update.date.map(|d| d.unix_timestamp()));
            if let Ok(mut pending) = app.state::<Updates>().pending.lock() {
                *pending = Some(update);
            }
            publish(app, |s| {
                s.phase = Phase::Available;
                s.version = Some(version);
                s.notes = notes.filter(|n| !n.trim().is_empty());
                s.date = date;
                s.checked_at = Some(now());
            })
        }
        Ok(None) => {
            if let Ok(mut pending) = app.state::<Updates>().pending.lock() {
                *pending = None;
            }
            publish(app, |s| {
                s.phase = Phase::UpToDate;
                s.version = None;
                s.notes = None;
                s.date = None;
                s.checked_at = Some(now());
            })
        }
        Err(detail) => {
            eprintln!("update check failed: {detail}");
            let had_offer = app.state::<Updates>().pending.lock().map(|p| p.is_some()).unwrap_or(false);
            publish(app, |s| {
                s.checked_at = Some(now());
                if had_offer {
                    s.phase = Phase::Available;
                } else if manual {
                    s.phase = Phase::Error;
                    s.error = Some(friendly_error(&detail));
                } else {
                    s.phase = Phase::Idle;
                }
            })
        }
    }
}

/// Background checks for the whole run of the app.
pub fn spawn_auto(app: AppHandle) {
    let _ = std::thread::Builder::new().name("skylimit-updater".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if auto_enabled(&app) {
                tauri::async_runtime::block_on(check(&app, false));
            } else {
                publish(&app, |_| {});
            }
            let mut waited = Duration::ZERO;
            while waited < EVERY {
                std::thread::sleep(STEP);
                waited += STEP;
            }
        }
    });
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    publish(&app, |_| {})
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> UpdateStatus {
    check(&app, true).await
}

/// Download, verify, install, restart. Returns only on failure: on success
/// the process ends (Windows: in the installer; elsewhere: by restarting).
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    let update = app
        .state::<Updates>()
        .pending
        .lock()
        .map_err(|_| "Aktualizace teď není k dispozici.".to_string())?
        .clone()
        .ok_or_else(|| "Aktualizace teď není k dispozici. Zkus ji nejdřív zkontrolovat.".to_string())?;

    publish(&app, |s| {
        s.phase = Phase::Downloading;
        s.progress = Some(0.0);
        s.error = None;
    });
    let mut downloaded = 0u64;
    let mut last_step = -1i64;
    let progress_app = app.clone();
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let fraction = (downloaded as f64 / total as f64).min(1.0);
                    // An event per 2 % is plenty for a progress bar.
                    let step = (fraction * 50.0) as i64;
                    if step != last_step {
                        last_step = step;
                        publish(&progress_app, |s| s.progress = Some(fraction));
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| {
            eprintln!("update download failed: {e}");
            let message = friendly_error(&e.to_string());
            publish(&app, |s| {
                s.phase = Phase::Error;
                s.progress = None;
                s.error = Some(if message.contains("podpis") {
                    message.clone()
                } else {
                    "Aktualizaci se nepodařilo stáhnout. Zkus to prosím znovu.".into()
                });
            });
            message
        })?;

    publish(&app, |s| {
        s.phase = Phase::Installing;
        s.progress = Some(1.0);
    });
    // Close the history database and stop Codex before files get replaced.
    if let Some(shell) = app.try_state::<Shell>() {
        shell.shutdown();
    }
    if let Err(e) = update.install(bytes) {
        eprintln!("update install failed: {e}");
        let message = "Aktualizaci se nepodařilo nainstalovat. Spusť prosím Skylimit znovu.".to_string();
        publish(&app, |s| {
            s.phase = Phase::Error;
            s.error = Some(message.clone());
        });
        return Err(message);
    }
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_can_switch_automatic_checks_off() {
        assert!(env_allows(None));
        assert!(env_allows(Some("1")));
        assert!(env_allows(Some("")));
        for off in ["0", "false", "off", "no", " 0 "] {
            assert!(!env_allows(Some(off)), "{off}");
        }
    }

    #[test]
    fn errors_are_told_in_plain_words() {
        assert!(friendly_error("signature verification failed").contains("podpis"));
        assert!(friendly_error("Could not fetch a valid release JSON from the remote: 404 Not Found").contains("zatím není vydání"));
        assert!(friendly_error("error sending request: dns error").contains("připojený k internetu"));
    }

    #[test]
    fn status_serializes_for_the_ui() {
        let mut s = UpdateStatus::new("0.6.0".into());
        s.phase = Phase::UpToDate;
        s.checked_at = Some(1);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["phase"], "upToDate");
        assert_eq!(json["current"], "0.6.0");
        assert_eq!(json["checkedAt"], 1);
        assert!(json["version"].is_null());
        assert_eq!(json["auto"], true);
    }
}
