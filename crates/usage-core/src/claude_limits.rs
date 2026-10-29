//! Claude's 5h and weekly limits. Two passive, credential-free sources, the
//! newest reading wins:
//!
//! - T3 Code's provider logs (`t3.rs`): the Claude Agent SDK's
//!   `rate_limit_event`, while Claude works in T3 Code.
//! - Claude Code's status line (`claude_code.rs`): `rate_limits` from the CLI
//!   in any terminal (iTerm2, Terminal, …), once connected in Settings.
//!
//! ```text
//!  every 5 s ─┬▶ t3::Tail::poll        (appended log lines)   ─┐ newest
//!             └▶ claude_code::Records  (one file per session) ─┴▶ Update::ClaudeLimits
//! ```
//!
//! Window states (the readings are passive, so time alone never ages them):
//! Known until Claude work shows up in the JSONL more than 15 min after the
//! reading (work no source reported, the value may be higher) → Stale. Once
//! `resetsAt` passes: Known 0 % when there was no Claude work since the reset,
//! AfterReset otherwise.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::claude_code::Records;
use crate::codex::governing;
use crate::diag::Diag;
use crate::model::{LimitWindow, LimitsState, Provider, ProviderLimits, WindowKind, WindowState};
use crate::t3::Tail;
use crate::{Clock, CoreMsg, Update};

const POLL: Duration = Duration::from_secs(5);
const STEP: Duration = Duration::from_millis(250);
/// Claude work this long after the reading happened where no source reports
/// limits (both write a new reading within minutes while Claude works).
pub(crate) const STALE_AFTER_WORK: i64 = 15 * 60;
const WEEK: i64 = 7 * 86_400;

pub(crate) const NOTE_OFF: &str = "limity Claude: připoj Claude Code v Nastavení";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaudeWindow {
    /// 0..100
    pub percent: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaudeReading {
    /// When the numbers were current (epoch s).
    pub at: i64,
    pub five_hour: Option<ClaudeWindow>,
    pub seven_day: Option<ClaudeWindow>,
}

/// Per source: T3 Code's log folder, Claude Code's status line folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Sources {
    pub t3: bool,
    pub code: bool,
}

impl Sources {
    fn any(self) -> bool {
        self.t3 || self.code
    }
}

fn note_waiting(available: Sources) -> &'static str {
    match (available.code, available.t3) {
        (true, true) => "limity Claude se ukážou po další odpovědi v Claude Code nebo T3 Code",
        (true, false) => "limity Claude se ukážou po další odpovědi v Claude Code",
        _ => "limity Claude se ukážou po další zprávě v T3 Code",
    }
}

fn note_stale(available: Sources) -> &'static str {
    match (available.code, available.t3) {
        (true, true) => "Claude od té doby běžel mimo terminál i T3 Code",
        (true, false) => "Claude od té doby běžel mimo terminál",
        (false, true) => "Claude od té doby běžel mimo T3 Code",
        (false, false) => "Claude od té doby běžel jinde",
    }
}

// ---------------------------------------------------------------- thread

pub(crate) struct ClaudeLimitsThread {
    pub t3_dir: Option<PathBuf>,
    pub code_dir: Option<PathBuf>,
    pub core: Sender<CoreMsg>,
    /// Any message polls right away (manual refresh, Claude Code connected).
    pub wake: Receiver<()>,
    pub stop: Arc<AtomicBool>,
    pub clock: Clock,
    pub diag: Diag,
}

