//! Claude limits from Claude Code's status line. Credential-free like `t3.rs`.
//!
//! Claude Code (the CLI in any terminal: iTerm2, Terminal, Windows Terminal,
//! an editor's terminal) runs the `statusLine` command from its settings and
//! hands it the session as JSON on stdin. For Pro and Max that includes the
//! account's `rate_limits.five_hour` and `rate_limits.seven_day`
//! (`used_percentage` 0–100, `resets_at` in epoch seconds), documented at
//! code.claude.com/docs/en/statusline. Once the user connects Claude Code in
//! Settings, that command is Skylimit itself (`skylimit statusline`, see the
//! shell's `claude_code.rs`), which calls [`record`] and exits. Nothing here
//! reads Claude login data or calls Anthropic.
//!
//! Privacy: only the session id (a random UUID, used as the file name), a
//! change marker built from counters and the two windows are kept. Working
//! directory, transcript path, model and everything else in the input are
//! dropped on the spot.
//!
//! Freshness: the status line also runs without a new response (session
//! start, a permission mode change, a timer) and then carries the numbers
//! from the session's last response, which may be hours old. A reading counts
//! only when the marker moved since the previous run of the same session,
//! that is when a response just arrived. The first run of a session only
//! remembers the marker.
//!
//! ```text
//!  Claude Code ─stdin JSON─▶ skylimit statusline ─▶ record()
//!              ─▶ <data>/claude-code/s-<session>.json
//!  core, every 5 s ─▶ Records::newest() ─▶ newest `at` of all sessions
//! ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude_limits::{ClaudeReading, ClaudeWindow};

/// Folder under Skylimit's data root shared by the CLI (writes) and the core
/// (reads). It exists exactly while Claude Code is connected.
pub const DIR_NAME: &str = "claude-code";
const FILE_PREFIX: &str = "s-";
/// Sessions untouched for longer cannot hold a reading of the current week.
const MAX_AGE: Duration = Duration::from_secs(8 * 86_400);
/// The input is a few kilobytes; anything far larger is not a status line.
pub const MAX_INPUT: usize = 4 << 20;

pub fn session_dir(data_root: &Path) -> PathBuf {
    data_root.join(DIR_NAME)
}

/// One limit window as Claude Code reports it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// 0..100
    pub percent: f64,
    pub resets_at: Option<i64>,
}

/// What Skylimit keeps from one status line input.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub session: String,
    /// Changes whenever the session gets an API response.
    pub mark: String,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

/// Result of [`record`]: the windows Claude Code knows now (for the default
/// status line text) and whether they counted as a new reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub fresh: bool,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    v: u8,
    mark: String,
    /// When the windows were seen right after a response; `None` until then.
    at: Option<i64>,
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

// ---------------------------------------------------------------- parsing

/// The parts of a status line input Skylimit uses, or `None` when it is not
/// one (no session id, not JSON).
pub fn parse(json: &str) -> Option<Sample> {
    let v: Value = serde_json::from_str(json).ok()?;
    let session = sanitize(v.get("session_id")?.as_str()?)?;
    // Counters that move with every API response; their raw JSON text is
    // compared, never interpreted.
    let mark = [
        v.pointer("/cost/total_api_duration_ms"),
        v.pointer("/cost/total_cost_usd"),
        v.pointer("/prompt_cache/requests"),
        v.pointer("/context_window/total_output_tokens"),
    ]
    .iter()
    .map(|x| x.map_or_else(String::new, Value::to_string))
    .collect::<Vec<_>>()
    .join("|");
    let limits = v.get("rate_limits");
    Some(Sample {
        session,
        mark,
        five_hour: limits.and_then(|l| l.get("five_hour")).and_then(window),
        seven_day: limits.and_then(|l| l.get("seven_day")).and_then(window),
    })
}

fn window(v: &Value) -> Option<Window> {
    let percent = v.get("used_percentage").and_then(Value::as_f64).filter(|p| p.is_finite())?;
    let resets_at = v
        .get("resets_at")
        .and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)));
    Some(Window {
        percent: ((percent * 10.0).round() / 10.0).clamp(0.0, 100.0),
        resets_at,
    })
}

