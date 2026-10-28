//! Codex source: the official `codex app-server` over JSON-RPC 2.0
//! (newline-delimited, stdio). The CLI owns the ChatGPT login; this module
//! never reads the Codex login file.
//!
//! ```text
//!  spawn (Job Object, kill-on-close) ─▶ initialize ─▶ initialized
//!        ▲                                   │
//!        │ backoff 5 s … 15 min              ├─ account/rateLimits/read  every 2 min
//!        │                                   │    (≥ 60 s apart, error backoff ≤ 15 min,
//!     EOF / timeout ◀────────────────────────┤     now on account/rateLimits/updated)
//!                                            └─ account/usage/read       every 5 min
//! ```
//!
//! Limits map by POSITION (decision D7): `primary` → 5h, `secondary` → week.
//! A window is Known while the last successful read is ≤ 10 min old, Stale
//! after that, Unknown when never read, and AfterReset once its `resetsAt`
//! passes while Stale (no percent, does not drive mood or tray).
//!
//! Cost (spike result): `account/usage/read` returns only daily token totals
//! (`dailyUsageBuckets`), no model split, so the core prices them with a
//! blended rate (`pricing::CODEX_BLEND_MODEL`). Buckets are authoritative and
//! upserted by date. Activity: a rise of the 5h `usedPercent` or of any daily
//! bucket between two reads counts until the next scheduled read + 5 min.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::diag::{json_shape, Diag};
use crate::model::{LimitWindow, LimitsState, Provider, ProviderLimits, WindowKind, WindowState};
use crate::mood::Activity;
use crate::{Clock, CoreMsg, Update};

const TICK: Duration = Duration::from_millis(250);
const LIMITS_EVERY: i64 = 120;
const USAGE_EVERY: i64 = 300;
const MIN_GAP: i64 = 60;
const MAX_BACKOFF: i64 = 900;
const SPAWN_BACKOFF_START: i64 = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Activity lasts until the next scheduled read plus this.
const ACTIVITY_GRACE: i64 = 300;
/// Known while the last successful read is at most this old.
pub(crate) const FRESH_SECS: i64 = 600;
const SLEEP_AFTER: i64 = 1800;