impl ClaudeLimitsThread {
    pub(crate) fn spawn(self) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("usage-claude-limits".into())
            .spawn(move || self.run())
            .expect("spawn Claude limits reader")
    }

    fn run(self) {
        let mut tail = Tail::default();
        let mut records = Records::default();
        let mut sent_at: Option<i64> = None;
        let mut sent_sources: Option<Sources> = None;
        let present = |dir: &Option<PathBuf>| dir.as_deref().is_some_and(Path::is_dir);
        loop {
            let now = (self.clock)();
            let sources = Sources {
                t3: present(&self.t3_dir),
                code: present(&self.code_dir),
            };
            let from_t3 = self.t3_dir.as_deref().filter(|_| sources.t3).and_then(|d| tail.poll(d, now, &self.diag));
            let from_code = self.code_dir.as_deref().filter(|_| sources.code).and_then(|d| records.newest(d));
            let reading = [from_t3, from_code]
                .into_iter()
                .flatten()
                .max_by_key(|r| r.at)
                .filter(|r| sent_at.is_none_or(|t| r.at > t));
            if reading.is_some() || sent_sources != Some(sources) {
                if let Some(r) = &reading {
                    sent_at = Some(r.at);
                }
                sent_sources = Some(sources);
                if self
                    .core
                    .send(CoreMsg::Update(Update::ClaudeLimits { sources, reading }))
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
    /// Sources present right now (T3 Code installed, Claude Code connected).
    pub available: Sources,
    pub reading: Option<ClaudeReading>,
}

impl ClaudeLimitsView {
    /// `last_work`: newest Claude assistant line from the JSONL, any source.
    pub(crate) fn provider_limits(&self, now: i64, last_work: Option<i64>) -> ProviderLimits {
        let Some(r) = &self.reading else {
            let waiting = self.available.any();
            return ProviderLimits {
                provider: Provider::Claude,
                state: if waiting { LimitsState::Unknown } else { LimitsState::Off },
                governing: None,
                windows: if waiting {
                    vec![unknown(WindowKind::FiveHour, "5h"), unknown(WindowKind::Week, "týden")]
                } else {
                    Vec::new()
                },
                note: Some(if waiting { note_waiting(self.available) } else { NOTE_OFF }.into()),
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
                LimitsState::Stale => Some(note_stale(self.available).into()),
                LimitsState::Unknown => Some(note_waiting(self.available).into()),
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

    const T: i64 = 1_793_200_000;
    const T3_ONLY: Sources = Sources { t3: true, code: false };

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

    fn view(available: Sources, r: ClaudeReading) -> ClaudeLimitsView {
        ClaudeLimitsView {
            available,
            reading: Some(r),
        }
    }

    #[test]
    fn known_until_unreported_work_then_stale() {
        let v = view(T3_ONLY, reading(T, 42.0, T + 3600, 12.0, T + 86_400));
        let l = v.provider_limits(T + 600, Some(T + 60));
        assert_eq!(l.state, LimitsState::Known);
        assert_eq!(l.governing, Some(42.0));
        assert_eq!(l.note, None);
        // Hours later with no Claude work at all: still known (passive source).
        assert_eq!(v.provider_limits(T + 3000, Some(T)).state, LimitsState::Known);
        // Work 20 min after the reading that no source reported.
        let l = v.provider_limits(T + 1500, Some(T + 1200));
        assert_eq!(l.state, LimitsState::Stale);
        assert_eq!(l.windows[0].state, WindowState::Stale);
        assert_eq!(l.note.as_deref(), Some("Claude od té doby běžel mimo T3 Code"));
        let both = view(Sources { t3: true, code: true }, reading(T, 42.0, T + 3600, 12.0, T + 86_400));
        assert_eq!(
            both.provider_limits(T + 1500, Some(T + 1200)).note.as_deref(),
            Some("Claude od té doby běžel mimo terminál i T3 Code")
        );
    }

    #[test]
    fn after_reset_is_empty_without_work_and_unknown_with_it() {
        let v = view(T3_ONLY, reading(T, 80.0, T + 3600, 30.0, T + 86_400));
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
    fn no_source_is_off_and_waiting_names_the_sources() {
        let none = ClaudeLimitsView::default().provider_limits(T, None);
        assert_eq!(none.state, LimitsState::Off);
        assert_eq!(none.note.as_deref(), Some(NOTE_OFF));
        assert!(none.windows.is_empty());

        let waiting = |available| {
            ClaudeLimitsView {
                available,
                reading: None,
            }
            .provider_limits(T, None)
        };
        let t3 = waiting(T3_ONLY);
        assert_eq!(t3.state, LimitsState::Unknown);
        assert_eq!(t3.windows.len(), 2);
        assert_eq!(t3.note.as_deref(), Some("limity Claude se ukážou po další zprávě v T3 Code"));
        let code = waiting(Sources { t3: false, code: true });
        assert_eq!(code.state, LimitsState::Unknown);
        assert_eq!(code.note.as_deref(), Some("limity Claude se ukážou po další odpovědi v Claude Code"));
    }
}