/// A file-name-safe session id, or `None` when nothing usable is left.
fn sanitize(id: &str) -> Option<String> {
    let clean: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(80)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

// ---------------------------------------------------------------- recording

/// Remembers the sample in `dir` (created if needed) and says whether it was
/// a new reading. A window the input lacks (Claude Code drops one once it
/// resets) keeps its previous value while that has not reset yet.
pub fn record(dir: &Path, sample: &Sample, now: i64) -> io::Result<Recorded> {
    let path = dir.join(format!("{FILE_PREFIX}{}.json", sample.session));
    let previous = read_stored(&path);
    let carry = |new: Option<Window>, old: Option<Window>| {
        new.or_else(|| old.filter(|w| w.resets_at.is_some_and(|r| r > now)))
    };
    let five_hour = carry(sample.five_hour, previous.as_ref().and_then(|p| p.five_hour));
    let seven_day = carry(sample.seven_day, previous.as_ref().and_then(|p| p.seven_day));

    if previous.as_ref().is_some_and(|p| p.mark == sample.mark) {
        return Ok(Recorded {
            fresh: false,
            five_hour,
            seven_day,
        });
    }
    // Only numbers that came with this input are new; carried ones keep their age.
    let fresh = previous.is_some() && (sample.five_hour.is_some() || sample.seven_day.is_some());
    let stored = Stored {
        v: 1,
        mark: sample.mark.clone(),
        at: if fresh { Some(now) } else { previous.as_ref().and_then(|p| p.at) },
        five_hour,
        seven_day,
    };
    fs::create_dir_all(dir)?;
    if previous.is_none() {
        prune(dir);
    }
    write_atomic(&path, &serde_json::to_vec(&stored).map_err(io::Error::other)?)?;
    Ok(Recorded {
        fresh,
        five_hour,
        seven_day,
    })
}

fn read_stored(path: &Path) -> Option<Stored> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Write next to the target, then rename over it: a reader never sees half a
/// file, and a status line run that Claude Code cancels leaves no debris.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Drops session files nobody has touched for over a week.
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(FILE_PREFIX) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > MAX_AGE);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The default status line: `☀ Claude 5h 23 % · týden 41 %`, or empty when
/// Claude Code reports no limits (API key, before the first response).
pub fn status_text(five_hour: Option<&Window>, seven_day: Option<&Window>) -> String {
    let parts: Vec<String> = [("5h", five_hour), ("týden", seven_day)]
        .iter()
        .filter_map(|(label, w)| w.map(|w| format!("{label} {:.0} %", w.percent)))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("☀ Claude {}", parts.join(" · "))
    }
}

// ---------------------------------------------------------------- reading

/// Newest reading in `dir`. Files are parsed again only when they change.
#[derive(Default)]
pub(crate) struct Records {
    cache: HashMap<PathBuf, (SystemTime, u64, Option<ClaudeReading>)>,
}

impl Records {
    pub(crate) fn newest(&mut self, dir: &Path) -> Option<ClaudeReading> {
        let entries = fs::read_dir(dir).ok()?;
        let mut best: Option<ClaudeReading> = None;
        let mut seen = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(FILE_PREFIX) || !name.ends_with(".json") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(mtime) = meta.modified() else { continue };
            if mtime.elapsed().is_ok_and(|age| age > MAX_AGE) {
                continue;
            }
            let path = entry.path();
            let reading = match self.cache.get(&path) {
                Some((t, len, r)) if *t == mtime && *len == meta.len() => r.clone(),
                _ => {
                    let r = read_stored(&path).and_then(reading_of);
                    self.cache.insert(path.clone(), (mtime, meta.len(), r.clone()));
                    r
                }
            };
            seen.push(path);
            if let Some(r) = reading {
                if best.as_ref().is_none_or(|b| r.at > b.at) {
                    best = Some(r);
                }
            }
        }
        self.cache.retain(|p, _| seen.contains(p));
        best
    }
}

fn reading_of(s: Stored) -> Option<ClaudeReading> {
    let at = s.at?;
    let convert = |w: Window| ClaudeWindow {
        percent: w.percent,
        resets_at: w.resets_at,
    };
    let r = ClaudeReading {
        at,
        five_hour: s.five_hour.map(convert),
        seven_day: s.seven_day.map(convert),
    };
    (r.five_hour.is_some() || r.seven_day.is_some()).then_some(r)
}

/// When Claude Code last delivered limits (for the Settings card).
pub fn last_reading_at(dir: &Path) -> Option<i64> {
    Records::default().newest(dir).map(|r| r.at)
}

