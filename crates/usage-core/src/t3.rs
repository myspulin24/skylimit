//! Claude limits from T3 Code's provider logs. Read-only and credential-free:
//! T3 Code runs Claude through the official Claude Agent SDK and writes every
//! decoded SDK message to `~/.t3/userdata/logs/provider/events.<thread>.log`,
//! one per line (`[<ISO time>] <TAG>: {json}`). The SDK's `rate_limit_event`
//! carries the account's unified 5h and 7-day utilization; this module tails
//! those files and keeps the newest one. Nothing here reads Claude login data
//! or calls Anthropic (terms gate in the design doc). The reading goes to
//! `claude_limits.rs`, which also reads Claude Code's status line.
//!
//! Privacy: the logs hold whole conversations. Only lines that contain the
//! literal `rate_limit_event` are parsed, only numbers leave this module, and
//! nothing from a line ever reaches the diagnostic log.
//!
//! ```text
//!  each poll ─▶ events.*.log* changed ≤ 8 days ago ─▶ appended bytes
//!               (first sight: last 4 MB, from the next line start)
//!           ─▶ complete lines containing "rate_limit_event"
//!           ─▶ JSON object {type:"rate_limit_event", rate_limit_info}
//!           ─▶ newest by line time
//! ```

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::claude_limits::{ClaudeReading, ClaudeWindow};
use crate::diag::Diag;

const NEEDLE: &[u8] = b"rate_limit_event";
/// First look at a file reads at most its last this many bytes.
const FIRST_READ_MAX: u64 = 4 << 20;
/// Files untouched for longer cannot hold a reading of the current week.
const MAX_FILE_AGE: i64 = 8 * 86_400;

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

}
