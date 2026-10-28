//! usage-core: the runtime behind Skylimit.
//!
//! One core thread owns the SQLite connection (WAL) and computes score, moods,
//! tray color and the [`Snapshot`]. Providers run on their own threads, keep
//! their own schedules and send updates over a channel:
//!
//! ```text
//!  <projects>/**/*.jsonl ──notify + 30 s poll──▶ claude.rs ─┐
//!  codex app-server (Job Object) ──JSON-RPC────▶ codex.rs  ─┤ Update
//!  T3 Code provider logs ──────10 s poll───────▶ t3.rs     ─┤
//!  ČNB daily fixing (≤ 1×/day) ────────────────▶ fx.rs     ─┤
//!                                                           ▼
//!             core thread: store.rs ─▶ pricing.rs ─▶ score.rs ─▶ mood.rs
//!                                                           │ watch
//!                                                        Snapshot (no secrets)
//! ```
//!
//! Claude limits never touch Claude login data and never call Anthropic
//! (terms gate): they come from the `rate_limit_event` messages the official
//! Claude Agent SDK hands to T3 Code, which logs them locally (`t3.rs`).
//! Without T3 Code they are Off and Claude's mood comes from score and
//! activity only.

mod claude;
mod codex;
mod diag;
mod fx;
pub mod model;
mod mood;
mod pricing;
mod score;
mod store;
mod t3;

pub use model::*;
/// Re-exported so the shell does not need its own `tokio` dependency.
pub use tokio::sync::watch;

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Days, NaiveDate};

use crate::claude::{ClaudeIn, ClaudeThread};
use crate::codex::{CodexCtl, CodexEvent, CodexThread, CodexView, LimitsReading, RateWindow, Status, UsageSummary};
use crate::diag::Diag;
use crate::fx::FxDay;
use crate::mood::{Activity, Level, MoodInput};
use crate::pricing::PriceTable;
use crate::store::Store;
use crate::t3::{ClaudeLimitsView, ClaudeReading, ClaudeWindow, T3Thread};

/// Seconds since the Unix epoch. Injectable so tests control time.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    })
}

/// Note on Claude's limits row when no T3 Code log folder is configured or found.
pub const CLAUDE_LIMITS_NOTE: &str = t3::NOTE_NO_T3;

const TICK: Duration = Duration::from_millis(250);
/// Recompute at least this often (seconds of the injected clock) so time-based
/// states (Stale, sleep, month change) move without new data.
const PERIODIC_SECS: i64 = 30;
const CELEBRATE_SECS: i64 = 4;
const HEALTH_WINDOW: i64 = 3600;
const LIMIT_HISTORY_DAYS: i64 = 90;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexSource {
    /// Spawn this program speaking the app-server protocol on stdio.
    Spawn { program: String, args: Vec<String> },
    Disabled,
}

