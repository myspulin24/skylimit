//! Claude limits from T3 Code's provider logs. Read-only and credential-free:
//! T3 Code runs Claude through the official Claude Agent SDK and writes every
//! decoded SDK message to `~/.t3/userdata/logs/provider/events.<thread>.log`,
//! one per line (`[<ISO time>] <TAG>: {json}`). The SDK's `rate_limit_event`
//! carries the account's unified 5h and 7-day utilization; this module tails
//! those files and keeps the newest one. Nothing here reads Claude login data
//! or calls Anthropic (terms gate in the design doc).
//!
//! Privacy: the logs hold whole conversations. Only lines that contain the
//! literal `rate_limit_event` are parsed, only numbers leave this module, and
//! nothing from a line ever reaches the diagnostic log.
//!
//! ```text
//!  every 10 s ─▶ events.*.log* changed ≤ 8 days ago ─▶ appended bytes
//!                (first sight: last 4 MB, from the next line start)
//!            ─▶ complete lines containing "rate_limit_event"
//!            ─▶ JSON object {type:"rate_limit_event", rate_limit_info}
//!            ─▶ newest by line time ─▶ Update::ClaudeLimits
//! ```
//!
//! Window states (the reading is passive, so time alone never ages it):
//! Known until Claude work shows up in the JSONL more than 15 min after the
//! reading (work outside T3 Code, the value may be higher) → Stale. Once
//! `resetsAt` passes: Known 0 % when there was no Claude work since the reset,
//! AfterReset otherwise.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::Value;

use crate::codex::governing;
use crate::diag::Diag;
use crate::model::{LimitWindow, LimitsState, Provider, ProviderLimits, WindowKind, WindowState};
use crate::{Clock, CoreMsg, Update};

const NEEDLE: &[u8] = b"rate_limit_event";
const POLL: Duration = Duration::from_secs(10);
const STEP: Duration = Duration::from_millis(250);
/// First look at a file reads at most its last this many bytes.
const FIRST_READ_MAX: u64 = 4 << 20;
/// Files untouched for longer cannot hold a reading of the current week.
const MAX_FILE_AGE: i64 = 8 * 86_400;
/// Claude work this long after the reading means it happened outside T3 Code
/// (T3 Code logs a new event every few minutes while it works).
pub(crate) const STALE_AFTER_WORK: i64 = 15 * 60;
const WEEK: i64 = 7 * 86_400;

pub(crate) const NOTE_NO_T3: &str = "limity Claude jen přes T3 Code (nenalezen)";
pub(crate) const NOTE_WAITING: &str = "limity Claude se ukážou po další zprávě v T3 Code";
pub(crate) const NOTE_STALE: &str = "Claude od té doby běžel mimo T3 Code";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaudeWindow {
    /// 0..100
    pub percent: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaudeReading {
    /// Line time of the event (epoch s).
    pub at: i64,
    pub five_hour: Option<ClaudeWindow>,
    pub seven_day: Option<ClaudeWindow>,
}

// ---------------------------------------------------------------- parsing

/// A reading from one log line, or `None` when the line is anything else.
pub(crate) fn parse_line(line: &[u8], fallback_at: i64) -> Option<ClaudeReading> {
    memchr::memmem::find(line, NEEDLE)?;
    let text = std::str::from_utf8(line).ok()?.trim_end();
    let json = &text[text.find('{')?..];
    let value: Value = serde_json::from_str(json).ok()?;
    if let Some(provider) = value.get("provider").and_then(Value::as_str) {
        if !provider.to_ascii_lowercase().contains("claude") {
            return None;
        }
    }
    let info = find_info(&value, 0)?;
    reading_from_info(info, line_time(text).unwrap_or(fallback_at))
}

fn line_time(text: &str) -> Option<i64> {
    let rest = text.strip_prefix('[')?;
    let stamp = &rest[..rest.find(']')?];
    chrono::DateTime::parse_from_rfc3339(stamp).ok().map(|d| d.timestamp())
}

/// The SDK message, wherever T3 Code nests it. Strings are never parsed, so
/// conversation text that merely mentions the event cannot match.
fn find_info(v: &Value, depth: u8) -> Option<&Value> {
    if depth > 8 {
        return None;
    }
    match v {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("rate_limit_event") {
                if let Some(info) = map.get("rate_limit_info").filter(|i| i.is_object()) {
                    return Some(info);
                }
            }
            map.values().find_map(|c| find_info(c, depth + 1))
        }
        Value::Array(items) => items.iter().find_map(|c| find_info(c, depth + 1)),
        _ => None,
    }
}

