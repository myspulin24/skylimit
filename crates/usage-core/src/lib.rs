//! usage-core: the runtime behind Skylimit.
//!
//! One core thread owns the SQLite connection (WAL) and computes score, moods,
//! tray color and the [`Snapshot`]. Providers run on their own threads, keep
//! their own schedules and send updates over a channel:
//!
//! ```text
//!  <projects>/**/*.jsonl ──notify + 30 s poll──▶ claude.rs ─┐
//!  codex app-server (Job Object) ──JSON-RPC────▶ codex.rs  ─┤ Update
//!  T3 Code provider logs ───────5 s poll───────▶ t3.rs     ─┤ claude_limits.rs
//!  Claude Code status line files ─5 s poll────▶ claude_code.rs ┤
//!  Claude Code ~/.claude.json ──5 s, on change▶ claude_config.rs┤
//!  ČNB daily fixing (≤ 1×/day) ────────────────▶ fx.rs     ─┤
//!  T3 Code usage cache (deleted transcripts) ──▶ t3_history.rs (core thread)
//!                                                           ▼
//!             core thread: store.rs ─▶ pricing.rs ─▶ score.rs ─▶ mood.rs
//!                                                           │ watch
//!                                                        Snapshot (no secrets)
//! ```
//!
//! The History tab asks the core thread for a period on demand
//! ([`CoreHandle::history`], `history.rs`); it never rides in the snapshot.
//!
//! Claude limits never touch Claude login data and never call Anthropic
//! (terms gate): they come from the `rate_limit_event` messages the official
//! Claude Agent SDK hands to T3 Code, which logs them locally (`t3.rs`), and
//! from the `rate_limits` Claude Code passes to its status line command
//! (`claude_code.rs`), and from the limits Claude Code caches in its global
//! config (`claude_config.rs`, also the weekly Fable limit). Without any of
//! them they are Off and Claude's mood comes from score and activity only.

mod claude;
pub mod claude_code;
pub mod claude_config;
mod claude_limits;
mod codex;
mod diag;
mod fx;
mod history;
pub mod model;
mod mood;
mod pricing;
mod score;
mod store;
mod t3;
mod t3_history;
mod user_path;

pub use model::*;
/// Re-exported so the shell does not need its own `tokio` dependency.
pub use tokio::sync::watch;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
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
use crate::claude_limits::{ClaudeLimitsThread, ClaudeLimitsView, ClaudeReading, ClaudeWindow, Sources};

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

/// Note on Claude's limits row when neither T3 Code nor Claude Code reports them.
pub const CLAUDE_LIMITS_NOTE: &str = claude_limits::NOTE_OFF;