pub(crate) const NOTE_NOT_FOUND: &str = "Codex CLI nenalezen";
pub(crate) const NOTE_DOWN: &str = "Codex app-server neodpovídá, restartuji";
pub(crate) const NOTE_READ_FAILED: &str = "Limity Codexu nejdou načíst";
pub(crate) const NOTE_DISABLED: &str = "Codex vypnutý";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RateWindow {
    pub used_percent: f64,
    pub window_mins: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LimitsReading {
    pub at: i64,
    pub primary: Option<RateWindow>,
    pub secondary: Option<RateWindow>,
    /// Extra limit ids (`rateLimitsByLimitId` other than `codex`): label, kind, window.
    pub extra: Vec<(String, WindowKind, RateWindow)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct UsageSummary {
    pub lifetime_tokens: Option<u64>,
    pub peak_daily_tokens: Option<u64>,
    pub current_streak_days: Option<u32>,
    pub longest_streak_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UsageReading {
    pub at: i64,
    pub summary: UsageSummary,
    pub buckets: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CodexEvent {
    /// `initialize` answered.
    Running { at: i64 },
    /// Could not be started and never answered in this run.
    NotFound,
    /// Was running, then exited or stopped answering; restart scheduled.
    Down,
    Limits(LimitsReading),
    LimitsFailed,
    Usage(UsageReading),
    UsageFailed,
    Activity { at: i64, until: i64 },
}

// ---------------------------------------------------------------- parsing

fn window(v: &Value) -> Option<RateWindow> {
    Some(RateWindow {
        used_percent: v.get("usedPercent")?.as_f64()?,
        window_mins: v.get("windowDurationMins").and_then(Value::as_i64),
        resets_at: v.get("resetsAt").and_then(Value::as_i64),
    })
}

pub(crate) fn mins_label(mins: Option<i64>) -> String {
    match mins {
        Some(300) => "5h".into(),
        Some(10080) => "týden".into(),
        Some(m) => format!("okno {m} min"),
        None => "okno".into(),
    }
}

/// `result` of `account/rateLimits/read`. `None` = unknown shape.
pub(crate) fn parse_rate_limits(result: &Value, at: i64) -> Option<LimitsReading> {
    let rl = result.get("rateLimits")?.as_object()?;
    if !rl.contains_key("primary") && !rl.contains_key("secondary") {
        return None;
    }
    let mut extra = Vec::new();
    if let Some(by_id) = result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        for (id, entry) in by_id {
            if id == "codex" {
                continue; // same as `rateLimits`
            }
            let name = entry
                .get("limitName")
                .and_then(Value::as_str)
                .or_else(|| entry.get("normalModelSlug").and_then(Value::as_str))
                .unwrap_or(id);
            for slot in ["primary", "secondary"] {
                if let Some(w) = entry.get(slot).and_then(window) {
                    let kind = if w.window_mins == Some(10080) {
                        WindowKind::WeekModel
                    } else {
                        WindowKind::Other
                    };
                    extra.push((format!("{} · {name}", mins_label(w.window_mins)), kind, w));
                }
            }
        }
    }
    Some(LimitsReading {
        at,
        primary: rl.get("primary").and_then(window),
        secondary: rl.get("secondary").and_then(window),
        extra,
    })
}

/// `result` of `account/usage/read` (null params). `None` = unknown shape.
pub(crate) fn parse_usage(result: &Value, at: i64) -> Option<UsageReading> {
    let summary = result.get("summary");
    let buckets = result.get("dailyUsageBuckets");
    if summary.is_none() && buckets.is_none() {
        return None;
    }
    let num = |k: &str| summary.and_then(|s| s.get(k)).and_then(Value::as_u64);
    let summary = UsageSummary {
        lifetime_tokens: num("lifetimeTokens"),
        peak_daily_tokens: num("peakDailyTokens"),
        current_streak_days: num("currentStreakDays").map(|v| v as u32),
        longest_streak_days: num("longestStreakDays").map(|v| v as u32),
    };
    let buckets = buckets
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|b| {
                    let date = b.get("startDate")?.as_str()?;
                    if date.len() != 10 {
                        return None;
                    }
                    Some((date.to_string(), b.get("tokens")?.as_u64()?))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(UsageReading { at, summary, buckets })
}

// ---------------------------------------------------------------- core-side view

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Starting,
    Running,
    NotFound,
    Down,
}

/// What the core knows about Codex. Owned by the core thread.
#[derive(Debug, Clone)]
pub(crate) struct CodexView {
    pub disabled: bool,
    pub status: Status,
    pub limits: Option<LimitsReading>,
    pub last_limits_failed: bool,
    pub usage_seen: bool,
    pub summary: Option<UsageSummary>,
    pub active_until: Option<i64>,
    pub last_rise_at: Option<i64>,
    pub running_since: Option<i64>,
}

impl CodexView {
    pub(crate) fn new(disabled: bool) -> Self {
        CodexView {
            disabled,
            status: Status::Starting,
            limits: None,
            last_limits_failed: false,
            usage_seen: false,
            summary: None,
            active_until: None,
            last_rise_at: None,
            running_since: None,
        }
    }

    pub(crate) fn apply(&mut self, ev: &CodexEvent) {
        match ev {
            CodexEvent::Running { at } => {
                self.status = Status::Running;
                self.running_since = Some(*at);
            }
            CodexEvent::NotFound => {
                self.status = Status::NotFound;
                self.running_since = None;
            }
            CodexEvent::Down => {
                self.status = Status::Down;
                self.running_since = None;
            }
            CodexEvent::Limits(r) => {
                self.limits = Some(r.clone());
                self.last_limits_failed = false;
            }
            CodexEvent::LimitsFailed => self.last_limits_failed = true,
            CodexEvent::Usage(u) => {
                self.usage_seen = true;
                self.summary = Some(u.summary.clone());
            }
            CodexEvent::UsageFailed => {}
            CodexEvent::Activity { at, until } => {
                self.last_rise_at = Some(*at);
                self.active_until = Some(self.active_until.map_or(*until, |u| u.max(*until)));
            }
        }
    }

    fn limit_window(&self, kind: WindowKind, label: String, w: Option<&RateWindow>, now: i64) -> LimitWindow {
        let Some(reading) = &self.limits else {
            return LimitWindow {
                kind,
                label,
                percent: None,
                resets_at: None,
                fetched_at: None,
                state: WindowState::Unknown,
            };
        };
        let Some(w) = w else {
            return LimitWindow {
                kind,
                label,
                percent: None,
                resets_at: None,
                fetched_at: Some(reading.at),
                state: WindowState::Unknown,
            };
        };
        let fresh = now - reading.at <= FRESH_SECS;
        let (state, percent) = if fresh {
            (WindowState::Known, Some(w.used_percent))
        } else if w.resets_at.is_some_and(|r| now >= r) {
            (WindowState::AfterReset, None)
        } else {
            (WindowState::Stale, Some(w.used_percent))
        };
        LimitWindow {
            kind,
            label,
            percent,
            resets_at: w.resets_at,
            fetched_at: Some(reading.at),
            state,
        }
    }

    pub(crate) fn provider_limits(&self, now: i64) -> ProviderLimits {
        if self.disabled {
            return ProviderLimits {
                provider: Provider::Codex,
                state: LimitsState::Off,
                governing: None,
                windows: Vec::new(),
                note: Some(NOTE_DISABLED.into()),
            };
        }
        let r = self.limits.as_ref();
        let mut windows = vec![
            self.limit_window(WindowKind::FiveHour, "5h".into(), r.and_then(|r| r.primary.as_ref()), now),
            self.limit_window(WindowKind::Week, "týden".into(), r.and_then(|r| r.secondary.as_ref()), now),
        ];
        if let Some(r) = r {
            for (label, kind, w) in &r.extra {
                windows.push(self.limit_window(*kind, label.clone(), Some(w), now));
            }
        }
        let (state, governing) = governing(&windows);
        let note = match self.status {
            Status::NotFound => Some(NOTE_NOT_FOUND.into()),
            Status::Down => Some(NOTE_DOWN.into()),
            _ if self.last_limits_failed => Some(NOTE_READ_FAILED.into()),
            _ => None,
        };
        ProviderLimits {
            provider: Provider::Codex,
            state,
            governing,
            windows,
            note,
        }
    }

    pub(crate) fn activity(&self, now: i64) -> Activity {
        if self.disabled || self.status != Status::Running {
            return Activity::NoSignal;
        }
        if self.active_until.is_some_and(|u| now < u) {
            return Activity::Active;
        }
        // Quiet time only counts while we could observe: a restart starts over.
        match self.last_rise_at.max(self.running_since) {
            Some(since) if now - since >= SLEEP_AFTER => Activity::Asleep,
            _ => Activity::Idle,
        }
    }
}

/// Governing state and value over the 5h and week windows (design
/// "Normalizovaný limit"): value = max percent of windows that have one;
/// Unknown when none has one, Stale when the max window is Stale.
pub(crate) fn governing(windows: &[LimitWindow]) -> (LimitsState, Option<f64>) {
    let best = windows
        .iter()
        .filter(|w| matches!(w.kind, WindowKind::FiveHour | WindowKind::Week))
        .filter_map(|w| w.percent.map(|p| (p, w.state)))
        .max_by(|a, b| a.0.total_cmp(&b.0));
    match best {
        None => (LimitsState::Unknown, None),
        Some((p, WindowState::Stale)) => (LimitsState::Stale, Some(p)),
        Some((p, _)) => (LimitsState::Known, Some(p)),
    }
}

// ---------------------------------------------------------------- process

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Req {
    Initialize,
    Limits,
    Usage,
}

struct Proc {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<Value>>,
    pending: HashMap<u64, (Req, Instant)>,
    initialized: bool,
    #[cfg(windows)]
    _job: Option<win::Job>,
}

impl Proc {
    fn send(&mut self, msg: &Value) -> bool {
        let Some(stdin) = self.stdin.as_mut() else { return false };
        let mut line = msg.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes()).is_ok() && stdin.flush().is_ok()
    }

    fn teardown(mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Dropping the job handle kills anything the CLI spawned.
    }
}

fn read_lines(stdout: ChildStdout, tx: Sender<Option<Value>>) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => {
                let _ = tx.send(None);
                return;
            }
            Ok(_) => {
                // Non-JSON output (banners, warnings) is ignored.
                if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                    if tx.send(Some(v)).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

fn spawn_process(program: &str, args: &[String]) -> std::io::Result<Proc> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Suspended until it sits in the job, so nothing it spawns escapes.
        cmd.creation_flags(win::CREATE_NO_WINDOW | win::CREATE_SUSPENDED);
    }
    let mut child = cmd.spawn()?;
    #[cfg(windows)]
    let job = {
        let job = win::Job::kill_on_close();
        let assigned = job.as_ref().is_some_and(|j| j.assign(&child));
        if !win::resume_threads(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("could not resume codex process"));
        }
        if assigned {
            job
        } else {
            None
        }
    };
    let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
    let stdin = child.stdin.take();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("usage-codex-reader".into())
        .spawn(move || read_lines(stdout, tx))?;
    Ok(Proc {
        child,
        stdin,
        lines: rx,
        pending: HashMap::new(),
        initialized: false,
        #[cfg(windows)]
        _job: job,
    })
}