fn reading_from_info(info: &Value, at: i64) -> Option<ClaudeReading> {
    let mut r = ClaudeReading {
        at,
        five_hour: None,
        seven_day: None,
    };
    if let Some(windows) = info.get("unifiedWindows").and_then(Value::as_object) {
        r.five_hour = windows.get("five_hour").and_then(window);
        r.seven_day = windows.get("seven_day").and_then(window);
    }
    // Without `unifiedWindows` an event describes the one window it names.
    if r.five_hour.is_none() && r.seven_day.is_none() {
        match info.get("rateLimitType").and_then(Value::as_str) {
            Some("five_hour") => r.five_hour = window(info),
            Some("seven_day") => r.seven_day = window(info),
            _ => {}
        }
    }
    (r.five_hour.is_some() || r.seven_day.is_some()).then_some(r)
}

/// `utilization` is a fraction (0.05 = 5 %, the same number Claude shows in
/// its usage settings). A rejected window without one is full.
fn window(v: &Value) -> Option<ClaudeWindow> {
    let resets_at = v
        .get("resetsAt")
        .and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)));
    let utilization = v.get("utilization").and_then(Value::as_f64).filter(|u| u.is_finite());
    let percent = match (utilization, v.get("status").and_then(Value::as_str)) {
        (Some(u), _) => ((u * 1000.0).round() / 10.0).clamp(0.0, 100.0),
        (None, Some("rejected")) => 100.0,
        _ => return None,
    };
    Some(ClaudeWindow { percent, resets_at })
}

// ---------------------------------------------------------------- tailing

/// Read positions per log file; each poll reads only what was appended.
#[derive(Default)]
pub(crate) struct Tail {
    offsets: HashMap<PathBuf, u64>,
}

impl Tail {
    /// Newest reading in the bytes appended since the previous poll.
    pub(crate) fn poll(&mut self, dir: &Path, now: i64, diag: &Diag) -> Option<ClaudeReading> {
        let entries = fs::read_dir(dir).ok()?;
        let mut best: Option<ClaudeReading> = None;
        let mut seen = HashSet::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("events.") || !name.contains(".log") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let path = entry.path();
            seen.insert(path.clone());
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(now, |d| d.as_secs() as i64);
            let len = meta.len();
            let resume = self.offsets.get(&path).copied().filter(|off| *off <= len);
            if resume.is_none() && now - mtime > MAX_FILE_AGE {
                continue;
            }
            let start = resume.unwrap_or_else(|| len.saturating_sub(FIRST_READ_MAX));
            if start == len {
                self.offsets.insert(path, len);
                continue;
            }
            // Mid-file first reads begin inside a line: skip to the next one.
            let skip_partial = resume.is_none() && start > 0;
            match read_complete_lines(&path, start, len, skip_partial, mtime) {
                Ok(Some((end, reading))) => {
                    self.offsets.insert(path, end);
                    if let Some(r) = reading {
                        if best.as_ref().is_none_or(|b| r.at >= b.at) {
                            best = Some(r);
                        }
                    }
                }
                // No complete line yet; look again next time.
                Ok(None) => {}
                Err(e) => diag.log(now, "t3", &format!("T3 Code log not readable: {:?}", e.kind())),
            }
        }
        self.offsets.retain(|p, _| seen.contains(p));
        best
    }
}

/// Reads `start..len`, parses complete lines and returns the offset after the
/// last complete line with the newest reading among them. `None` when the
/// range holds no complete line.
fn read_complete_lines(
    path: &Path,
    start: u64,
    len: u64,
    skip_partial: bool,
    mtime: i64,
) -> std::io::Result<Option<(u64, Option<ClaudeReading>)>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.take(len - start).read_to_end(&mut buf)?;

    let Some(last_nl) = memchr::memrchr(b'\n', &buf) else {
        return Ok(None);
    };
    let mut body = &buf[..last_nl];
    if skip_partial {
        match memchr::memchr(b'\n', &buf) {
            Some(first_nl) if first_nl < last_nl => body = &buf[first_nl + 1..last_nl],
            _ => return Ok(Some((start + last_nl as u64 + 1, None))),
        }
    }
    let mut best: Option<ClaudeReading> = None;
    for line in body.split(|b| *b == b'\n') {
        if let Some(r) = parse_line(line, mtime) {
            if best.as_ref().is_none_or(|b| r.at >= b.at) {
                best = Some(r);
            }
        }
    }
    Ok(Some((start + last_nl as u64 + 1, best)))
}