const TICK: Duration = Duration::from_millis(250);
/// Recompute at least this often (seconds of the injected clock) so time-based
/// states (Stale, sleep, month change) move without new data.
const PERIODIC_SECS: i64 = 30;
const CELEBRATE_SECS: i64 = 4;
const HEALTH_WINDOW: i64 = 3600;
const LIMIT_HISTORY_DAYS: i64 = 90;
/// How often T3 Code's usage cache is checked for a change.
const T3_HISTORY_CHECK_SECS: i64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexSource {
    /// Spawn this program speaking the app-server protocol on stdio. With
    /// `login_path`, the child first gets the PATH of the user's interactive
    /// login shell (`user_path.rs`): a macOS app started outside a terminal
    /// does not have it, and Codex would not be found.
    Spawn {
        program: String,
        args: Vec<String>,
        login_path: bool,
    },
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
                login_path: false,
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
                login_path: true,
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
    /// one source of Claude's limits.
    pub t3_logs_dir: Option<PathBuf>,
    /// Where `skylimit statusline` keeps Claude Code's limits
    /// (`<data>/claude-code`, see [`claude_code`]); the other source. It
    /// exists only while Claude Code is connected. Both `None` = Claude
    /// limits Off.
    pub claude_code_dir: Option<PathBuf>,
    /// Claude Code's `.claude.json` (see [`claude_config`]): the limits
    /// Claude Code caches for itself, the only source of the Fable limit.
    pub claude_config_file: Option<PathBuf>,
    /// T3 Code's data folder (`%USERPROFILE%\.t3\userdata`): its usage cache
    /// restores Claude history whose transcripts Claude Code already deleted
    /// (see `t3_history.rs`).
    pub t3_userdata_dir: Option<PathBuf>,
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
            claude_code_dir: None,
            claude_config_file: None,
            t3_userdata_dir: None,
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
        sources: Sources,
        /// New readings, at most one per source.
        readings: Vec<ClaudeReading>,
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
    History {
        range: HistoryRange,
        filter: HistoryFilter,
        reply: Sender<History>,
    },
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

        let (claude_limits_wake, wake_rx) = mpsc::channel();
        let claude_limit_sources =
            config.t3_logs_dir.is_some() || config.claude_code_dir.is_some() || config.claude_config_file.is_some();
        if claude_limit_sources {
            children.push(
                ClaudeLimitsThread {
                    t3_dir: config.t3_logs_dir.clone(),
                    code_dir: config.claude_code_dir.clone(),
                    config_file: config.claude_config_file.clone(),
                    core: tx.clone(),
                    wake: wake_rx,
                    stop: stop.clone(),
                    clock: clock.clone(),
                    diag: diag.clone(),
                }
                .spawn(),
            );
        }

        let codex_disabled = config.codex == CodexSource::Disabled;
        let codex_ctl = match &config.codex {
            CodexSource::Spawn { program, args, login_path } => {
                let (ctl_tx, ctl_rx) = mpsc::channel();
                children.push(
                    CodexThread {
                        program: program.clone(),
                        args: args.clone(),
                        login_path: *login_path,
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
        let present = |dir: &Option<PathBuf>| dir.as_deref().is_some_and(|d| d.is_dir());
        let (reading, fable) = if claude_limit_sources {
            restore_claude_limits(&store, now)
        } else {
            (None, None)
        };
        let claude_limits = ClaudeLimitsView {
            available: Sources {
                t3: present(&config.t3_logs_dir),
                code: present(&config.claude_code_dir),
            },
            reading,
            fable,
        };
        let codex_account = Arc::new(Mutex::new(codex.account()));
        let state = CoreState {
            fx: store.latest_fx().unwrap_or_else(FxDay::fallback),
            claude_last_ts: store.meta_get("claude_last_ts").and_then(|v| v.parse().ok()),
            claude_fable_last_ts: store.meta_get("claude_fable_last_ts").and_then(|v| v.parse().ok()),
            codex_account: codex_account.clone(),
            claude_dir_exists: config.claude_projects_dir.is_dir(),
            t3_userdata_dir: config.t3_userdata_dir.clone(),
            t3_cache_seen: HashMap::new(),
            t3_next_check: now,
            clock,
            diag,
            store,
            prices: PriceTable::embedded(),
            settings: settings.clone(),
            claude_tx,
            codex_ctl,
            claude_limits_wake,
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
            codex_account,
            stop,
            thread: Some(thread),
        }
    }
}

pub struct CoreHandle {
    tx: Sender<CoreMsg>,
    snapshot: watch::Receiver<Snapshot>,
    settings: Arc<Mutex<Settings>>,
    /// Kept apart from the snapshot: the e-mail never goes to the widget.
    codex_account: Arc<Mutex<Option<CodexAccount>>>,
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

    /// The account the Codex CLI is signed in with (Settings › Účty).
    pub fn codex_account(&self) -> Option<CodexAccount> {
        self.codex_account.lock().map(|a| a.clone()).unwrap_or_default()
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

    /// History tab: consumption over `range`, priced now. Waits for the core
    /// thread (up to 10 s); `None` when it does not answer.
    pub fn history(&self, range: HistoryRange, filter: HistoryFilter) -> Option<History> {
        let (reply, rx) = mpsc::channel();
        self.tx.send(CoreMsg::History { range, filter, reply }).ok()?;
        rx.recv_timeout(Duration::from_secs(10)).ok()
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
                account: Default::default(),
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

/// Last Claude limits from the database (T3 Code logs rotate away, Claude
/// Code sessions end): the 5h and weekly reading, and the Fable window,
/// which has its own time.
fn restore_claude_limits(store: &Store, now: i64) -> (Option<ClaudeReading>, Option<(i64, ClaudeWindow)>) {
    let recent = |ts: i64| now - ts < LIMIT_HISTORY_DAYS * 86_400;
    let window = |r: &store::LimitRow| {
        r.percent.map(|percent| ClaudeWindow {
            percent,
            resets_at: r.resets_at,
        })
    };
    let rows = store.latest_limit_snapshot("claude");
    let reading = rows.first().map(|r| r.ts).filter(|ts| recent(*ts)).and_then(|at| {
        let named = |name: &str| rows.iter().find(|r| r.win == name).and_then(window);
        let reading = ClaudeReading {
            at,
            five_hour: named("5h"),
            seven_day: named("week"),
            fable: None,
        };
        reading.has_main().then_some(reading)
    });
    let fable = store
        .latest_limit_row("claude", FABLE_WIN)
        .filter(|r| recent(r.ts))
        .and_then(|r| Some((r.ts, window(&r)?)));
    (reading, fable)
}

/// Window name of the weekly Fable limit in `limit_snapshots`.
const FABLE_WIN: &str = "fable";

struct CoreState {
    clock: Clock,
    diag: Diag,
    store: Store,
    prices: PriceTable,
    fx: FxDay,
    settings: Arc<Mutex<Settings>>,
    claude_tx: Sender<ClaudeIn>,
    codex_ctl: Option<Sender<CodexCtl>>,
    /// Polls T3 Code and Claude Code now (refresh, Claude Code connected).
    claude_limits_wake: Sender<()>,
    fx_stop: Option<Sender<()>>,
    prices_stop: Option<Sender<()>>,
    children: Vec<JoinHandle<()>>,

    claude_epoch: u64,
    backfill: Backfill,
    claude_dir_exists: bool,
    t3_userdata_dir: Option<PathBuf>,
    /// Modification time of each T3 cache file when it was last imported.
    t3_cache_seen: HashMap<PathBuf, i64>,
    t3_next_check: i64,
    claude_last_ts: Option<i64>,
    /// Newest assistant line of a Fable model: only Fable work ages the
    /// weekly Fable window.
    claude_fable_last_ts: Option<i64>,
    /// What `CoreHandle::codex_account` returns; rewritten on Codex events.
    codex_account: Arc<Mutex<Option<CodexAccount>>>,
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
            self.import_t3_history(now);
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
                let _ = self.claude_limits_wake.send(());
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
            CoreMsg::History { range, filter, reply } => {
                let _ = reply.send(self.history(range, filter, now));
            }
            CoreMsg::Update(u) => self.apply(u, now),
        }
        true
    }

    fn history(&self, range: HistoryRange, filter: HistoryFilter, now: i64) -> History {
        let settings = self.settings.lock().map(|s| s.clone()).unwrap_or_default();
        let codex_days = self.store.codex_buckets().unwrap_or_default();
        let codex_enabled = !self.codex.disabled;
        let first_data_at = self.first_data_at(filter, &codex_days);
        let period = history::Period::new(range, now, first_data_at);
        let rows = self.store.usage_rows(period.from, period.query_end).unwrap_or_default();
        let web = self.store.web_rows(period.from, period.query_end).unwrap_or_default();
        history::build(
            range,
            filter,
            now,
            &history::Inputs {
                rows: &rows,
                web: &web,
                codex_days: &codex_days,
                prices: &self.prices,
                codex_rate: self.prices.codex_blended_usd_per_token(),
                codex_enabled,
                excluded: &[],
                conv: self.fx.usd_to(settings.currency),
                currency: settings.currency,
                first_data_at,
            },
        )
    }

    /// Claude history from T3 Code's usage cache for transcripts Claude Code
    /// already deleted (`t3_history.rs`). Waits for the backfill (the layout
    /// check needs Skylimit's own numbers), then rereads a cache file only
    /// when it changed, checked every 10 minutes.
    fn import_t3_history(&mut self, now: i64) {
        let Some(dir) = self.t3_userdata_dir.clone() else { return };
        if !self.backfill.done || now < self.t3_next_check {
            return;
        }
        self.t3_next_check = now + T3_HISTORY_CHECK_SECS;
        let mut added = 0;
        for (path, mtime) in t3_history::cache_files(&dir) {
            if self.t3_cache_seen.get(&path) == Some(&mtime) {
                continue;
            }
            self.t3_cache_seen.insert(path.clone(), mtime);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(messages) = std::fs::read_to_string(&path).ok().and_then(|t| t3_history::parse(&t)) else {
                self.diag.log(now, "t3-history", &format!("{name}: not the known layout, skipped"));
                continue;
            };
            let store = &self.store;
            let plan = t3_history::plan(
                &messages,
                |key| store.seen_tokens(key),
                |p| Path::new(p).exists(),
                t3_history::project_from_folder,
            );
            match plan {
                Ok(plan) if !plan.records.is_empty() => match self.store.apply_records(&plan.records) {
                    Ok(0) => {}
                    Ok(n) => {
                        added += n;
                        self.diag.log(
                            now,
                            "t3-history",
                            &format!(
                                "{name}: {n} messages from {} deleted transcripts restored ({} of {} known messages agree)",
                                plan.files, plan.agree, plan.overlap
                            ),
                        );
                    }
                    Err(e) => self.diag.log(now, "t3-history", &format!("{name}: not stored: {e}")),
                },
                Ok(_) => {}
                Err(e) => self.diag.log(now, "t3-history", &format!("{name}: {e}")),
            }
        }
        if added > 0 {
            // Old days are not live usage: a record they set is noted silently.
            self.milestones_primed = false;
            self.last_change_at = now;
            self.dirty = true;
        }
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
                let newest_fable = b
                    .records
                    .iter()
                    .filter(|r| r.ts <= now + 300 && r.model.to_ascii_lowercase().contains("fable"))
                    .map(|r| r.ts)
                    .max();
                if let Some(ts) = newest_fable {
                    if self.claude_fable_last_ts.is_none_or(|t| ts > t) {
                        self.claude_fable_last_ts = Some(ts);
                        let _ = self.store.meta_set("claude_fable_last_ts", &ts.to_string());
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
                self.publish_codex_account();
                self.dirty = true;
            }
            Update::ClaudeLimits { sources, readings } => {
                self.claude_limits.available = sources;
                for r in readings {
                    let at = r.at;
                    let (main, fable) = self.claude_limits.take(r);
                    let mut rows = Vec::new();
                    if let Some(r) = self.claude_limits.reading.as_ref().filter(|_| main) {
                        rows.push(("5h", r.five_hour.clone()));
                        rows.push(("week", r.seven_day.clone()));
                    }
                    if let Some((_, w)) = self.claude_limits.fable.as_ref().filter(|_| fable) {
                        rows.push((FABLE_WIN, Some(w.clone())));
                    }
                    for (win, w) in rows {
                        let _ = self.store.add_limit_snapshot(
                            at,
                            "claude",
                            win,
                            w.as_ref().map(|w| w.percent),
                            w.as_ref().and_then(|w| w.resets_at),
                        );
                    }
                    if main || fable {
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
        self.t3_cache_seen.clear();
        self.t3_next_check = now;
        self.claude_last_ts = None;
        self.claude_fable_last_ts = None;
        self.codex.usage_seen = false;
        self.codex.summary = None;
        self.publish_codex_account();
        self.queue.clear();
        self.current = None;
        self.milestones_primed = false;
        self.last_change_at = now;
        self.dirty = true;
    }

    fn publish_codex_account(&self) {
        if let Ok(mut a) = self.codex_account.lock() {
            *a = self.codex.account();
        }
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
            self.claude_limits.provider_limits(now, self.claude_last_ts, self.claude_fable_last_ts),
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

        let daily: Vec<DailyRow> = score::month_days(&mb)
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

        let widget = self.widget_score(
            now,
            &settings,
            (&scores, &total, &daily),
            format_changed,
            codex_state,
            &all_buckets,
        );

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
            widget,
        }
    }

    /// The widget's score (Settings › Widget). The month in API prices is the
    /// snapshot's own; another period or the cache savings come from
    /// `history::aggregate`, priced like the month, honouring "Co se počítá do
    /// skóre", with the plans pro-rated to the period's days.
    fn widget_score(
        &self,
        now: i64,
        settings: &Settings,
        month: (&[ProviderScore], &Total, &[DailyRow]),
        format_changed: bool,
        codex_state: ScoreState,
        codex_days: &[(String, u64)],
    ) -> WidgetScore {
        let (period_setting, amount) = (settings.widget_period, settings.widget_amount);
        if period_setting == WidgetPeriod::Month && amount == WidgetAmount::Api {
            let (scores, total, daily) = month;
            return WidgetScore {
                period: period_setting,
                amount,
                scores: scores.to_vec(),
                total: total.clone(),
                spark: daily.iter().map(|d| d.claude_display + d.codex_display).collect(),
            };
        }
        let range = match period_setting {
            WidgetPeriod::Month => None,
            WidgetPeriod::Day => Some(HistoryRange::Day),
            WidgetPeriod::Week => Some(HistoryRange::Week),
            WidgetPeriod::Days30 => Some(HistoryRange::Month),
            WidgetPeriod::Days90 => Some(HistoryRange::Quarter),
            WidgetPeriod::All => Some(HistoryRange::All),
        };
        let period = match range {
            None => history::Period::new_month(now),
            Some(r) => history::Period::new(r, now, self.first_data_at(HistoryFilter::All, codex_days)),
        };
        let rows = self.store.usage_rows(period.from, period.query_end).unwrap_or_default();
        let web = self.store.web_rows(period.from, period.query_end).unwrap_or_default();
        let conv = self.fx.usd_to(settings.currency);
        let codex_rate = self.prices.codex_blended_usd_per_token();
        let codex_enabled = !self.codex.disabled;
        let agg = history::aggregate(
            &period,
            HistoryFilter::All,
            &history::Inputs {
                rows: &rows,
                web: &web,
                codex_days,
                prices: &self.prices,
                codex_rate,
                codex_enabled,
                excluded: &settings.excluded_entrypoints,
                conv,
                currency: settings.currency,
                first_data_at: None,
            },
        );
        let savings = amount == WidgetAmount::WithCacheSavings;
        let hourly = period.bucket == HistoryBucket::Hour;
        let cost = score::MonthCost {
            claude: score::ProviderTotals {
                usd: agg.claude.usd + if savings { agg.savings } else { 0.0 },
                tokens: agg.claude.tokens,
                unpriced: agg.claude_unpriced,
            },
            codex: score::ProviderTotals {
                usd: agg.codex.usd,
                tokens: agg.codex.tokens,
                unpriced: agg.codex_unpriced,
            },
            ..score::MonthCost::default()
        };
        let claude_state = if format_changed {
            ScoreState::FormatChanged
        } else if !self.claude_dir_exists && agg.claude.tokens == 0 {
            ScoreState::Unknown
        } else if agg.claude_unpriced > 0 {
            ScoreState::Partial
        } else {
            ScoreState::Ok
        };
        // Codex reports days: it has no place in the last 24 hours.
        let codex_state = if hourly { ScoreState::Off } else { codex_state };
        let gap = (hourly && codex_enabled).then_some("Codex hlásí jen dny");
        let plan_factor = match range {
            None => 1.0,
            Some(_) => period.day_count() as f64 / (365.25 / 12.0),
        };
        let (scores, total) =
            score::scores_and_total_for(&cost, claude_state, codex_state, settings, &self.fx, plan_factor, gap);
        let with_codex = codex_state == ScoreState::Estimate && settings.codex_estimate_in_score;
        let spark = (0..agg.claude_pts.len())
            .map(|i| {
                let claude = agg.claude_pts[i].usd + if savings { agg.savings_pts[i] } else { 0.0 };
                let codex = if with_codex { agg.codex_pts[i].usd } else { 0.0 };
                (claude + codex) * conv
            })
            .collect();
        WidgetScore {
            period: period_setting,
            amount,
            scores,
            total,
            spark,
        }
    }

    /// Oldest record of the providers a filter shows: how far "all" reaches.
    fn first_data_at(&self, filter: HistoryFilter, codex_days: &[(String, u64)]) -> Option<i64> {
        let first_claude = if filter == HistoryFilter::Codex {
            None
        } else {
            self.store.first_usage_hour("claude")
        };
        let first_codex = if filter == HistoryFilter::Claude || self.codex.disabled {
            None
        } else {
            codex_days
                .iter()
                .filter(|(_, t)| *t > 0)
                .filter_map(|(d, _)| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .min()
                .map(score::local_midnight)
        };
        first_claude.into_iter().chain(first_codex).min()
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
    use crate::pricing::TokenKind;
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
            claude_limits_wake: mpsc::channel().0,
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
            t3_userdata_dir: None,
            t3_cache_seen: HashMap::new(),
            t3_next_check: 0,
            claude_last_ts: None,
            claude_fable_last_ts: None,
            codex_account: Arc::new(Mutex::new(None)),
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
    fn t3_cache_restores_deleted_transcripts_once_and_history_shows_them() {
        let t0 = 1_793_190_000;
        let mut st = bare_state(Arc::new(AtomicI64::new(t0)));
        let tmp = tempfile::tempdir().unwrap();
        st.t3_userdata_dir = Some(tmp.path().to_path_buf());
        let live = tmp.path().join("C--dev-alpha").join("live.jsonl");
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        std::fs::write(&live, "").unwrap();
        let gone = tmp.path().join("C--dev-beta").join("gone.jsonl");
        let key = |i: usize| claude::fnv1a64(&[format!("msg_{i}").as_bytes(), b"\x1f", format!("req_{i}").as_bytes()]);
        let old = t0 - 60 * 86_400;
        let mut tuples = Vec::new();
        for i in 0..25 {
            // Counted by Skylimit from the live transcript.
            feed(&mut st, key(i), t0 - 3600, 100);
            tuples.push((live.clone(), t0 - 3600, i));
        }
        for i in 100..103 {
            tuples.push((gone.clone(), old, i));
        }
        let mut files = serde_json::Map::new();
        for (path, ts, i) in tuples {
            let entry = files
                .entry(path.to_string_lossy().into_owned())
                .or_insert_with(|| serde_json::json!({ "p": "claude", "r": [] }));
            entry["r"].as_array_mut().unwrap().push(serde_json::json!([
                ts * 1000, 0, 0, 0, 0, 0, 100, 0, format!("msg_{i}:req_{i}"), null, 0, null
            ]));
        }
        let cache = serde_json::json!({ "version": 9, "models": ["claude-opus-5-5"], "sessions": [], "files": files });
        std::fs::write(tmp.path().join("usage-scan-cache-v9.json"), cache.to_string()).unwrap();

        // Not before the backfill: the layout check needs Skylimit's own numbers.
        st.import_t3_history(t0);
        assert_eq!(st.store.first_usage_hour("claude"), Some((t0 - 3600).div_euclid(3600) * 3600));

        st.backfill.done = true;
        st.import_t3_history(t0);
        assert_eq!(st.store.first_usage_hour("claude"), Some(old.div_euclid(3600) * 3600));

        let h = st.history(HistoryRange::All, HistoryFilter::All, t0);
        assert_eq!(h.total_tokens, 28 * 100);
        assert_eq!(h.first_data_at, Some(old.div_euclid(3600) * 3600));
        let restored = h.by_source.iter().find(|r| r.key == t3_history::ENTRYPOINT).unwrap();
        assert_eq!(restored.tokens, 300);
        let project = t3_history::project_from_folder("C--dev-beta");
        assert_eq!(h.by_project.iter().find(|r| r.key == project).map(|r| r.tokens), Some(300));
        let week = st.history(HistoryRange::Week, HistoryFilter::Claude, t0);
        assert_eq!(week.total_tokens, 25 * 100, "the old days stay out of the week");

        // Unchanged cache: not read again; read again anyway: nothing doubles.
        st.import_t3_history(t0 + T3_HISTORY_CHECK_SECS);
        st.t3_cache_seen.clear();
        st.import_t3_history(t0 + 2 * T3_HISTORY_CHECK_SECS);
        assert_eq!(st.history(HistoryRange::All, HistoryFilter::All, t0).total_tokens, 28 * 100);
    }

    #[test]
    fn widget_score_follows_its_period_and_amount() {
        let t0 = 1_793_190_000; // 2026-10-28 12:20 UTC
        let mut st = bare_state(Arc::new(AtomicI64::new(t0)));
        let record = |key: u64, ts: i64, entrypoint: &str, tokens: [u64; 5]| ClaudeRecord {
            entrypoint: entrypoint.into(),
            tokens,
            ..output_record(key, ts, 0)
        };
        let batch = |st: &mut CoreState, records: Vec<ClaudeRecord>| {
            st.apply(
                Update::ClaudeBatch(claude::FileBatch {
                    epoch: 0,
                    path: "f".into(),
                    state: FileState {
                        created: None,
                        prefix_len: 0,
                        prefix_hash: 0,
                        size: 0,
                        mtime: 0,
                        offset: 0,
                    },
                    records,
                    health: Vec::new(),
                }),
                t0,
            )
        };
        batch(
            &mut st,
            vec![
                // Today: 1 M cache reads and 10 k output.
                record(1, t0 - 600, "cli", [0, 10_000, 1_000_000, 0, 0]),
                // 20 days ago (this month, outside the week).
                record(2, t0 - 20 * 86_400, "cli", [0, 50_000, 0, 0, 0]),
                // An excluded source today.
                record(3, t0 - 600, "sdk-ts", [0, 70_000, 0, 0, 0]),
            ],
        );
        let unit = |k: TokenKind| {
            st.prices
                .lookup("claude-opus-5-5")
                .unwrap()
                .unit_price(k, 0, "standard", "standard")
                .unwrap()
        };
        let (read, out, input) = (unit(TokenKind::CacheRead), unit(TokenKind::Output), unit(TokenKind::Input));
        let conv = st.fx.usd_to(Currency::Eur);
        if let Ok(mut s) = st.settings.lock() {
            s.excluded_entrypoints = vec!["sdk-ts".into()];
        }

        // Default: the month, exactly the snapshot's own score.
        let s = st.compute(t0);
        assert_eq!(s.widget.period, WidgetPeriod::Month);
        assert_eq!(s.widget.total, s.total);
        assert_eq!(s.widget.scores, s.scores);
        assert_eq!(s.widget.spark.len(), s.daily.len());

        // Last 7 days in API prices, plan pro-rated to 7 days.
        if let Ok(mut s) = st.settings.lock() {
            s.widget_period = WidgetPeriod::Week;
        }
        let s = st.compute(t0);
        let week_usd = 1e6 * read + 1e4 * out;
        assert!((s.widget.total.display - week_usd * conv).abs() < 1e-9);
        let plan = conv * 7.0 / (365.25 / 12.0);
        assert!((s.widget.total.plan_monthly_display.unwrap() - plan).abs() < 1e-9);
        assert!((s.widget.total.multiple.unwrap() - week_usd * conv / plan).abs() < 1e-9);
        assert_eq!(s.widget.spark.len(), 7);
        assert!(s.total.display > s.widget.total.display, "the month also has the day 20 days ago");

        // With the cache savings on top.
        if let Ok(mut s) = st.settings.lock() {
            s.widget_amount = WidgetAmount::WithCacheSavings;
        }
        let s = st.compute(t0);
        let saved = 1e6 * (input - read);
        assert!((s.widget.total.display - (week_usd + saved) * conv).abs() < 1e-9);
        let claude = s.widget.scores.iter().find(|x| x.provider == Provider::Claude).unwrap();
        assert!((claude.display - (week_usd + saved) * conv).abs() < 1e-9);

        // The month with savings keeps the month's days.
        if let Ok(mut s) = st.settings.lock() {
            s.widget_period = WidgetPeriod::Month;
        }
        let s = st.compute(t0);
        assert!((s.widget.total.display - (week_usd + 5e4 * out + saved) * conv).abs() < 1e-9);
        assert_eq!(s.widget.total.plan_monthly_display, s.total.plan_monthly_display);
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