pub(crate) enum CodexCtl {
    ReadUsageNow,
    /// "Obnovit" in the widget: read limits and usage as soon as the manual
    /// gap allows, and retry a dead app-server right away.
    RefreshNow,
}

/// Manual refreshes never read the same thing more often than this.
const MANUAL_GAP: i64 = 10;

pub(crate) struct CodexThread {
    pub program: String,
    pub args: Vec<String>,
    pub core: Sender<CoreMsg>,
    pub ctl: Receiver<CodexCtl>,
    pub stop: Arc<AtomicBool>,
    pub clock: Clock,
    pub diag: Diag,
}

struct Sched {
    next_id: u64,
    next_spawn_at: i64,
    spawn_backoff: i64,
    ever_initialized: bool,
    next_limits_at: i64,
    last_limits_sent: Option<i64>,
    limits_errors: u32,
    next_usage_at: i64,
    last_usage_sent: Option<i64>,
    usage_errors: u32,
    prev_primary: Option<f64>,
    prev_buckets: Option<HashMap<String, u64>>,
}

fn backoff(base: i64, errors: u32) -> i64 {
    base.saturating_mul(1i64 << errors.min(10)).min(MAX_BACKOFF)
}

impl CodexThread {
    pub(crate) fn spawn(self) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("usage-codex".into())
            .spawn(move || self.run())
            .expect("spawn codex thread")
    }

    fn emit(&self, ev: CodexEvent) -> bool {
        self.core.send(CoreMsg::Update(Update::Codex(ev))).is_ok()
    }

    fn run(self) {
        let now = (self.clock)();
        let mut s = Sched {
            next_id: 1,
            next_spawn_at: now,
            spawn_backoff: SPAWN_BACKOFF_START,
            ever_initialized: false,
            next_limits_at: now,
            last_limits_sent: None,
            limits_errors: 0,
            next_usage_at: now,
            last_usage_sent: None,
            usage_errors: 0,
            prev_primary: None,
            prev_buckets: None,
        };
        let mut proc: Option<Proc> = None;

        loop {
            match self.ctl.recv_timeout(TICK) {
                Ok(CodexCtl::ReadUsageNow) => {
                    let now = (self.clock)();
                    s.prev_buckets = None;
                    s.next_usage_at = s.last_usage_sent.map_or(now, |t| now.max(t + MIN_GAP));
                }
                Ok(CodexCtl::RefreshNow) => {
                    let now = (self.clock)();
                    s.next_limits_at = s.last_limits_sent.map_or(now, |t| now.max(t + MANUAL_GAP));
                    s.next_usage_at = s.last_usage_sent.map_or(now, |t| now.max(t + MANUAL_GAP));
                    if proc.is_none() {
                        s.next_spawn_at = now;
                        s.spawn_backoff = SPAWN_BACKOFF_START;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            let now = (self.clock)();

            let Some(p) = proc.as_mut() else {
                if now >= s.next_spawn_at {
                    match spawn_process(&self.program, &self.args) {
                        Ok(mut p) => {
                            let id = s.next_id;
                            s.next_id += 1;
                            let init = json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "method": "initialize",
                                "params": {"clientInfo": {"name": "skylimit", "version": env!("CARGO_PKG_VERSION")}}
                            });
                            if p.send(&init) {
                                p.pending.insert(id, (Req::Initialize, Instant::now()));
                            }
                            self.diag.log(now, "codex", &format!("app-server started pid={}", p.child.id()));
                            proc = Some(p);
                        }
                        Err(e) => {
                            self.diag.log(now, "codex", &format!("spawn failed: {:?}", e.kind()));
                            let ev = if s.ever_initialized { CodexEvent::Down } else { CodexEvent::NotFound };
                            if !self.emit(ev) {
                                break;
                            }
                            s.next_spawn_at = now + s.spawn_backoff;
                            s.spawn_backoff = (s.spawn_backoff * 2).min(MAX_BACKOFF);
                        }
                    }
                }
                continue;
            };

            let mut dead = None;
            loop {
                match p.lines.try_recv() {
                    Ok(Some(msg)) => {
                        if !self.handle_message(p, &mut s, msg, now) {
                            dead = Some("initialize failed");
                        }
                    }
                    Ok(None) | Err(TryRecvError::Disconnected) => {
                        dead = Some("stdout closed");
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            // EOF alone is not enough: another process holding an inherited
            // pipe handle would keep stdout open after the server is gone.
            if dead.is_none() && matches!(p.child.try_wait(), Ok(Some(_))) {
                dead = Some("process exited");
            }
            if dead.is_none() && p.pending.values().any(|(_, t)| t.elapsed() > REQUEST_TIMEOUT) {
                dead = Some("request timed out");
            }

            if dead.is_none() && p.initialized {
                if now >= s.next_limits_at && !p.pending.values().any(|(r, _)| *r == Req::Limits) {
                    let id = s.next_id;
                    s.next_id += 1;
                    if p.send(&json!({"jsonrpc": "2.0", "id": id, "method": "account/rateLimits/read", "params": null})) {
                        p.pending.insert(id, (Req::Limits, Instant::now()));
                        s.last_limits_sent = Some(now);
                        s.next_limits_at = now + LIMITS_EVERY;
                    } else {
                        dead = Some("stdin closed");
                    }
                }
                if now >= s.next_usage_at && !p.pending.values().any(|(r, _)| *r == Req::Usage) {
                    let id = s.next_id;
                    s.next_id += 1;
                    if p.send(&json!({"jsonrpc": "2.0", "id": id, "method": "account/usage/read", "params": null})) {
                        p.pending.insert(id, (Req::Usage, Instant::now()));
                        s.last_usage_sent = Some(now);
                        s.next_usage_at = now + USAGE_EVERY;
                    } else {
                        dead = Some("stdin closed");
                    }
                }
            }

            if let Some(reason) = dead {
                let p = proc.take().expect("process present");
                let was_initialized = p.initialized;
                let pid = p.child.id();
                p.teardown();
                let ev = if s.ever_initialized {
                    CodexEvent::Down
                } else {
                    CodexEvent::NotFound
                };
                self.diag.log(
                    now,
                    "codex",
                    &format!(
                        "app-server pid={pid} gone ({reason}, initialized={was_initialized}), restart in {} s",
                        s.spawn_backoff
                    ),
                );
                if !self.emit(ev) {
                    break;
                }
                s.next_spawn_at = now + s.spawn_backoff;
                s.spawn_backoff = (s.spawn_backoff * 2).min(MAX_BACKOFF);
                s.prev_primary = None;
                s.prev_buckets = None;
            }
        }
        if let Some(p) = proc.take() {
            p.teardown();
        }
    }

    /// Returns false when the process should be restarted.
    fn handle_message(&self, p: &mut Proc, s: &mut Sched, msg: Value, now: i64) -> bool {
        let id = msg.get("id").and_then(Value::as_u64);
        let method = msg.get("method").and_then(Value::as_str);
        match (id, method) {
            // Request from the server (approvals etc.): not supported.
            (Some(id), Some(_)) => {
                p.send(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not supported"}}));
                true
            }
            (None, Some(method)) => {
                if method == "account/rateLimits/updated" {
                    s.next_limits_at = s.last_limits_sent.map_or(now, |t| now.max(t + MIN_GAP));
                }
                // Anything else (remoteControl/status/changed, account/updated, …) is ignored.
                true
            }
            (Some(id), None) => {
                let Some((req, _)) = p.pending.remove(&id) else { return true };
                let result = msg.get("result");
                if let Some(err) = msg.get("error") {
                    let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
                    self.diag.log(now, "codex", &format!("{req:?} failed: rpc error code={code}"));
                }
                self.handle_response(p, s, req, result, now)
            }
            (None, None) => true,
        }
    }

    fn handle_response(&self, p: &mut Proc, s: &mut Sched, req: Req, result: Option<&Value>, now: i64) -> bool {
        match req {
            Req::Initialize => {
                if result.is_none() {
                    return false;
                }
                p.send(&json!({"jsonrpc": "2.0", "method": "initialized"}));
                p.initialized = true;
                s.ever_initialized = true;
                s.next_limits_at = s.last_limits_sent.map_or(now, |t| now.max(t + MIN_GAP));
                s.next_usage_at = s.last_usage_sent.map_or(now, |t| now.max(t + MIN_GAP));
                self.emit(CodexEvent::Running { at: now });
                true
            }
            Req::Limits => {
                match result.and_then(|r| parse_rate_limits(r, now)) {
                    Some(reading) => {
                        s.limits_errors = 0;
                        s.spawn_backoff = SPAWN_BACKOFF_START;
                        let used = reading.primary.as_ref().map(|w| w.used_percent);
                        if let (Some(prev), Some(cur)) = (s.prev_primary, used) {
                            if cur > prev + 1e-9 {
                                self.emit(CodexEvent::Activity {
                                    at: now,
                                    until: s.next_limits_at + ACTIVITY_GRACE,
                                });
                            }
                        }
                        s.prev_primary = used;
                        self.emit(CodexEvent::Limits(reading));
                    }
                    None => {
                        if let Some(r) = result {
                            self.diag.log(now, "codex", &format!("unknown rateLimits shape: {}", json_shape(r)));
                        }
                        s.next_limits_at = now + backoff(LIMITS_EVERY, s.limits_errors);
                        s.limits_errors += 1;
                        self.emit(CodexEvent::LimitsFailed);
                    }
                }
                true
            }
            Req::Usage => {
                match result.and_then(|r| parse_usage(r, now)) {
                    Some(reading) => {
                        s.usage_errors = 0;
                        let current: HashMap<String, u64> = reading.buckets.iter().cloned().collect();
                        if let Some(prev) = &s.prev_buckets {
                            let rose = current
                                .iter()
                                .any(|(d, t)| *t > prev.get(d).copied().unwrap_or(0));
                            if rose {
                                self.emit(CodexEvent::Activity {
                                    at: now,
                                    until: s.next_usage_at + ACTIVITY_GRACE,
                                });
                            }
                        }
                        s.prev_buckets = Some(current);
                        self.emit(CodexEvent::Usage(reading));
                    }
                    None => {
                        if let Some(r) = result {
                            self.diag.log(now, "codex", &format!("unknown usage shape: {}", json_shape(r)));
                        }
                        s.next_usage_at = now + backoff(USAGE_EVERY, s.usage_errors);
                        s.usage_errors += 1;
                        self.emit(CodexEvent::UsageFailed);
                    }
                }
                true
            }
        }
    }
}

// ---------------------------------------------------------------- Windows job

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    pub const CREATE_SUSPENDED: u32 = 0x0000_0004;

    /// Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when the handle
    /// closes (normal drop or our process crashing) every process in it dies,
    /// so no orphaned app-server keeps refreshing the Codex login.
    pub struct Job(HANDLE);

    // The handle is only used from the owning thread; moving it is fine.
    unsafe impl Send for Job {}

    impl Job {
        pub fn kill_on_close() -> Option<Job> {
            // SAFETY: plain Win32 calls with valid, zero-initialised arguments.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const c_void,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    CloseHandle(h);
                    return None;
                }
                Some(Job(h))
            }
        }

        pub fn assign(&self, child: &Child) -> bool {
            // SAFETY: both handles are valid for the duration of the call.
            unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) != 0 }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: we own the handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Resume every thread of a process started with `CREATE_SUSPENDED`.
    pub fn resume_threads(pid: u32) -> bool {
        // SAFETY: Toolhelp snapshot walk with a correctly sized entry.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap == INVALID_HANDLE_VALUE || snap.is_null() {
                return false;
            }
            let mut entry: THREADENTRY32 = zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut resumed = false;
            if Thread32First(snap, &mut entry) != 0 {
                loop {
                    if entry.th32OwnerProcessID == pid {
                        let th = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                        if !th.is_null() {
                            if ResumeThread(th) != u32::MAX {
                                resumed = true;
                            }
                            CloseHandle(th);
                        }
                    }
                    if Thread32Next(snap, &mut entry) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
            resumed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> Value {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex").join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn parses_live_rate_limits_by_position() {
        let v = fixture("rate-limits.json");
        let r = parse_rate_limits(&v["result"], 100).unwrap();
        let p = r.primary.unwrap();
        let s = r.secondary.unwrap();
        assert_eq!(p.window_mins, Some(300));
        assert_eq!(s.window_mins, Some(10080));
        assert_eq!(p.resets_at, Some(1_793_202_776));
        assert_eq!(p.used_percent, 0.0);
        assert!(r.extra.is_empty(), "the `codex` entry duplicates rateLimits");
    }

    #[test]
    fn parses_live_usage() {
        let v = fixture("usage-account.json");
        let u = parse_usage(&v["result"], 100).unwrap();
        assert_eq!(u.summary.current_streak_days, Some(2));
        assert_eq!(u.summary.longest_streak_days, Some(9));
        assert_eq!(u.buckets.first().unwrap(), &("2026-04-16".to_string(), 15_083_263));
        assert_eq!(u.buckets.last().unwrap(), &("2026-10-27".to_string(), 27_267));
        // Per-thread usage is null on this plan: still a known shape, no buckets.
        let t = fixture("usage-thread.json");
        let tu = parse_usage(&t["result"], 100).unwrap();
        assert!(tu.buckets.is_empty());
    }

    #[test]
    fn unknown_shapes_are_rejected_not_panicking() {
        assert!(parse_rate_limits(&json!({"limits": [1, 2]}), 0).is_none());
        assert!(parse_rate_limits(&json!(null), 0).is_none());
        assert!(parse_rate_limits(&json!({"rateLimits": "x"}), 0).is_none());
        assert!(parse_usage(&json!({"foo": 1}), 0).is_none());
        // Known envelope, odd window → that window is simply missing.
        let r = parse_rate_limits(&json!({"rateLimits": {"primary": {"usedPercent": "lots"}, "secondary": null}}), 0)
            .unwrap();
        assert!(r.primary.is_none() && r.secondary.is_none());
    }

    fn reading(at: i64, p5h: f64, reset5h: i64, pweek: f64, reset_week: i64) -> LimitsReading {
        LimitsReading {
            at,
            primary: Some(RateWindow {
                used_percent: p5h,
                window_mins: Some(300),
                resets_at: Some(reset5h),
            }),
            secondary: Some(RateWindow {
                used_percent: pweek,
                window_mins: Some(10080),
                resets_at: Some(reset_week),
            }),
            extra: Vec::new(),
        }
    }

    #[test]
    fn window_states_known_stale_after_reset() {
        let mut v = CodexView::new(false);
        let unknown = v.provider_limits(0);
        assert_eq!(unknown.state, LimitsState::Unknown);
        assert_eq!(unknown.windows.len(), 2);
        assert!(unknown.windows.iter().all(|w| w.state == WindowState::Unknown));

        v.apply(&CodexEvent::Limits(reading(1000, 40.0, 5000, 70.0, 90_000)));
        let l = v.provider_limits(1000 + 600);
        assert_eq!(l.state, LimitsState::Known);
        assert_eq!(l.governing, Some(70.0));
        assert_eq!(l.windows[0].label, "5h");
        assert_eq!(l.windows[1].label, "týden");

        // 11 minutes later: Stale, value kept.
        let l = v.provider_limits(1000 + 660);
        assert_eq!(l.state, LimitsState::Stale);
        assert_eq!(l.windows[0].state, WindowState::Stale);
        assert_eq!(l.governing, Some(70.0));

        // 5h reset passed while Stale: no percent, does not govern.
        let l = v.provider_limits(5000);
        assert_eq!(l.windows[0].state, WindowState::AfterReset);
        assert_eq!(l.windows[0].percent, None);
        assert_eq!(l.windows[1].state, WindowState::Stale);
        assert_eq!(l.governing, Some(70.0));

        // Both reset: Unknown.
        let l = v.provider_limits(90_000);
        assert_eq!(l.state, LimitsState::Unknown);
        assert_eq!(l.governing, None);
    }

    #[test]
    fn mixed_known_and_stale_is_stale_with_max() {
        // design example: 5h Known 40 %, week Stale 70 % → Stale 70 %
        let ws = vec![
            LimitWindow {
                kind: WindowKind::FiveHour,
                label: "5h".into(),
                percent: Some(40.0),
                resets_at: None,
                fetched_at: None,
                state: WindowState::Known,
            },
            LimitWindow {
                kind: WindowKind::Week,
                label: "týden".into(),
                percent: Some(70.0),
                resets_at: None,
                fetched_at: None,
                state: WindowState::Stale,
            },
            LimitWindow {
                kind: WindowKind::WeekModel,
                label: "týden · x".into(),
                percent: Some(99.0),
                resets_at: None,
                fetched_at: None,
                state: WindowState::Known,
            },
        ];
        assert_eq!(governing(&ws), (LimitsState::Stale, Some(70.0)));
    }

    #[test]
    fn notes_and_activity() {
        let mut v = CodexView::new(false);
        v.apply(&CodexEvent::NotFound);
        assert_eq!(v.provider_limits(0).note.as_deref(), Some(NOTE_NOT_FOUND));
        assert_eq!(v.activity(0), Activity::NoSignal);
        v.apply(&CodexEvent::Running { at: 100 });
        assert_eq!(v.activity(100), Activity::Idle);
        v.apply(&CodexEvent::Activity { at: 200, until: 200 + 420 });
        assert_eq!(v.activity(500), Activity::Active);
        assert_eq!(v.activity(700), Activity::Idle);
        assert_eq!(v.activity(200 + 1800), Activity::Asleep);
        v.apply(&CodexEvent::Down);
        assert_eq!(v.provider_limits(0).note.as_deref(), Some(NOTE_DOWN));
        assert_eq!(v.activity(5000), Activity::NoSignal);
        let off = CodexView::new(true).provider_limits(0);
        assert_eq!(off.state, LimitsState::Off);
        assert!(off.windows.is_empty());
    }

    #[test]
    /// The suspended start + Job Object + resume path really runs the child,
    /// and an exit is seen both as EOF and by `try_wait`.
    fn suspended_spawn_in_job_runs_and_exits() {
        let dir = tempfile::tempdir().unwrap();
        let ctl = dir.path().join("c.json");
        std::fs::write(&ctl, r#"{"die": true}"#).unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let args: Vec<String> = vec![
            root.join("tests/fake-codex/server.mjs").to_string_lossy().into(),
            "--fixtures".into(),
            root.join("tests/fixtures/codex").to_string_lossy().into(),
            "--control".into(),
            ctl.to_string_lossy().into(),
        ];
        // Round 0: a healthy server answers initialize, then is told to die.
        std::fs::write(&ctl, "{}").unwrap();
        let mut p = spawn_process("node", &args).unwrap();
        assert!(p.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})));
        let msg = p.lines.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(msg["id"], 1);
        std::fs::write(&ctl, r#"{"die": true}"#).unwrap();
        // Drain until EOF (the notifications after initialize, then exit).
        while p.lines.recv_timeout(Duration::from_secs(5)).unwrap().is_some() {}
        p.teardown();
        for round in 1..4 {
            let mut p = spawn_process("node", &args).unwrap();
            assert!(p.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})));
            let started = Instant::now();
            loop {
                if let Ok(Some(_)) = p.child.try_wait() {
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(5), "round {round}: node did not exit");
                std::thread::sleep(Duration::from_millis(20));
            }
            p.teardown();
        }
    }

    #[test]
    fn backoff_caps_at_fifteen_minutes() {
        assert_eq!(backoff(120, 0), 120);
        assert_eq!(backoff(120, 1), 240);
        assert_eq!(backoff(120, 5), 900);
        assert_eq!(backoff(300, 30), 900);
    }
}
