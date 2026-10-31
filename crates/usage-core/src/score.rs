//! The score: API-equivalent cost of this calendar month (Europe/Prague)
//! against the flat fee, plus breakdowns, streaks and milestones.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, Days, NaiveDate, TimeZone};
use chrono_tz::Europe::Prague;

use crate::fx::FxDay;
use crate::model::{
    BreakdownRow, CelebrationWho, PlanPeriod, PlanSetting, Provider, ProviderScore, ScoreState, Settings, Total,
};
use crate::pricing::{normalize_model, PriceTable};
use crate::store::{UsageRow, WebRow};

pub(crate) fn prague_date(ts: i64) -> NaiveDate {
    Prague
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

pub(crate) fn prague_date_string(ts: i64) -> String {
    prague_date(ts).format("%Y-%m-%d").to_string()
}

pub(crate) fn local_midnight(d: NaiveDate) -> i64 {
    Prague
        .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .map(|t| t.timestamp())
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MonthBounds {
    /// `YYYY-MM`
    pub month: String,
    /// UTC epoch of local midnight on the 1st (whole hour: Prague is UTC+1/+2).
    pub start: i64,
    pub end: i64,
    pub first_day: NaiveDate,
    pub today: NaiveDate,
}

pub(crate) fn month_bounds(now: i64) -> MonthBounds {
    let today = prague_date(now);
    let first_day = today.with_day(1).unwrap_or(today);
    let next_first = if today.month() == 12 {
        NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
    }
    .unwrap_or(first_day);
    MonthBounds {
        month: first_day.format("%Y-%m").to_string(),
        start: local_midnight(first_day),
        end: local_midnight(next_first),
        first_day,
        today,
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ProviderTotals {
    pub usd: f64,
    pub tokens: u64,
    pub unpriced: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MonthCost {
    pub claude: ProviderTotals,
    pub codex: ProviderTotals,
    pub daily_claude_usd: BTreeMap<NaiveDate, f64>,
    pub daily_codex_usd: BTreeMap<NaiveDate, f64>,
    pub by_source: HashMap<String, (f64, u64)>,
    pub by_model: HashMap<String, (f64, u64)>,
    pub by_project: HashMap<String, (f64, u64)>,
    pub unpriced_models: BTreeSet<String>,
}

fn bump(map: &mut HashMap<String, (f64, u64)>, key: &str, usd: f64, tokens: u64) {
    let e = map.entry(key.to_string()).or_insert((0.0, 0));
    e.0 += usd;
    e.1 += tokens;
}

/// Price this month's rows. `excluded` entrypoints are dropped here (query
/// time), never at ingest. Codex rows enter the breakdowns only when
/// `include_codex` (i.e. it counts towards the total).
pub(crate) fn month_cost(
    rows: &[UsageRow],
    web: &[WebRow],
    codex_buckets: &[(String, u64)],
    prices: &PriceTable,
    codex_rate: Option<f64>,
    excluded: &[String],
    include_codex: bool,
) -> MonthCost {
    let mut out = MonthCost::default();
    for r in rows {
        if excluded.iter().any(|e| e == &r.entrypoint) {
            continue;
        }
        let unit = prices
            .lookup(&r.model)
            .and_then(|p| p.unit_price(r.kind, r.band, &r.speed, &r.service_tier));
        let usd = match unit {
            Some(u) => u * r.tokens as f64,
            None => {
                out.claude.unpriced += r.tokens;
                out.unpriced_models.insert(r.model.clone());
                0.0
            }
        };
        out.claude.usd += usd;
        out.claude.tokens += r.tokens;
        *out.daily_claude_usd.entry(prague_date(r.hour)).or_default() += usd;
        bump(&mut out.by_source, &r.entrypoint, usd, r.tokens);
        bump(&mut out.by_model, &normalize_model(&r.model), usd, r.tokens);
        bump(&mut out.by_project, &r.project, usd, r.tokens);
    }
    for w in web {
        if excluded.iter().any(|e| e == &w.entrypoint) {
            continue;
        }
        let Some(price) = prices.lookup(&w.model).and_then(|p| p.web_search_price()) else {
            out.unpriced_models.insert(w.model.clone());
            continue;
        };
        let usd = price * w.requests as f64;
        out.claude.usd += usd;
        *out.daily_claude_usd.entry(prague_date(w.hour)).or_default() += usd;
        bump(&mut out.by_source, &w.entrypoint, usd, 0);
        bump(&mut out.by_model, &normalize_model(&w.model), usd, 0);
        bump(&mut out.by_project, &w.project, usd, 0);
    }
    for (date, tokens) in codex_buckets {
        out.codex.tokens += tokens;
        let usd = codex_rate.map(|r| r * *tokens as f64);
        match usd {
            Some(usd) => {
                out.codex.usd += usd;
                if let Ok(d) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                    *out.daily_codex_usd.entry(d).or_default() += usd;
                }
                if include_codex {
                    bump(&mut out.by_source, "codex", usd, *tokens);
                    bump(&mut out.by_model, "codex (odhad)", usd, *tokens);
                }
            }
            None => out.codex.unpriced += tokens,
        }
    }
    out
}

pub(crate) fn plan_monthly_display(plan: &PlanSetting, settings: &Settings, fx: &FxDay) -> f64 {
    let monthly = match plan.period {
        PlanPeriod::Month => plan.amount,
        PlanPeriod::Year => plan.amount / 12.0,
    };
    fx.convert(monthly, plan.currency, settings.currency)
}

fn priced(state: ScoreState) -> bool {
    matches!(state, ScoreState::Ok | ScoreState::Estimate | ScoreState::Partial)
}

fn name(p: Provider) -> &'static str {
    match p {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

/// Per-provider scores and the total. Providers without a price (or excluded
/// by settings) drop out of the total together with their plan, and the
/// total multiple only counts providers that have both a price and a plan.
pub(crate) fn scores_and_total(
    cost: &MonthCost,
    claude_state: ScoreState,
    codex_state: ScoreState,
    settings: &Settings,
    fx: &FxDay,
) -> (Vec<ProviderScore>, Total) {
    let conv = fx.usd_to(settings.currency);
    let entries = [
        (Provider::Claude, &cost.claude, claude_state, settings.plans.claude.as_ref()),
        (Provider::Codex, &cost.codex, codex_state, settings.plans.codex.as_ref()),
    ];
    let mut scores = Vec::new();
    let mut total_display = 0.0;
    let mut num = 0.0;
    let mut den = 0.0;
    let mut notes: Vec<String> = Vec::new();
    let mut included_without_plan: Vec<Provider> = Vec::new();

    for (provider, totals, state, plan) in entries {
        let display = totals.usd * conv;
        let plan_display = plan.map(|p| plan_monthly_display(p, settings, fx)).filter(|v| *v > 0.0);
        let multiple = match plan_display {
            Some(pd) if priced(state) => Some(display / pd),
            _ => None,
        };
        scores.push(ProviderScore {
            provider,
            usd: totals.usd,
            display,
            tokens: totals.tokens,
            unpriced_tokens: totals.unpriced,
            state,
            plan_monthly_display: plan_display,
            multiple,
        });

        let excluded_by_setting = provider == Provider::Codex && !settings.codex_estimate_in_score;
        let included = priced(state) && !excluded_by_setting;
        if included {
            total_display += display;
            match plan_display {
                Some(pd) => {
                    num += display;
                    den += pd;
                }
                None => included_without_plan.push(provider),
            }
            continue;
        }
        let why = match state {
            ScoreState::Off => format!("{} vypnutý", name(provider)),
            ScoreState::FormatChanged => format!("{}: formát záznamů se změnil", name(provider)),
            ScoreState::Unknown => format!("{} bez ceny", name(provider)),
            _ if excluded_by_setting => "Odhad Codexu vypnutý".to_string(),
            _ => format!("{} nezapočten", name(provider)),
        };
        if plan_display.is_some() {
            notes.push(format!("{why}, paušál nezapočten"));
        } else if state != ScoreState::Off {
            notes.push(format!("{why}, nezapočten"));
        }
    }
    if den > 0.0 {
        for p in included_without_plan {
            notes.push(format!("{} bez paušálu, do násobku nezapočten", name(p)));
        }
    }
    let total = Total {
        display: total_display,
        multiple: if den > 0.0 { Some(num / den) } else { None },
        plan_monthly_display: if den > 0.0 { Some(den) } else { None },
        excluded_note: if notes.is_empty() { None } else { Some(notes.join("; ")) },
    };
    (scores, total)
}

pub(crate) fn breakdown(map: &HashMap<String, (f64, u64)>, conv: f64) -> Vec<BreakdownRow> {
    let mut rows: Vec<BreakdownRow> = map
        .iter()
        .map(|(k, (usd, tokens))| BreakdownRow {
            key: k.clone(),
            usd: *usd,
            display: usd * conv,
            tokens: *tokens,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.usd
            .total_cmp(&a.usd)
            .then(b.tokens.cmp(&a.tokens))
            .then(a.key.cmp(&b.key))
    });
    rows
}

/// Every day of the month up to today.
pub(crate) fn month_days(b: &MonthBounds) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut d = b.first_day;
    while d <= b.today {
        out.push(d);
        match d.checked_add_days(Days::new(1)) {
            Some(n) => d = n,
            None => break,
        }
    }
    out
}

/// Tokens per Prague calendar day from hourly totals.
pub(crate) fn daily_tokens(hourly: &[(i64, u64)]) -> BTreeMap<NaiveDate, u64> {
    let mut out = BTreeMap::new();
    for (hour, tokens) in hourly {
        *out.entry(prague_date(*hour)).or_default() += tokens;
    }
    out
}

/// (current, longest, first day of the current streak). The current streak
/// may end yesterday: a day without activity *yet* does not break it.
pub(crate) fn streaks(days: &BTreeMap<NaiveDate, u64>, today: NaiveDate) -> (u32, u32, Option<NaiveDate>) {
    let active: BTreeSet<NaiveDate> = days.iter().filter(|(_, t)| **t > 0).map(|(d, _)| *d).collect();
    let mut longest = 0u32;
    let mut run = 0u32;
    let mut prev: Option<NaiveDate> = None;
    for d in &active {
        run = match prev {
            Some(p) if p.succ_opt() == Some(*d) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(*d);
    }
    let mut d = if active.contains(&today) {
        today
    } else {
        today.pred_opt().unwrap_or(today)
    };
    let mut current = 0u32;
    let mut start = None;
    while active.contains(&d) {
        current += 1;
        start = Some(d);
        match d.pred_opt() {
            Some(p) => d = p,
            None => break,
        }
    }
    (current, longest, start)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub id: String,
    pub who: CelebrationWho,
    pub text: String,
}

pub(crate) const MULTIPLE_STEPS: [u32; 5] = [1, 2, 3, 5, 10];
pub(crate) const STREAK_STEPS: [u32; 3] = [7, 14, 30];

fn who(p: Provider) -> CelebrationWho {
    match p {
        Provider::Claude => CelebrationWho::Claude,
        Provider::Codex => CelebrationWho::Codex,
    }
}

/// Milestones currently satisfied. Ids are stable so each is celebrated once:
/// multiple per month and step, daily max per provider and day, streak per
/// provider, step and streak start.
pub(crate) fn milestone_candidates(
    month: &str,
    total_multiple: Option<f64>,
    daily: &[(Provider, &BTreeMap<NaiveDate, u64>)],
    streak_info: &[(Provider, u32, Option<NaiveDate>)],
    today: NaiveDate,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    if let Some(m) = total_multiple {
        for k in MULTIPLE_STEPS {
            if m >= f64::from(k) {
                out.push(Candidate {
                    id: format!("multiple:{month}:{k}"),
                    who: CelebrationWho::Both,
                    text: format!("Tento měsíc už {k}× paušálu!"),
                });
            }
        }
    }
    for (p, days) in daily {
        let today_tokens = days.get(&today).copied().unwrap_or(0);
        let prev_max = days.range(..today).map(|(_, t)| *t).max().unwrap_or(0);
        if prev_max > 0 && today_tokens > prev_max {
            out.push(Candidate {
                id: format!("dailymax:{}:{today}", p.as_str()),
                who: who(*p),
                text: format!("Nový denní rekord tokenů ({})!", name(*p)),
            });
        }
    }
    for (p, current, start) in streak_info {
        let Some(start) = start else { continue };
        for n in STREAK_STEPS {
            if *current >= n {
                out.push(Candidate {
                    id: format!("streak:{}:{n}:{start}", p.as_str()),
                    who: who(*p),
                    text: format!("Série {n} dní v řadě ({})!", name(*p)),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Currency, PlanCurrency, Plans};
    use crate::pricing::TokenKind;

    fn row(hour: i64, model: &str, entrypoint: &str, kind: TokenKind, tokens: u64) -> UsageRow {
        UsageRow {
            hour,
            provider: "claude".into(),
            model: model.into(),
            entrypoint: entrypoint.into(),
            project: "alpha".into(),
            speed: "standard".into(),
            service_tier: "standard".into(),
            band: 0,
            kind,
            tokens,
        }
    }

    #[test]
    fn month_boundary_uses_prague_time() {
        // 2026-10-31T23:30:00Z is 00:30 on 1 November in Prague (CET).
        let ts = 1_793_489_400;
        assert_eq!(prague_date_string(ts), "2026-11-01");
        let oct = month_bounds(1_793_190_000);
        assert_eq!(oct.month, "2026-10");
        assert!(ts >= oct.end, "the message belongs to November");
        let nov = month_bounds(ts);
        assert_eq!(nov.month, "2026-11");
        assert_eq!(nov.start, 1_793_487_600); // 2026-10-31T23:00:00Z
        assert!(ts >= nov.start && ts < nov.end);
        // Summer time: October starts at 22:00 UTC on 30 September.
        assert_eq!(oct.start, 1_790_805_600);
        assert_eq!(month_days(&oct).len(), 28);
    }

    #[test]
    fn unpriced_model_is_never_zero_priced() {
        let prices = PriceTable::embedded();
        let rows = vec![
            row(0, "claude-opus-5-5", "cli", TokenKind::Output, 1000),
            row(0, "claude-unreleased-9", "cli", TokenKind::Output, 500),
        ];
        let c = month_cost(&rows, &[], &[], &prices, None, &[], true);
        assert!((c.claude.usd - 1000.0 * 2e-5).abs() < 1e-12);
        assert_eq!(c.claude.tokens, 1500);
        assert_eq!(c.claude.unpriced, 500);
        assert!(c.unpriced_models.contains("claude-unreleased-9"));
    }

    #[test]
    fn excluded_entrypoints_filter_at_query_time() {
        let prices = PriceTable::embedded();
        let rows = vec![
            row(0, "claude-opus-5-5", "cli", TokenKind::Output, 1000),
            row(0, "claude-opus-5-5", "sdk-ts", TokenKind::Output, 3000),
        ];
        let c = month_cost(&rows, &[], &[], &prices, None, &["sdk-ts".into()], true);
        assert_eq!(c.claude.tokens, 1000);
        assert!(!c.by_source.contains_key("sdk-ts"));
    }

    fn settings(claude: Option<PlanSetting>, codex: Option<PlanSetting>) -> Settings {
        Settings {
            currency: Currency::Eur,
            plans: Plans { claude, codex },
            ..Settings::default()
        }
    }

    fn eur(amount: f64) -> Option<PlanSetting> {
        Some(PlanSetting {
            amount,
            currency: PlanCurrency::Eur,
            period: PlanPeriod::Month,
        })
    }

    fn cost(claude_usd: f64, codex_usd: f64) -> MonthCost {
        MonthCost {
            claude: ProviderTotals {
                usd: claude_usd,
                tokens: 1,
                unpriced: 0,
            },
            codex: ProviderTotals {
                usd: codex_usd,
                tokens: 1,
                unpriced: 0,
            },
            ..MonthCost::default()
        }
    }

    #[test]
    fn multiple_and_plans() {
        let fx = FxDay::fallback();
        let conv = fx.eur_per_usd();
        let s = settings(eur(100.0), eur(50.0));
        let c = cost(300.0 / conv, 75.0 / conv); // 300 € and 75 €
        let (scores, total) = scores_and_total(&c, ScoreState::Ok, ScoreState::Estimate, &s, &fx);
        assert!((scores[0].multiple.unwrap() - 3.0).abs() < 1e-9);
        assert!((scores[1].multiple.unwrap() - 1.5).abs() < 1e-9);
        assert!((total.display - 375.0).abs() < 1e-9);
        assert!((total.plan_monthly_display.unwrap() - 150.0).abs() < 1e-9);
        assert!((total.multiple.unwrap() - 2.5).abs() < 1e-9);
        assert_eq!(total.excluded_note, None);
    }

    #[test]
    fn codex_without_price_drops_its_plan() {
        let fx = FxDay::fallback();
        let conv = fx.eur_per_usd();
        let s = settings(eur(100.0), eur(50.0));
        let c = cost(300.0 / conv, 0.0);
        let (scores, total) = scores_and_total(&c, ScoreState::Ok, ScoreState::Unknown, &s, &fx);
        assert_eq!(scores[1].multiple, None);
        assert!((total.multiple.unwrap() - 3.0).abs() < 1e-9, "Codex plan must leave the denominator");
        assert!((total.plan_monthly_display.unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(total.excluded_note.as_deref(), Some("Codex bez ceny, paušál nezapočten"));
    }

    #[test]
    fn codex_estimate_switched_off() {
        let fx = FxDay::fallback();
        let conv = fx.eur_per_usd();
        let mut s = settings(eur(100.0), eur(50.0));
        s.codex_estimate_in_score = false;
        let c = cost(100.0 / conv, 75.0 / conv);
        let (scores, total) = scores_and_total(&c, ScoreState::Ok, ScoreState::Estimate, &s, &fx);
        assert_eq!(scores[1].state, ScoreState::Estimate);
        assert!((total.display - 100.0).abs() < 1e-9);
        assert!((total.multiple.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(total.excluded_note.as_deref(), Some("Odhad Codexu vypnutý, paušál nezapočten"));
    }

    #[test]
    fn yearly_plan_in_other_currency() {
        let fx = FxDay::fallback();
        let s = Settings {
            currency: Currency::Czk,
            ..Settings::default()
        };
        let plan = PlanSetting {
            amount: 1200.0,
            currency: PlanCurrency::Usd,
            period: PlanPeriod::Year,
        };
        let v = plan_monthly_display(&plan, &s, &fx);
        assert!((v - 100.0 * 21.834).abs() < 1e-9);
    }

    #[test]
    fn no_plan_no_multiple() {
        let fx = FxDay::fallback();
        let (scores, total) =
            scores_and_total(&cost(10.0, 1.0), ScoreState::Ok, ScoreState::Estimate, &Settings::default(), &fx);
        assert_eq!(scores[0].multiple, None);
        assert_eq!(total.multiple, None);
        assert_eq!(total.plan_monthly_display, None);
        assert_eq!(total.excluded_note, None);
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn streak_counting() {
        let mut days = BTreeMap::new();
        for s in ["2026-10-01", "2026-10-02", "2026-10-03", "2026-10-10", "2026-10-11"] {
            days.insert(d(s), 5);
        }
        // Today (12th) has no activity yet: the streak ending yesterday counts.
        assert_eq!(streaks(&days, d("2026-10-12")), (2, 3, Some(d("2026-10-10"))));
        days.insert(d("2026-10-12"), 1);
        assert_eq!(streaks(&days, d("2026-10-12")), (3, 3, Some(d("2026-10-10"))));
        assert_eq!(streaks(&days, d("2026-10-20")).0, 0);
    }

    #[test]
    fn milestone_ids() {
        let mut claude = BTreeMap::new();
        claude.insert(d("2026-10-01"), 100u64);
        claude.insert(d("2026-10-02"), 150u64);
        let c = milestone_candidates(
            "2026-10",
            Some(2.4),
            &[(Provider::Claude, &claude)],
            &[(Provider::Codex, 8, Some(d("2026-09-25")))],
            d("2026-10-02"),
        );
        let ids: Vec<&str> = c.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "multiple:2026-10:1",
                "multiple:2026-10:2",
                "dailymax:claude:2026-10-02",
                "streak:codex:7:2026-09-25"
            ]
        );
    }
}