impl CodexSource {
    /// The official CLI: `cmd.exe /C codex app-server` on Windows (the npm
    /// shim is a `.cmd`). On macOS and Linux through the user's login shell:
    /// an app started from Finder or a desktop menu does not get the PATH the
    /// terminal has (Homebrew, npm global, nvm), so a plain `codex` would not
    /// be found. `exec` makes the shell become codex, so stopping the child
    /// stops codex itself.
    pub fn official() -> Self {
        if cfg!(windows) {
            CodexSource::Spawn {
                program: "cmd.exe".into(),
                args: vec!["/C".into(), "codex".into(), "app-server".into()],
            }
        } else {
            let fallback = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/sh" };
            let shell = std::env::var("SHELL")
                .ok()
                .filter(|s| s.starts_with('/'))
                .unwrap_or_else(|| fallback.into());
            CodexSource::Spawn {
                program: shell,
                args: vec!["-lc".into(), "exec codex app-server".into()],
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxSource {
    /// ČNB daily fixing, at most one successful fetch per day, cached in SQLite.
    Cnb,
    /// No network: cached rate from SQLite, else the documented fallback.
    Offline,
}

#[derive(Clone)]
pub struct Config {
    /// Usually `%USERPROFILE%\.claude\projects`.
    pub claude_projects_dir: PathBuf,
    /// Usually `%LOCALAPPDATA%\skylimit-data\history.sqlite` (parents are created).
    pub db_path: PathBuf,
    /// T3 Code's provider logs (`%USERPROFILE%\.t3\userdata\logs\provider`),
    /// the only source of Claude's limits. `None` = Claude limits Off.
    pub t3_logs_dir: Option<PathBuf>,
    pub diag_dir: PathBuf,
    pub codex: CodexSource,
    /// Initial settings; persisting them is the shell's job.
    pub settings: Settings,
    pub clock: Clock,
    pub fx: FxSource,
    /// Weekly refresh of the LiteLLM price list. OFF by default in the MVP.
    pub pricing_refresh: bool,
}

impl Config {
    pub fn new(claude_projects_dir: PathBuf, db_path: PathBuf, diag_dir: PathBuf) -> Self {
        Config {
            claude_projects_dir,
            db_path,
            t3_logs_dir: None,
            diag_dir,
            codex: CodexSource::official(),
            settings: Settings::default(),
            clock: system_clock(),
            fx: FxSource::Cnb,
            pricing_refresh: false,
        }
    }
}

pub(crate) enum Update {
    ClaudeBatch(claude::FileBatch),
    ClaudeProgress {
        epoch: u64,
        files_done: u32,
        files_total: u32,
        done: bool,
        dir_exists: bool,
    },
    Codex(CodexEvent),
    ClaudeLimits {
        dir_exists: bool,
        reading: Option<ClaudeReading>,
    },
    Fx {
        day: FxDay,
        fetched_on: String,
    },
    Prices(PriceTable),
}

pub(crate) enum CoreMsg {
    Update(Update),
    SettingsChanged,
    Refresh,
    ClearHistory(Sender<()>),
    Shutdown,
}

/// Small blocking GET used for the ČNB fixing and the optional price refresh.
pub(crate) fn http_get_text(url: &str) -> Result<String, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("skylimit/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.text().map_err(|e| e.to_string())
}

pub struct Core;

impl Core {
    /// Start the core and its providers. Never fails: a database that cannot
    /// be opened falls back to memory (and says so in the diagnostic log).
    pub fn start(config: Config) -> CoreHandle {
        let clock = config.clock.clone();
        let now = clock();
        let diag = Diag::new(&config.diag_dir);
        let store = match Store::open(&config.db_path) {
            Ok(s) => s,
            Err(e) => {
                diag.log(now, "store", &format!("history database unavailable, using memory: {e}"));
                Store::open_in_memory()
            }
        };
        let settings = Arc::new(Mutex::new(config.settings.clone()));
        let mut first = Snapshot::empty(now, config.settings.currency);
        first.widget_transparency = config.settings.widget_transparency.min(WIDGET_TRANSPARENCY_MAX);
        let (snap_tx, snap_rx) = watch::channel(first);
        let (tx, rx) = mpsc::channel::<CoreMsg>();
        let stop = Arc::new(AtomicBool::new(false));
        let mut children = Vec::new();

        let (claude_tx, claude_rx) = mpsc::channel();
        children.push(
            ClaudeThread {
                dir: config.claude_projects_dir.clone(),
                files: store.load_files(),
                core: tx.clone(),
                me: claude_tx.clone(),
                rx: claude_rx,
                stop: stop.clone(),
                clock: clock.clone(),
                diag: diag.clone(),
            }
            .spawn(),
        );

        let (t3_wake, t3_wake_rx) = mpsc::channel();
        if let Some(dir) = &config.t3_logs_dir {
            children.push(
                T3Thread {
                    dir: dir.clone(),
                    core: tx.clone(),
                    wake: t3_wake_rx,
                    stop: stop.clone(),
                    clock: clock.clone(),
                    diag: diag.clone(),
                }
                .spawn(),
            );
        }

        let codex_disabled = config.codex == CodexSource::Disabled;
        let codex_ctl = match &config.codex {
            CodexSource::Spawn { program, args } => {
                let (ctl_tx, ctl_rx) = mpsc::channel();
                children.push(
                    CodexThread {
                        program: program.clone(),
                        args: args.clone(),
                        core: tx.clone(),
                        ctl: ctl_rx,
                        stop: stop.clone(),
                        clock: clock.clone(),
                        diag: diag.clone(),
                    }
                    .spawn(),
                );
                Some(ctl_tx)
            }
            CodexSource::Disabled => None,
        };

        let fx_stop = match config.fx {
            FxSource::Cnb => {
                let (stop_tx, stop_rx) = mpsc::channel();
                // Not joined on shutdown: a request in flight may take its full timeout.
                let _ = fx::spawn_fetcher(tx.clone(), stop_rx, store.meta_get("fx_fetched_on"), clock.clone(), diag.clone());
                Some(stop_tx)
            }
            FxSource::Offline => None,
        };
        let prices_stop = if config.pricing_refresh {
            let (stop_tx, stop_rx) = mpsc::channel();
            spawn_price_refresh(tx.clone(), stop_rx, clock.clone(), diag.clone());
            Some(stop_tx)
        } else {
            None
        };

        let mut codex = CodexView::new(codex_disabled);
        restore_codex(&store, &mut codex, now);
        let t3_dir = config.t3_logs_dir.as_deref();
        let claude_limits = ClaudeLimitsView {
            configured: t3_dir.is_some(),
            dir_exists: t3_dir.is_some_and(|d| d.is_dir()),
            reading: t3_dir.and_then(|_| restore_claude_limits(&store, now)),
        };
        let state = CoreState {
            fx: store.latest_fx().unwrap_or_else(FxDay::fallback),
            claude_last_ts: store.meta_get("claude_last_ts").and_then(|v| v.parse().ok()),
            claude_dir_exists: config.claude_projects_dir.is_dir(),
            clock,
            diag,
            store,
            prices: PriceTable::embedded(),
            settings: settings.clone(),
            claude_tx,
            codex_ctl,
            t3_wake,
            fx_stop,
            prices_stop,
            children,
            claude_epoch: 0,
            backfill: Backfill {
                done: false,
                files_done: 0,
                files_total: 0,
            },
            health: Vec::new(),
            codex,
            claude_limits,
            claude_level: Level::Normal,
            codex_level: Level::Normal,
            tray_level: Level::Normal,
            milestones_primed: false,
            queue: VecDeque::new(),
            current: None,
            started_at: now,
            last_change_at: now,
            dirty: true,
            next_recompute_at: now,
            last_prune_day: None,
            logged_unpriced: HashSet::new(),
            snap_tx,
        };
        let thread = std::thread::Builder::new()
            .name("usage-core".into())
            .spawn(move || state.run(rx))
            .expect("spawn core thread");

        CoreHandle {
            tx,
            snapshot: snap_rx,
            settings,
            stop,
            thread: Some(thread),
        }
    }
}

pub struct CoreHandle {
    tx: Sender<CoreMsg>,
    snapshot: watch::Receiver<Snapshot>,
    settings: Arc<Mutex<Settings>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CoreHandle {
    /// Receiver that sees every new snapshot.
    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.snapshot.clone()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.borrow().clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_settings(&self, settings: Settings) {
        if let Ok(mut s) = self.settings.lock() {
            *s = settings;
        }
        let _ = self.tx.send(CoreMsg::SettingsChanged);
    }

    /// Read every source now ("Obnovit" in the widget) instead of waiting
    /// for its schedule. Returns at once; new data arrives as snapshots.
    pub fn refresh(&self) {
        let _ = self.tx.send(CoreMsg::Refresh);
    }

    /// Wipe the history database ("Smazat historii") and read the transcripts
    /// that still exist again. Returns once the tables are empty.
    pub fn clear_history(&self) {
        let (ack_tx, ack_rx) = mpsc::channel();
        if self.tx.send(CoreMsg::ClearHistory(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(Duration::from_secs(10));
        }
    }

    pub fn shutdown(mut self) {
        self.stop_now();
    }

    fn stop_now(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.stop.store(true, Ordering::Relaxed);
            let _ = self.tx.send(CoreMsg::Shutdown);
            let _ = thread.join();
        }
    }
}

impl Drop for CoreHandle {
    fn drop(&mut self) {
        self.stop_now();
    }
}

fn spawn_price_refresh(tx: Sender<CoreMsg>, stop: Receiver<()>, clock: Clock, diag: Diag) {
    let _ = std::thread::Builder::new()
        .name("usage-prices".into())
        .spawn(move || {
            let mut last: Option<i64> = None;
            loop {
                let now = clock();
                if last.is_none_or(|t| now - t >= 7 * 86_400) {
                    last = Some(now);
                    match pricing::fetch_litellm() {
                        Ok(table) => {
                            if tx.send(CoreMsg::Update(Update::Prices(table))).is_err() {
                                return;
                            }
                        }
                        Err(e) => diag.log(now, "pricing", &format!("price refresh failed: {e}")),
                    }
                }
                match stop.recv_timeout(Duration::from_secs(5)) {
                    Err(RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
            }
        });
}

/// Last known Codex limits and streaks from the database, so a restart shows
/// Stale values instead of nothing.
fn restore_codex(store: &Store, codex: &mut CodexView, now: i64) {
    let rows = store.latest_limit_snapshot("codex");
    if let Some(ts) = rows.first().map(|r| r.ts) {
        if now - ts < LIMIT_HISTORY_DAYS * 86_400 {
            let window = |name: &str| {
                rows.iter().find(|r| r.win == name).and_then(|r| {
                    r.percent.map(|p| RateWindow {
                        used_percent: p,
                        window_mins: None,
                        resets_at: r.resets_at,
                    })
                })
            };
            codex.limits = Some(LimitsReading {
                at: ts,
                primary: window("5h"),
                secondary: window("week"),
                extra: Vec::new(),
            });
        }
    }
    if let Some(v) = store
        .meta_get("codex_summary")
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        codex.summary = Some(UsageSummary {
            lifetime_tokens: v["lifetime"].as_u64(),
            peak_daily_tokens: v["peak"].as_u64(),
            current_streak_days: v["current"].as_u64().map(|x| x as u32),
            longest_streak_days: v["longest"].as_u64().map(|x| x as u32),
        });
    }
}

/// Last Claude limits reading from the database (T3 Code logs rotate away).
fn restore_claude_limits(store: &Store, now: i64) -> Option<ClaudeReading> {
    let rows = store.latest_limit_snapshot("claude");
    let at = rows.first().map(|r| r.ts).filter(|ts| now - ts < LIMIT_HISTORY_DAYS * 86_400)?;
    let window = |name: &str| {
        rows.iter().find(|r| r.win == name).and_then(|r| {
            r.percent.map(|percent| ClaudeWindow {
                percent,
                resets_at: r.resets_at,
            })
        })
    };
    let reading = ClaudeReading {
        at,
        five_hour: window("5h"),
        seven_day: window("week"),
    };
    (reading.five_hour.is_some() || reading.seven_day.is_some()).then_some(reading)
}

struct CoreState {
    clock: Clock,
    diag: Diag,
    store: Store,
    prices: PriceTable,
    fx: FxDay,
    settings: Arc<Mutex<Settings>>,
    claude_tx: Sender<ClaudeIn>,
    codex_ctl: Option<Sender<CodexCtl>>,
    t3_wake: Sender<()>,
    fx_stop: Option<Sender<()>>,
    prices_stop: Option<Sender<()>>,
    children: Vec<JoinHandle<()>>,

    claude_epoch: u64,
    backfill: Backfill,
    claude_dir_exists: bool,
    claude_last_ts: Option<i64>,
    /// (line time, has usage) of assistant lines from the last hour.
    health: Vec<(i64, bool)>,

    codex: CodexView,
    claude_limits: ClaudeLimitsView,
    claude_level: Level,
    codex_level: Level,
    tray_level: Level,

    milestones_primed: bool,
    queue: VecDeque<Celebration>,
    current: Option<(Celebration, i64)>,

    started_at: i64,
    last_change_at: i64,
    dirty: bool,
    next_recompute_at: i64,
    last_prune_day: Option<String>,
    logged_unpriced: HashSet<String>,
    snap_tx: watch::Sender<Snapshot>,
}

impl CoreState {
    fn run(mut self, rx: Receiver<CoreMsg>) {
        self.publish();
        'main: loop {
            match rx.recv_timeout(TICK) {
                Ok(msg) => {
                    if !self.handle(msg) {
                        break;
                    }
                    // Drain what is queued, but publish between large bursts so
                    // the backfill progress keeps moving.
                    for _ in 0..64 {
                        let Ok(msg) = rx.try_recv() else { break };
                        if !self.handle(msg) {
                            break 'main;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            let now = (self.clock)();
            if self.dirty || now >= self.next_recompute_at {
                self.publish();
            }
        }
        self.stop_children();
    }

    fn stop_children(&mut self) {
        let _ = self.claude_tx.send(ClaudeIn::Shutdown);
        self.codex_ctl.take();
        self.fx_stop.take();
        self.prices_stop.take();
        for child in self.children.drain(..) {
            let _ = child.join();
        }
    }

    /// Returns false on shutdown.
    fn handle(&mut self, msg: CoreMsg) -> bool {
        let now = (self.clock)();
        match msg {
            CoreMsg::Shutdown => return false,
            CoreMsg::SettingsChanged => {
                // A new plan or currency is not live usage: whatever it
                // reaches is recorded silently, like a repricing.
                self.milestones_primed = false;
                self.dirty = true;
            }
            CoreMsg::Refresh => {
                let _ = self.claude_tx.send(ClaudeIn::PollNow);
                let _ = self.t3_wake.send(());
                if let Some(ctl) = &self.codex_ctl {
                    let _ = ctl.send(CodexCtl::RefreshNow);
                }
                self.dirty = true;
            }
            CoreMsg::ClearHistory(ack) => {
                self.clear_history(now);
                self.publish();
                let _ = ack.send(());
            }
            CoreMsg::Update(u) => self.apply(u, now),
        }
        true
    }

    fn apply(&mut self, update: Update, now: i64) {
        match update {
            Update::ClaudeBatch(b) => {
                if b.epoch != self.claude_epoch {
                    return;
                }
                match self.store.apply_claude_batch(&b.path, &b.state, &b.records) {
                    Ok(changed) if changed > 0 => {
                        self.last_change_at = now;
                        self.dirty = true;
                    }
                    Ok(_) => {}
                    Err(e) => self.diag.log(now, "store", &format!("claude batch not stored: {e}")),
                }
                let newest = b.records.iter().map(|r| r.ts).filter(|ts| *ts <= now + 300).max();
                if let Some(ts) = newest {
                    if self.claude_last_ts.is_none_or(|t| ts > t) {
                        self.claude_last_ts = Some(ts);
                        let _ = self.store.meta_set("claude_last_ts", &ts.to_string());
                        self.dirty = true;
                    }
                }
                if !b.health.is_empty() {
                    self.health.extend(b.health);
                    self.dirty = true;
                }
            }
            Update::ClaudeProgress {
                epoch,
                files_done,
                files_total,
                done,
                dir_exists,
            } => {
                if epoch != self.claude_epoch {
                    return;
                }
                self.backfill = Backfill {
                    done,
                    files_done,
                    files_total,
                };
                self.claude_dir_exists = dir_exists;
                self.dirty = true;
            }
            Update::Codex(ev) => {
                match &ev {
                    CodexEvent::Limits(r) => {
                        for (win, w) in [("5h", &r.primary), ("week", &r.secondary)] {
                            let _ = self.store.add_limit_snapshot(
                                r.at,
                                "codex",
                                win,
                                w.as_ref().map(|w| w.used_percent),
                                w.as_ref().and_then(|w| w.resets_at),
                            );
                        }
                        self.last_change_at = now;
                    }
                    CodexEvent::Usage(u) => {
                        if let Err(e) = self.store.upsert_codex_buckets(&u.buckets, now) {
                            self.diag.log(now, "store", &format!("codex buckets not stored: {e}"));
                        }
                        let summary = serde_json::json!({
                            "lifetime": u.summary.lifetime_tokens,
                            "peak": u.summary.peak_daily_tokens,
                            "current": u.summary.current_streak_days,
                            "longest": u.summary.longest_streak_days,
                        });
                        let _ = self.store.meta_set("codex_summary", &summary.to_string());
                        self.last_change_at = now;
                    }
                    _ => {}
                }
                self.codex.apply(&ev);
                self.dirty = true;
            }
            Update::ClaudeLimits { dir_exists, reading } => {
                self.claude_limits.dir_exists = dir_exists;
                if let Some(r) = reading {
                    if self.claude_limits.reading.as_ref().is_none_or(|old| r.at > old.at) {
                        for (win, w) in [("5h", &r.five_hour), ("week", &r.seven_day)] {
                            let _ = self.store.add_limit_snapshot(
                                r.at,
                                "claude",
                                win,
                                w.as_ref().map(|w| w.percent),
                                w.as_ref().and_then(|w| w.resets_at),
                            );
                        }
                        self.claude_limits.reading = Some(r);
                        self.last_change_at = now;
                    }
                }
                self.dirty = true;
            }
            Update::Fx { day, fetched_on } => {
                let _ = self.store.put_fx(&day, now);
                let _ = self.store.meta_set("fx_fetched_on", &fetched_on);
                self.fx = day;
                self.dirty = true;
            }
            Update::Prices(table) => {
                if self.prices.plausible_successor(&table) {
                    self.prices = table;
                    // Repricing never fires a celebration.
                    self.milestones_primed = false;
                    self.dirty = true;
                } else {
                    self.diag.log(now, "pricing", "downloaded price list rejected by sanity check");
                }
            }
        }
    }

    fn clear_history(&mut self, now: i64) {
        if let Err(e) = self.store.clear_all() {
            self.diag.log(now, "store", &format!("clear history failed: {e}"));
        }
        self.claude_epoch += 1;
        let _ = self.claude_tx.send(ClaudeIn::Reset(self.claude_epoch));
        if let Some(ctl) = &self.codex_ctl {
            let _ = ctl.send(CodexCtl::ReadUsageNow);
        }
        self.backfill = Backfill {
            done: false,
            files_done: 0,
            files_total: 0,
        };
        self.health.clear();
        self.claude_last_ts = None;
        self.codex.usage_seen = false;
        self.codex.summary = None;
        self.queue.clear();
        self.current = None;
        self.milestones_primed = false;
        self.last_change_at = now;
        self.dirty = true;
    }

    fn publish(&mut self) {
        let now = (self.clock)();
        let snap = self.compute(now);
        self.snap_tx.send_replace(snap);
        self.dirty = false;
        let mut next = now + PERIODIC_SECS;
        if let Some((_, until)) = &self.current {
            next = next.min(*until);
        }
        self.next_recompute_at = next;

        let today = score::prague_date_string(now);
        if self.last_prune_day.as_deref() != Some(today.as_str()) {
            let _ = self.store.prune_limit_snapshots(now - LIMIT_HISTORY_DAYS * 86_400);
            self.last_prune_day = Some(today);
        }
    }

    /// Format health check (D5): ≥ 20 assistant lines in the last hour and
    /// more than 20 % of them without a recognizable `usage`.
    fn format_changed(&mut self, now: i64) -> bool {
        self.health.retain(|(t, _)| *t >= now - HEALTH_WINDOW && *t <= now + 300);
        let total = self.health.len();
        let bad = self.health.iter().filter(|(_, ok)| !ok).count();
        total >= 20 && bad * 5 > total
    }

    fn claude_activity(&self, now: i64) -> Activity {
        if !self.claude_dir_exists {
            return Activity::NoSignal;
        }
        let since = self.claude_last_ts.unwrap_or(self.started_at);
        let quiet = now - since;
        if self.claude_last_ts.is_some() && quiet < 300 {
            Activity::Active
        } else if quiet >= 1800 {
            Activity::Asleep
        } else {
            Activity::Idle
        }
    }

    fn compute(&mut self, now: i64) -> Snapshot {
        let settings = self.settings.lock().map(|s| s.clone()).unwrap_or_default();
        let mb = score::month_bounds(now);
        let rows = self.store.usage_rows(mb.start, mb.end).unwrap_or_default();
        let web = self.store.web_rows(mb.start, mb.end).unwrap_or_default();
        let all_buckets = self.store.codex_buckets().unwrap_or_default();
        let month_buckets: Vec<(String, u64)> = all_buckets
            .iter()
            .filter(|(d, _)| d.starts_with(&mb.month))
            .cloned()
            .collect();

        let format_changed = self.format_changed(now);
        let codex_rate = self.prices.codex_blended_usd_per_token();
        let codex_state = if self.codex.disabled {
            ScoreState::Off
        } else if codex_rate.is_none() || (!self.codex.usage_seen && all_buckets.is_empty()) {
            ScoreState::Unknown
        } else {
            ScoreState::Estimate
        };
        let include_codex = codex_state == ScoreState::Estimate && settings.codex_estimate_in_score;
        let cost = score::month_cost(
            &rows,
            &web,
            &month_buckets,
            &self.prices,
            codex_rate,
            &settings.excluded_entrypoints,
            include_codex,
        );
        for m in &cost.unpriced_models {
            if self.logged_unpriced.insert(m.clone()) {
                self.diag.log(now, "pricing", &format!("no price for model {m}; its tokens stay unpriced"));
            }
        }
        let claude_state = if format_changed {
            ScoreState::FormatChanged
        } else if !self.claude_dir_exists && rows.is_empty() {
            ScoreState::Unknown
        } else if cost.claude.unpriced > 0 {
            ScoreState::Partial
        } else {
            ScoreState::Ok
        };
        let (scores, total) = score::scores_and_total(&cost, claude_state, codex_state, &settings, &self.fx);
        let conv = self.fx.usd_to(settings.currency);

        let limits = vec![
            self.claude_limits.provider_limits(now, self.claude_last_ts),
            self.codex.provider_limits(now),
        ];
        self.claude_level = mood::next_level(self.claude_level, limits[0].governing);
        self.codex_level = mood::next_level(self.codex_level, limits[1].governing);
        self.tray_level = mood::next_level(self.tray_level, mood::tray_value(&limits));
        let tray = mood::tray(&limits, self.tray_level);

        // History-wide daily totals for streaks and the daily record.
        let claude_days = score::daily_tokens(&self.store.hourly_totals("claude").unwrap_or_default());
        let codex_days: BTreeMap<NaiveDate, u64> = all_buckets
            .iter()
            .filter_map(|(d, t)| Some((NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?, *t)))
            .collect();
        let (c_cur, c_long, c_start) = score::streaks(&claude_days, mb.today);
        let (x_cur, x_long, x_start) = match &self.codex.summary {
            Some(s) => {
                let cur = s.current_streak_days.unwrap_or(0);
                let long = s.longest_streak_days.unwrap_or(cur);
                let last_active = codex_days.iter().rev().find(|(_, t)| **t > 0).map(|(d, _)| *d);
                let start = last_active
                    .filter(|_| cur > 0)
                    .and_then(|e| e.checked_sub_days(Days::new(u64::from(cur - 1))));
                (cur, long, start)
            }
            None => (0, 0, None),
        };

        self.evaluate_milestones(
            now,
            &mb,
            total.multiple,
            &claude_days,
            &codex_days,
            &[(Provider::Claude, c_cur, c_start), (Provider::Codex, x_cur, x_start)],
        );
        if self.current.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.current = None;
        }
        if self.current.is_none() {
            if let Some(c) = self.queue.pop_front() {
                self.current = Some((c, now + CELEBRATE_SECS));
            }
        }
        let celebration = self.current.as_ref().map(|(c, _)| c.clone());
        let celebrating = |who: CelebrationWho| {
            celebration
                .as_ref()
                .is_some_and(|c| c.who == who || c.who == CelebrationWho::Both)
        };

        let moods = vec![
            ProviderMood {
                provider: Provider::Claude,
                mood: mood::mood(&MoodInput {
                    limits: limits[0].state,
                    level: self.claude_level,
                    format_changed,
                    celebrating: celebrating(CelebrationWho::Claude),
                    activity: self.claude_activity(now),
                }),
            },
            ProviderMood {
                provider: Provider::Codex,
                mood: mood::mood(&MoodInput {
                    limits: limits[1].state,
                    level: self.codex_level,
                    format_changed: false,
                    celebrating: celebrating(CelebrationWho::Codex),
                    activity: self.codex.activity(now),
                }),
            },
        ];

        let daily = score::month_days(&mb)
            .into_iter()
            .map(|d| DailyRow {
                date: d.format("%Y-%m-%d").to_string(),
                claude_display: cost.daily_claude_usd.get(&d).copied().unwrap_or(0.0) * conv,
                codex_display: if include_codex {
                    cost.daily_codex_usd.get(&d).copied().unwrap_or(0.0) * conv
                } else {
                    0.0
                },
            })
            .collect();

        Snapshot {
            generated_at: now,
            month: mb.month.clone(),
            currency: settings.currency,
            limits,
            scores,
            total,
            moods,
            tray,
            daily,
            by_source: score::breakdown(&cost.by_source, conv),
            by_model: score::breakdown(&cost.by_model, conv),
            by_project: score::breakdown(&cost.by_project, conv),
            streaks: vec![
                Streak {
                    provider: Provider::Claude,
                    current: c_cur,
                    longest: c_long,
                },
                Streak {
                    provider: Provider::Codex,
                    current: x_cur,
                    longest: x_long,
                },
            ],
            backfill: self.backfill.clone(),
            celebration,
            updated_ago_sec: (now - self.last_change_at).max(0),
            widget_transparency: settings.widget_transparency.min(WIDGET_TRANSPARENCY_MAX),
        }
    }

    /// Milestones are evaluated only on live data: after the first backfill
    /// and the first Codex usage read (or once Codex is known to be missing).
    /// The first evaluation records everything already reached silently
    /// (history and catch-up); later ones celebrate each new id once.
    fn evaluate_milestones(
        &mut self,
        now: i64,
        mb: &score::MonthBounds,
        multiple: Option<f64>,
        claude_days: &BTreeMap<NaiveDate, u64>,
        codex_days: &BTreeMap<NaiveDate, u64>,
        streaks: &[(Provider, u32, Option<NaiveDate>)],
    ) {
        let codex_settled = self.codex.disabled
            || self.codex.usage_seen
            || self.codex.status == Status::NotFound
            || now - self.started_at >= 120;
        if !self.backfill.done || !codex_settled {
            return;
        }
        let silent = !self.milestones_primed;
        self.milestones_primed = true;
        let candidates = score::milestone_candidates(
            &mb.month,
            multiple,
            &[(Provider::Claude, claude_days), (Provider::Codex, codex_days)],
            streaks,
            mb.today,
        );
        for c in candidates {
            if self.store.milestone_known(&c.id) {
                continue;
            }
            let _ = self.store.mark_milestone(&c.id, now, !silent);
            if !silent {
                self.queue.push_back(Celebration {
                    id: c.id,
                    who: c.who,
                    text: c.text,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::ClaudeRecord;
    use crate::claude::FileState;
    use std::sync::atomic::AtomicI64;

    /// A CoreState without threads, for milestone tests.
    fn bare_state(now: Arc<AtomicI64>) -> CoreState {
        let (claude_tx, _rx) = mpsc::channel();
        let clock_now = now.clone();
        let dir = std::env::temp_dir();
        CoreState {
            clock: Arc::new(move || clock_now.load(Ordering::SeqCst)),
            diag: Diag::new(&dir.join("usage-core-test-diag")),
            store: Store::open_in_memory(),
            prices: PriceTable::embedded(),
            fx: FxDay::fallback(),
            settings: Arc::new(Mutex::new(Settings {
                plans: Plans {
                    claude: Some(PlanSetting {
                        amount: 1.0,
                        currency: PlanCurrency::Usd,
                        period: PlanPeriod::Month,
                    }),
                    codex: None,
                },
                ..Settings::default()
            })),
            claude_tx,
            codex_ctl: None,
            t3_wake: mpsc::channel().0,
            fx_stop: None,
            prices_stop: None,
            children: Vec::new(),
            claude_epoch: 0,
            backfill: Backfill {
                done: false,
                files_done: 0,
                files_total: 0,
            },
            claude_dir_exists: true,
            claude_last_ts: None,
            health: Vec::new(),
            codex: CodexView::new(true),
            claude_limits: ClaudeLimitsView::default(),
            claude_level: Level::Normal,
            codex_level: Level::Normal,
            tray_level: Level::Normal,
            milestones_primed: false,
            queue: VecDeque::new(),
            current: None,
            started_at: now.load(Ordering::SeqCst),
            last_change_at: 0,
            dirty: false,
            next_recompute_at: 0,
            last_prune_day: None,
            logged_unpriced: HashSet::new(),
            snap_tx: watch::channel(Snapshot::empty(0, Currency::Eur)).0,
        }
    }

    fn output_record(key: u64, ts: i64, output: u64) -> ClaudeRecord {
        ClaudeRecord {
            ts,
            hour: ts.div_euclid(3600) * 3600,
            model: "claude-opus-5-5".into(),
            entrypoint: "cli".into(),
            project: "alpha".into(),
            speed: "standard".into(),
            service_tier: "standard".into(),
            band: 0,
            tokens: [0, output, 0, 0, 0],
            web_search: 0,
            key,
        }
    }

    fn feed(state: &mut CoreState, key: u64, ts: i64, output: u64) {
        let fs = FileState {
            created: None,
            prefix_len: 0,
            prefix_hash: 0,
            size: 0,
            mtime: 0,
            offset: 0,
        };
        state.apply(
            Update::ClaudeBatch(claude::FileBatch {
                epoch: 0,
                path: "f".into(),
                state: fs,
                records: vec![output_record(key, ts, output)],
                health: Vec::new(),
            }),
            ts,
        );
    }

    #[test]
    fn milestones_backfill_silent_once_per_month_no_refire_on_reprice() {
        let t0 = 1_793_190_000; // 2026-10-28 12:20 UTC
        let now = Arc::new(AtomicI64::new(t0));
        let mut st = bare_state(now.clone());
        // Backfill in progress: 1.5× of a 1 USD plan (75 000 output tokens × 2e-5).
        feed(&mut st, 1, t0 - 86_400 * 3, 75_000);
        let s = st.compute(t0);
        assert!(s.celebration.is_none(), "no celebration during backfill");

        // Backfill done: the first evaluation records 1× silently.
        st.apply(
            Update::ClaudeProgress {
                epoch: 0,
                files_done: 1,
                files_total: 1,
                done: true,
                dir_exists: true,
            },
            t0,
        );
        let s = st.compute(t0);
        assert!(s.celebration.is_none(), "history must not celebrate");
        assert!(st.store.milestone_known("multiple:2026-10:1"));

        // Live: crossing 2× celebrates once, for both characters.
        feed(&mut st, 2, t0 - 60, 30_000);
        let s = st.compute(t0);
        let c = s.celebration.clone().expect("2× celebration");
        assert_eq!(c.id, "multiple:2026-10:2");
        assert_eq!(c.who, CelebrationWho::Both);
        assert_eq!(s.mood_for(Provider::Claude), Some(Mood::Celebrate));
        // Lasts 4 s, then gone and never again.
        let s = st.compute(t0 + 3);
        assert!(s.celebration.is_some());
        let s = st.compute(t0 + 4);
        assert!(s.celebration.is_none());
        let s = st.compute(t0 + 10);
        assert!(s.celebration.is_none());

        // Repricing (a new table) does not fire the same milestone again.
        st.prices = PriceTable::embedded();
        let s = st.compute(t0 + 20);
        assert!(s.celebration.is_none());
        assert_eq!(st.queue.len(), 0);
    }

    #[test]
    fn health_check_flags_format_change() {
        let now = Arc::new(AtomicI64::new(1_793_190_000));
        let mut st = bare_state(now);
        let t = 1_793_190_000 - 100;
        let fs = FileState {
            created: None,
            prefix_len: 0,
            prefix_hash: 0,
            size: 0,
            mtime: 0,
            offset: 0,
        };
        // 25 assistant lines, 6 without usage → 24 % > 20 %.
        let mut health: Vec<(i64, bool)> = (0..19).map(|_| (t, true)).collect();
        health.extend((0..6).map(|_| (t, false)));
        st.apply(
            Update::ClaudeBatch(claude::FileBatch {
                epoch: 0,
                path: "f".into(),
                state: fs.clone(),
                records: Vec::new(),
                health,
            }),
            t,
        );
        let s = st.compute(1_793_190_000);
        assert_eq!(s.score_for(Provider::Claude).unwrap().state, ScoreState::FormatChanged);
        assert_eq!(s.mood_for(Provider::Claude), Some(Mood::Confused));
        // An hour later the old lines no longer count.
        let s = st.compute(1_793_190_000 + 3700);
        assert_eq!(s.score_for(Provider::Claude).unwrap().state, ScoreState::Ok);

        // Exactly 20 % (5 of 25) is not a format change.
        let mut st = bare_state(Arc::new(AtomicI64::new(1_793_190_000)));
        let mut health: Vec<(i64, bool)> = (0..20).map(|_| (t, true)).collect();
        health.extend((0..5).map(|_| (t, false)));
        st.apply(
            Update::ClaudeBatch(claude::FileBatch {
                epoch: 0,
                path: "f".into(),
                state: fs,
                records: Vec::new(),
                health,
            }),
            t,
        );
        assert_eq!(
            st.compute(1_793_190_000).score_for(Provider::Claude).unwrap().state,
            ScoreState::Ok
        );
    }
}