pub(crate) struct T3Thread {
    pub dir: PathBuf,
    pub core: Sender<CoreMsg>,
    /// Any message polls right away (manual refresh).
    pub wake: Receiver<()>,
    pub stop: Arc<AtomicBool>,
    pub clock: Clock,
    pub diag: Diag,
}

impl T3Thread {
    pub(crate) fn spawn(self) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("usage-t3".into())
            .spawn(move || self.run())
            .expect("spawn T3 Code log reader")
    }

    fn run(self) {
        let mut tail = Tail::default();
        let mut sent_at: Option<i64> = None;
        let mut sent_dir: Option<bool> = None;
        loop {
            let now = (self.clock)();
            let dir_exists = self.dir.is_dir();
            let reading = if dir_exists {
                tail.poll(&self.dir, now, &self.diag)
            } else {
                None
            };
            let reading = reading.filter(|r| sent_at.is_none_or(|t| r.at > t));
            if reading.is_some() || sent_dir != Some(dir_exists) {
                if let Some(r) = &reading {
                    sent_at = Some(r.at);
                }
                sent_dir = Some(dir_exists);
                if self
                    .core
                    .send(CoreMsg::Update(Update::ClaudeLimits { dir_exists, reading }))
                    .is_err()
                {
                    return;
                }
            }
            let mut waited = Duration::ZERO;
            while waited < POLL {
                if self.stop.load(Ordering::Relaxed) {
                    return;
                }
                match self.wake.recv_timeout(STEP) {
                    Ok(()) => break,
                    Err(RecvTimeoutError::Timeout) => waited += STEP,
                    // The core is gone; keep honouring `stop` with plain sleeps.
                    Err(RecvTimeoutError::Disconnected) => {
                        std::thread::sleep(STEP);
                        waited += STEP;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- view

/// What the core knows about Claude's limits.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClaudeLimitsView {
    /// A T3 Code log folder is configured at all.
    pub configured: bool,
    pub dir_exists: bool,
    pub reading: Option<ClaudeReading>,
}

impl ClaudeLimitsView {
    /// `last_work`: newest Claude assistant line from the JSONL, any source.
    pub(crate) fn provider_limits(&self, now: i64, last_work: Option<i64>) -> ProviderLimits {
        let Some(r) = &self.reading else {
            let waiting = self.configured && self.dir_exists;
            return ProviderLimits {
                provider: Provider::Claude,
                state: if waiting { LimitsState::Unknown } else { LimitsState::Off },
                governing: None,
                windows: if waiting {
                    vec![unknown(WindowKind::FiveHour, "5h"), unknown(WindowKind::Week, "týden")]
                } else {
                    Vec::new()
                },
                note: Some(if waiting { NOTE_WAITING } else { NOTE_NO_T3 }.into()),
            };
        };
        let windows = vec![
            claude_window(WindowKind::FiveHour, "5h", r.five_hour.as_ref(), r.at, now, last_work),
            claude_window(WindowKind::Week, "týden", r.seven_day.as_ref(), r.at, now, last_work),
        ];
        let (state, governing) = governing(&windows);
        ProviderLimits {
            provider: Provider::Claude,
            state,
            governing,
            note: match state {
                LimitsState::Stale => Some(NOTE_STALE.into()),
                LimitsState::Unknown => Some(NOTE_WAITING.into()),
                _ => None,
            },
            windows,
        }
    }
}

fn unknown(kind: WindowKind, label: &str) -> LimitWindow {
    LimitWindow {
        kind,
        label: label.into(),
        percent: None,
        resets_at: None,
        fetched_at: None,
        state: WindowState::Unknown,
    }
}

fn claude_window(
    kind: WindowKind,
    label: &str,
    w: Option<&ClaudeWindow>,
    at: i64,
    now: i64,
    last_work: Option<i64>,
) -> LimitWindow {
    let Some(w) = w else {
        return LimitWindow {
            fetched_at: Some(at),
            ..unknown(kind, label)
        };
    };
    if let Some(reset) = w.resets_at.filter(|r| now >= *r) {
        if last_work.is_none_or(|t| t < reset) {
            // Rolled over and nothing used since: empty. The weekly window
            // keeps its fixed schedule; the 5h one starts with the next message.
            let next = (kind == WindowKind::Week).then(|| reset + ((now - reset) / WEEK + 1) * WEEK);
            return LimitWindow {
                kind,
                label: label.into(),
                percent: Some(0.0),
                resets_at: next,
                fetched_at: Some(at),
                state: WindowState::Known,
            };
        }
        return LimitWindow {
            kind,
            label: label.into(),
            percent: None,
            resets_at: Some(reset),
            fetched_at: Some(at),
            state: WindowState::AfterReset,
        };
    }
    let stale = last_work.is_some_and(|t| t > at + STALE_AFTER_WORK);
    LimitWindow {
        kind,
        label: label.into(),
        percent: Some(w.percent),
        resets_at: w.resets_at,
        fetched_at: Some(at),
        state: if stale { WindowState::Stale } else { WindowState::Known },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const T: i64 = 1_793_200_000;

    fn event_line(stamp: &str, five: f64, week: f64, reset5: i64, reset7: i64) -> String {
        format!(
            r#"[{stamp}] NTIVE: {{"provider":"claudeAgent","protocol":"claude-agent-sdk.query","kind":"protocol","event":{{"direction":"incoming","stage":"decoded","payload":{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed","resetsAt":{reset5},"rateLimitType":"five_hour","unifiedWindows":{{"five_hour":{{"utilization":{five},"resetsAt":{reset5},"status":"allowed"}},"seven_day":{{"utilization":{week},"resetsAt":{reset7},"status":"allowed"}}}}}},"uuid":"u","session_id":"s"}}}}}}"#
        )
    }

    fn reading(at: i64, five: f64, reset5: i64, week: f64, reset7: i64) -> ClaudeReading {
        ClaudeReading {
            at,
            five_hour: Some(ClaudeWindow {
                percent: five,
                resets_at: Some(reset5),
            }),
            seven_day: Some(ClaudeWindow {
                percent: week,
                resets_at: Some(reset7),
            }),
        }
    }

    #[test]
    fn parses_unified_windows_from_a_t3_line() {
        let line = event_line("2026-10-28T16:37:12.803Z", 0.04, 0.05, 1_793_223_000, 1_793_725_200);
        let r = parse_line(line.as_bytes(), 0).unwrap();
        assert_eq!(r.at, 1_793_205_432);
        assert_eq!(r.five_hour.unwrap(), ClaudeWindow { percent: 4.0, resets_at: Some(1_793_223_000) });
        assert_eq!(r.seven_day.unwrap(), ClaudeWindow { percent: 5.0, resets_at: Some(1_793_725_200) });
    }

    #[test]
    fn single_window_events_and_rejected_status() {
        let line = r#"[2026-10-28T10:00:00Z] X: {"payload":{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1793223000,"rateLimitType":"seven_day"}}}"#;
        let r = parse_line(line.as_bytes(), 0).unwrap();
        assert!(r.five_hour.is_none());
        assert_eq!(r.seven_day.unwrap().percent, 100.0);
    }

    #[test]
    fn ignores_text_that_only_mentions_the_event() {
        // A tool call whose input quotes an event is a string, not an object.
        let quoted = r#"[2026-10-28T10:00:00Z] X: {"provider":"claudeAgent","payload":{"type":"assistant","text":"{\"type\":\"rate_limit_event\",\"rate_limit_info\":{\"utilization\":0.9}}"}}"#;
        assert!(parse_line(quoted.as_bytes(), 0).is_none());
        let codex = r#"[2026-10-28T10:00:00Z] X: {"provider":"codex","payload":{"type":"rate_limit_event","rate_limit_info":{"rateLimitType":"five_hour","utilization":0.5}}}"#;
        assert!(parse_line(codex.as_bytes(), 0).is_none());
        assert!(parse_line(b"[2026-10-28T10:00:00Z] X: not json rate_limit_event", 0).is_none());
        assert!(parse_line(b"no needle here", 0).is_none());
    }

    #[test]
    fn tail_reads_appends_skips_partial_lines_and_follows_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let diag = Diag::new(&dir.path().join("diag"));
        let file = dir.path().join("events.abc.log");
        let now = chrono::Utc::now().timestamp();
        let mut f = File::create(&file).unwrap();
        writeln!(f, "{}", event_line("2026-10-28T10:00:00Z", 0.10, 0.20, T + 100, T + 900)).unwrap();
        write!(f, "{}", event_line("2026-10-28T10:05:00Z", 0.30, 0.40, T + 100, T + 900)).unwrap(); // no newline yet
        drop(f);
        // Unrelated files in the folder are ignored.
        fs::write(dir.path().join("other.txt"), event_line("2026-10-28T11:00:00Z", 0.9, 0.9, 0, 0)).unwrap();

        let mut tail = Tail::default();
        let r = tail.poll(dir.path(), now, &diag).unwrap();
        assert_eq!(r.five_hour.unwrap().percent, 10.0, "the unterminated line waits");
        assert!(tail.poll(dir.path(), now, &diag).is_none(), "nothing new");

        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f).unwrap();
        drop(f);
        let r = tail.poll(dir.path(), now, &diag).unwrap();
        assert_eq!(r.five_hour.unwrap().percent, 30.0);

        // Rotation: a fresh, shorter file under the same name.
        fs::write(&file, format!("{}\n", event_line("2026-10-28T12:00:00Z", 0.01, 0.41, T + 200, T + 900))).unwrap();
        let r = tail.poll(dir.path(), now, &diag).unwrap();
        assert_eq!(r.five_hour.unwrap().percent, 1.0);
    }

    #[test]
    fn first_read_of_a_big_file_starts_at_a_line_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let diag = Diag::new(&dir.path().join("diag"));
        let file = dir.path().join("events.big.log");
        let mut f = File::create(&file).unwrap();
        let filler = format!("[2026-10-28T09:00:00Z] X: {{\"pad\":\"{}\"}}\n", "x".repeat(1 << 20));
        for _ in 0..5 {
            f.write_all(filler.as_bytes()).unwrap();
        }
        writeln!(f, "{}", event_line("2026-10-28T10:00:00Z", 0.25, 0.5, T + 100, T + 900)).unwrap();
        drop(f);
        let mut tail = Tail::default();
        let r = tail.poll(dir.path(), chrono::Utc::now().timestamp(), &diag).unwrap();
        assert_eq!(r.five_hour.unwrap().percent, 25.0);
    }

    fn view(r: ClaudeReading) -> ClaudeLimitsView {
        ClaudeLimitsView {
            configured: true,
            dir_exists: true,
            reading: Some(r),
        }
    }

    #[test]
    fn known_until_work_outside_t3_then_stale() {
        let v = view(reading(T, 42.0, T + 3600, 12.0, T + 86_400));
        let l = v.provider_limits(T + 600, Some(T + 60));
        assert_eq!(l.state, LimitsState::Known);
        assert_eq!(l.governing, Some(42.0));
        assert_eq!(l.note, None);
        // Hours later with no Claude work at all: still known (passive source).
        assert_eq!(v.provider_limits(T + 3000, Some(T)).state, LimitsState::Known);
        // Work 20 min after the reading that T3 Code did not report.
        let l = v.provider_limits(T + 1500, Some(T + 1200));
        assert_eq!(l.state, LimitsState::Stale);
        assert_eq!(l.windows[0].state, WindowState::Stale);
        assert_eq!(l.note.as_deref(), Some(NOTE_STALE));
    }

    #[test]
    fn after_reset_is_empty_without_work_and_unknown_with_it() {
        let v = view(reading(T, 80.0, T + 3600, 30.0, T + 86_400));
        let quiet = v.provider_limits(T + 4000, Some(T));
        assert_eq!(quiet.windows[0].state, WindowState::Known);
        assert_eq!(quiet.windows[0].percent, Some(0.0));
        assert_eq!(quiet.windows[0].resets_at, None, "5h starts with the next message");
        assert_eq!(quiet.governing, Some(30.0));

        let busy = v.provider_limits(T + 4000, Some(T + 3700));
        assert_eq!(busy.windows[0].state, WindowState::AfterReset);
        assert_eq!(busy.windows[0].percent, None);

        // The weekly window keeps its schedule.
        let later = v.provider_limits(T + 86_400 + 10, Some(T));
        assert_eq!(later.windows[1].percent, Some(0.0));
        assert_eq!(later.windows[1].resets_at, Some(T + 86_400 + WEEK));
    }

    #[test]
    fn no_source_is_off_and_waiting_is_unknown() {
        let none = ClaudeLimitsView::default().provider_limits(T, None);
        assert_eq!(none.state, LimitsState::Off);
        assert_eq!(none.note.as_deref(), Some(NOTE_NO_T3));
        let waiting = ClaudeLimitsView {
            configured: true,
            dir_exists: true,
            reading: None,
        }
        .provider_limits(T, None);
        assert_eq!(waiting.state, LimitsState::Unknown);
        assert_eq!(waiting.windows.len(), 2);
        assert_eq!(waiting.note.as_deref(), Some(NOTE_WAITING));
    }
}
