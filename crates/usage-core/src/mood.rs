//! Character moods and tray color. The core computes them; UI and tray only
//! render.
//!
//! ```text
//!  priority (higher wins)
//!  1 confused   governing limit Unknown, or Claude log format changed (D5)
//!  2 dozing     governing limit Stale
//!  3 panic      value ≥ 95, leave below 90
//!  4 sweat      value ≥ 75, leave below 70
//!  5 celebrate  4 s after a milestone
//!  6 focus      activity in the last 5 min
//!  7 sleep      30 min without activity (only if an activity signal exists)
//!  8 calm       otherwise
//!
//!  limits Off  → never confused/dozing from limits (Claude: terms gate)
//!  no activity signal → never focus/sleep
//! ```
//!
//! Tray: max governing value over providers whose limits are not Off,
//! same 75/70 and 95/90 hysteresis; dim when that value is Stale; grey when no
//! provider has a value; mono when all limits are Off.

use crate::model::{LimitsState, Mood, ProviderLimits, Tray};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Level {
    #[default]
    Normal,
    Sweat,
    Panic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Activity {
    /// No way to observe activity (e.g. Codex app-server down).
    NoSignal,
    /// Activity in the last 5 minutes.
    Active,
    /// Signal exists, quiet for less than 30 minutes.
    Idle,
    /// Signal exists, quiet for 30 minutes or more.
    Asleep,
}

pub(crate) fn next_level(prev: Level, value: Option<f64>) -> Level {
    let Some(v) = value else { return Level::Normal };
    if v >= 95.0 || (prev == Level::Panic && v >= 90.0) {
        Level::Panic
    } else if v >= 75.0 || (prev != Level::Normal && v >= 70.0) {
        Level::Sweat
    } else {
        Level::Normal
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MoodInput {
    pub limits: LimitsState,
    pub level: Level,
    pub format_changed: bool,
    pub celebrating: bool,
    pub activity: Activity,
}

pub(crate) fn mood(i: &MoodInput) -> Mood {
    let off = i.limits == LimitsState::Off;
    if i.format_changed || (!off && i.limits == LimitsState::Unknown) {
        return Mood::Confused;
    }
    if !off && i.limits == LimitsState::Stale {
        return Mood::Dozing;
    }
    match i.level {
        Level::Panic => return Mood::Panic,
        Level::Sweat => return Mood::Sweat,
        Level::Normal => {}
    }
    if i.celebrating {
        return Mood::Celebrate;
    }
    match i.activity {
        Activity::Active => Mood::Focus,
        Activity::Asleep => Mood::Sleep,
        Activity::Idle | Activity::NoSignal => Mood::Calm,
    }
}

/// Highest governing value over providers whose limits are not Off, with the
/// state of the limit that holds it.
fn max_governing(limits: &[ProviderLimits]) -> Option<(f64, LimitsState)> {
    limits
        .iter()
        .filter(|l| l.state != LimitsState::Off)
        .filter_map(|l| l.governing.map(|g| (g, l.state)))
        .max_by(|a, b| a.0.total_cmp(&b.0))
}

pub(crate) fn tray_value(limits: &[ProviderLimits]) -> Option<f64> {
    max_governing(limits).map(|(v, _)| v)
}

pub(crate) fn tray(limits: &[ProviderLimits], level: Level) -> Tray {
    if limits.iter().all(|l| l.state == LimitsState::Off) {
        return Tray::Mono;
    }
    let Some((_, state)) = max_governing(limits) else {
        return Tray::Grey;
    };
    let dim = state == LimitsState::Stale;
    match (level, dim) {
        (Level::Normal, false) => Tray::Green,
        (Level::Sweat, false) => Tray::Orange,
        (Level::Panic, false) => Tray::Red,
        (Level::Normal, true) => Tray::GreenDim,
        (Level::Sweat, true) => Tray::OrangeDim,
        (Level::Panic, true) => Tray::RedDim,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;

    fn walk(values: &[f64]) -> Vec<Level> {
        let mut l = Level::Normal;
        values
            .iter()
            .map(|v| {
                l = next_level(l, Some(*v));
                l
            })
            .collect()
    }

    #[test]
    fn hysteresis_from_test_plan() {
        // 76 → sweat, 72 → still sweat, 69 → calm
        assert_eq!(walk(&[76.0, 72.0, 69.0]), vec![Level::Sweat, Level::Sweat, Level::Normal]);
        // 96 → panic, 91 → still panic, 89 → sweat, 69 → normal
        assert_eq!(
            walk(&[96.0, 91.0, 89.0, 69.0]),
            vec![Level::Panic, Level::Panic, Level::Sweat, Level::Normal]
        );
        // Rising through 72 from below does not enter sweat.
        assert_eq!(walk(&[60.0, 72.0]), vec![Level::Normal, Level::Normal]);
        assert_eq!(next_level(Level::Panic, None), Level::Normal);
    }

    fn input(limits: LimitsState) -> MoodInput {
        MoodInput {
            limits,
            level: Level::Normal,
            format_changed: false,
            celebrating: false,
            activity: Activity::Idle,
        }
    }

    #[test]
    fn priorities() {
        let mut i = input(LimitsState::Unknown);
        i.level = Level::Panic;
        i.celebrating = true;
        assert_eq!(mood(&i), Mood::Confused);
        i.limits = LimitsState::Stale;
        assert_eq!(mood(&i), Mood::Dozing);
        i.limits = LimitsState::Known;
        assert_eq!(mood(&i), Mood::Panic);
        i.level = Level::Sweat;
        assert_eq!(mood(&i), Mood::Sweat);
        i.level = Level::Normal;
        assert_eq!(mood(&i), Mood::Celebrate);
        i.celebrating = false;
        i.activity = Activity::Active;
        assert_eq!(mood(&i), Mood::Focus);
        i.activity = Activity::Asleep;
        assert_eq!(mood(&i), Mood::Sleep);
        i.activity = Activity::NoSignal;
        assert_eq!(mood(&i), Mood::Calm);
    }

    #[test]
    fn limits_off_never_confused_or_dozing() {
        let mut i = input(LimitsState::Off);
        assert_eq!(mood(&i), Mood::Calm);
        i.activity = Activity::Active;
        assert_eq!(mood(&i), Mood::Focus);
        i.celebrating = true;
        assert_eq!(mood(&i), Mood::Celebrate);
        // The format health check still confuses Claude.
        i.format_changed = true;
        assert_eq!(mood(&i), Mood::Confused);
    }

    fn pl(provider: Provider, state: LimitsState, governing: Option<f64>) -> ProviderLimits {
        ProviderLimits {
            provider,
            state,
            governing,
            windows: Vec::new(),
            note: None,
        }
    }

    #[test]
    fn tray_colors() {
        let off = pl(Provider::Claude, LimitsState::Off, None);
        assert_eq!(tray(&[off.clone(), pl(Provider::Codex, LimitsState::Off, None)], Level::Normal), Tray::Mono);
        assert_eq!(tray(&[off.clone(), pl(Provider::Codex, LimitsState::Unknown, None)], Level::Normal), Tray::Grey);
        let known = [off.clone(), pl(Provider::Codex, LimitsState::Known, Some(80.0))];
        assert_eq!(tray_value(&known), Some(80.0));
        assert_eq!(tray(&known, next_level(Level::Normal, Some(80.0))), Tray::Orange);
        let stale = [off.clone(), pl(Provider::Codex, LimitsState::Stale, Some(97.0))];
        assert_eq!(tray(&stale, next_level(Level::Normal, Some(97.0))), Tray::RedDim);
        let low = [off, pl(Provider::Codex, LimitsState::Stale, Some(10.0))];
        assert_eq!(tray(&low, Level::Normal), Tray::GreenDim);
    }
}