/// Seconds since the Unix epoch (the CLI has no injected clock).
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_793_200_000;

    fn input(session: &str, api_ms: u64, five: Option<f64>, week: Option<f64>) -> String {
        let mut limits = serde_json::Map::new();
        if let Some(p) = five {
            limits.insert("five_hour".into(), serde_json::json!({ "used_percentage": p, "resets_at": T + 3600 }));
        }
        if let Some(p) = week {
            limits.insert("seven_day".into(), serde_json::json!({ "used_percentage": p, "resets_at": T + 86_400 }));
        }
        serde_json::json!({
            "session_id": session,
            "cwd": "/Users/someone/secret-project",
            "transcript_path": "/Users/someone/.claude/projects/x/abc.jsonl",
            "model": { "id": "claude-opus-5-5", "display_name": "Opus" },
            "cost": { "total_cost_usd": api_ms as f64 / 1000.0, "total_api_duration_ms": api_ms },
            "rate_limits": limits,
        })
        .to_string()
    }

    #[test]
    fn parses_the_documented_input_and_keeps_only_numbers() {
        let s = parse(&input("abc-123", 2300, Some(23.54), Some(41.2))).unwrap();
        assert_eq!(s.session, "abc-123");
        assert_eq!(s.five_hour, Some(Window { percent: 23.5, resets_at: Some(T + 3600) }));
        assert_eq!(s.seven_day, Some(Window { percent: 41.2, resets_at: Some(T + 86_400) }));
        assert!(!s.mark.contains("secret-project"));
        assert!(!s.mark.contains("Opus"));
    }

    #[test]
    fn inputs_without_limits_or_session_still_parse_or_are_rejected() {
        let no_limits = parse(&input("abc", 0, None, None)).unwrap();
        assert_eq!((no_limits.five_hour, no_limits.seven_day), (None, None));
        assert!(parse(r#"{"rate_limits":{}}"#).is_none(), "no session id");
        assert!(parse("not json").is_none());
        assert_eq!(parse(r#"{"session_id":"../../etc/x"}"#).unwrap().session, "etcx");
        assert!(parse(r#"{"session_id":"///"}"#).is_none());
    }

    #[test]
    fn only_a_moved_marker_makes_a_reading() {
        let dir = tempfile::tempdir().unwrap();
        let mut records = Records::default();

        // First run of a session (start or resume): numbers may be old.
        let first = record(dir.path(), &parse(&input("s1", 1000, Some(10.0), Some(20.0))).unwrap(), T).unwrap();
        assert!(!first.fresh);
        assert_eq!(first.five_hour.unwrap().percent, 10.0, "the status line still shows them");
        assert!(records.newest(dir.path()).is_none());

        // A run without a new response (permission mode, timer): still nothing.
        let same = record(dir.path(), &parse(&input("s1", 1000, Some(10.0), Some(20.0))).unwrap(), T + 600).unwrap();
        assert!(!same.fresh);
        assert!(records.newest(dir.path()).is_none());

        // A response arrived: the counters moved.
        let next = record(dir.path(), &parse(&input("s1", 4000, Some(12.0), Some(21.0))).unwrap(), T + 700).unwrap();
        assert!(next.fresh);
        let r = records.newest(dir.path()).unwrap();
        assert_eq!(r.at, T + 700);
        assert_eq!(r.five_hour.unwrap().percent, 12.0);

        // An idle re-run later keeps the time of that response.
        record(dir.path(), &parse(&input("s1", 4000, Some(12.0), Some(21.0))).unwrap(), T + 5000).unwrap();
        assert_eq!(records.newest(dir.path()).unwrap().at, T + 700);
    }

    #[test]
    fn a_resumed_session_without_limits_yet_does_not_refresh_old_numbers() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), &parse(&input("s1", 1, Some(10.0), Some(20.0))).unwrap(), T).unwrap();
        record(dir.path(), &parse(&input("s1", 2, Some(11.0), Some(21.0))).unwrap(), T + 60).unwrap();
        // Resumed half an hour later: counters differ, no response yet so no limits.
        let resumed = record(dir.path(), &parse(&input("s1", 3, None, None)).unwrap(), T + 1800).unwrap();
        assert!(!resumed.fresh);
        assert_eq!(Records::default().newest(dir.path()).unwrap().at, T + 60, "still the old reading's time");
    }

    #[test]
    fn newest_session_wins_and_missing_windows_carry_over() {
        let dir = tempfile::tempdir().unwrap();
        for (s, ms) in [("a", 1), ("b", 1)] {
            record(dir.path(), &parse(&input(s, ms, Some(5.0), Some(5.0))).unwrap(), T).unwrap();
        }
        record(dir.path(), &parse(&input("a", 2, Some(30.0), Some(40.0))).unwrap(), T + 100).unwrap();
        // Session b answers later but its input lacks the 5h window.
        record(dir.path(), &parse(&input("b", 2, None, Some(41.0))).unwrap(), T + 200).unwrap();
        let r = Records::default().newest(dir.path()).unwrap();
        assert_eq!(r.at, T + 200);
        assert_eq!(r.seven_day.unwrap().percent, 41.0);
        assert_eq!(r.five_hour.unwrap().percent, 5.0, "b's own earlier 5h value, not yet reset");
        assert_eq!(last_reading_at(dir.path()), Some(T + 200));
    }

    #[test]
    fn stored_files_hold_no_text_from_the_session() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), &parse(&input("s1", 1, Some(1.0), None)).unwrap(), T).unwrap();
        record(dir.path(), &parse(&input("s1", 2, Some(2.0), None)).unwrap(), T + 1).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("s-s1.json")], "no temporary files left");
        let text = fs::read_to_string(dir.path().join("s-s1.json")).unwrap();
        for leak in ["secret-project", "transcript", "Opus", "cwd"] {
            assert!(!text.contains(leak), "{leak} leaked: {text}");
        }
    }

    #[test]
    fn default_status_text() {
        let w = |p| Some(Window { percent: p, resets_at: None });
        assert_eq!(status_text(w(23.5).as_ref(), w(41.2).as_ref()), "☀ Claude 5h 24 % · týden 41 %");
        assert_eq!(status_text(None, w(7.0).as_ref()), "☀ Claude týden 7 %");
        assert_eq!(status_text(None, None), "");
    }
}
