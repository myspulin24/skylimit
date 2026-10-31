//! History tab: consumption over a chosen period with the month's breakdowns
//! (model, day, source, project), the split by token type and what the cache
//! saved. Like the month it is priced at query time from stored tokens, so a
//! new price list reprices the whole history. One query per view, answered
//! on the core thread; nothing here goes into the widget's snapshot.
//!
//! ```text
//!  range ─▶ period (24 h by hours, else Prague days up to today)
//!        ─▶ usage_hourly + web_search_hourly rows ─▶ price per kind
//!        ─▶ codex_daily days (day buckets only) × blended rate
//!        ─▶ series, kinds, breakdowns, cache savings
//! ```

use std::collections::HashMap;

use chrono::{Days, NaiveDate};

use crate::model::{
    Currency, History, HistoryBucket, HistoryFilter, HistoryKind, HistoryKindRow, HistoryPoint, HistoryProvider,
    HistoryRange, HistoryRow, Provider,
};
use crate::pricing::{normalize_model, PriceTable, TokenKind};
use crate::score::{local_midnight, prague_date};
use crate::store::{UsageRow, WebRow};

/// The key of Codex's single row in every breakdown.
pub(crate) const CODEX_KEY: &str = "codex";

/// Buckets of a period. `query_end` is exclusive and covers the current hour.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Period {
    pub bucket: HistoryBucket,
    pub from: i64,
    pub query_end: i64,
    /// Day buckets: every Prague date from the first to today.
    pub days: Vec<NaiveDate>,
}

impl Period {
    pub(crate) fn new(range: HistoryRange, now: i64, first_data_at: Option<i64>) -> Period {
        let hour = now.div_euclid(3600) * 3600;
        let query_end = hour + 3600;
        let today = prague_date(now);
        let back = match range {
            HistoryRange::Day => {
                return Period {
                    bucket: HistoryBucket::Hour,
                    from: hour - 23 * 3600,
                    query_end,
                    days: Vec::new(),
                }
            }
            HistoryRange::Week => 6,
            HistoryRange::Month => 29,
            HistoryRange::Quarter => 89,
            HistoryRange::All => first_data_at
                .map(|t| (today - prague_date(t)).num_days().max(0) as u64)
                .unwrap_or(0),
        };
        let first = today.checked_sub_days(Days::new(back)).unwrap_or(today);
        let mut days = Vec::new();
        let mut d = first;
        while d <= today {
            days.push(d);
            match d.checked_add_days(Days::new(1)) {
                Some(n) => d = n,
                None => break,
            }
        }
        Period {
            bucket: HistoryBucket::Day,
            from: local_midnight(first),
            query_end,
            days,
        }
    }

    fn len(&self) -> usize {
        match self.bucket {
            HistoryBucket::Hour => 24,
            HistoryBucket::Day => self.days.len(),
        }
    }

    fn index_of_hour(&self, hour: i64) -> Option<usize> {
        if hour < self.from || hour >= self.query_end {
            return None;
        }
        match self.bucket {
            HistoryBucket::Hour => Some(((hour - self.from) / 3600) as usize).filter(|i| *i < 24),
            HistoryBucket::Day => self.index_of_day(prague_date(hour)),
        }
    }

    fn index_of_day(&self, d: NaiveDate) -> Option<usize> {
        let first = *self.days.first()?;
        let i = (d - first).num_days();
        (i >= 0 && (i as usize) < self.days.len()).then_some(i as usize)
    }
}

pub(crate) struct Inputs<'a> {
    pub rows: &'a [UsageRow],
    pub web: &'a [WebRow],
    /// Codex daily totals (`YYYY-MM-DD`, tokens), any dates.
    pub codex_days: &'a [(String, u64)],
    pub prices: &'a PriceTable,
    pub codex_rate: Option<f64>,
    /// Codex switched off: its old days stay out like in the month.
    pub codex_enabled: bool,
    /// USD → display currency.
    pub conv: f64,
    pub currency: Currency,
    pub first_data_at: Option<i64>,
}

#[derive(Default)]
struct Acc {
    usd: f64,
    tokens: u64,
}

impl Acc {
    fn add(&mut self, usd: f64, tokens: u64) {
        self.usd += usd;
        self.tokens += tokens;
    }
}

fn bump(map: &mut HashMap<String, Acc>, key: &str, usd: f64, tokens: u64) {
    map.entry(key.to_string()).or_default().add(usd, tokens);
}

fn kind_of(k: TokenKind) -> HistoryKind {
    match k {
        TokenKind::Input => HistoryKind::Input,
        TokenKind::Output => HistoryKind::Output,
        TokenKind::CacheRead => HistoryKind::CacheRead,
        TokenKind::CacheWrite5m | TokenKind::CacheWrite1h => HistoryKind::CacheWrite,
    }
}

const KIND_ORDER: [HistoryKind; 6] = [
    HistoryKind::Input,
    HistoryKind::CacheRead,
    HistoryKind::CacheWrite,
    HistoryKind::Output,
    HistoryKind::WebSearch,
    HistoryKind::Codex,
];

pub(crate) fn build(range: HistoryRange, filter: HistoryFilter, now: i64, inp: &Inputs<'_>) -> History {
    let period = Period::new(range, now, inp.first_data_at);
    let show_claude = filter != HistoryFilter::Codex;
    let show_codex = filter != HistoryFilter::Claude && inp.codex_enabled;
    let codex_in_view = show_codex && period.bucket == HistoryBucket::Day;

    let n = period.len();
    let mut claude_pts: Vec<Acc> = (0..n).map(|_| Acc::default()).collect();
    let mut codex_pts: Vec<Acc> = (0..n).map(|_| Acc::default()).collect();
    let mut claude = Acc::default();
    let mut claude_unpriced = 0u64;
    let mut codex = Acc::default();
    let mut codex_unpriced = 0u64;
    let mut kinds: HashMap<HistoryKind, Acc> = HashMap::new();
    let mut savings = 0.0;
    let mut by_model = HashMap::new();
    let mut by_source = HashMap::new();
    let mut by_project = HashMap::new();
    let mut by_day = HashMap::new();

    if show_claude {
        for r in inp.rows {
            let Some(i) = period.index_of_hour(r.hour) else { continue };
            let price = inp.prices.lookup(&r.model);
            let unit = |k: TokenKind| price.and_then(|p| p.unit_price(k, r.band, &r.speed, &r.service_tier));
            let usd = match unit(r.kind) {
                Some(u) => u * r.tokens as f64,
                None => {
                    claude_unpriced += r.tokens;
                    0.0
                }
            };
            if r.kind == TokenKind::CacheRead {
                if let (Some(full), Some(cached)) = (unit(TokenKind::Input), unit(TokenKind::CacheRead)) {
                    savings += (full - cached).max(0.0) * r.tokens as f64;
                }
            }
            claude.add(usd, r.tokens);
            claude_pts[i].add(usd, r.tokens);
            kinds.entry(kind_of(r.kind)).or_default().add(usd, r.tokens);
            bump(&mut by_model, &normalize_model(&r.model), usd, r.tokens);
            bump(&mut by_source, &r.entrypoint, usd, r.tokens);
            bump(&mut by_project, &r.project, usd, r.tokens);
            bump(&mut by_day, &prague_date(r.hour).format("%Y-%m-%d").to_string(), usd, r.tokens);
        }
        for w in inp.web {
            let Some(i) = period.index_of_hour(w.hour) else { continue };
            let Some(price) = inp.prices.lookup(&w.model).and_then(|p| p.web_search_price()) else {
                continue;
            };
            let usd = price * w.requests as f64;
            claude.add(usd, 0);
            claude_pts[i].add(usd, 0);
            kinds.entry(HistoryKind::WebSearch).or_default().add(usd, 0);
            bump(&mut by_model, &normalize_model(&w.model), usd, 0);
            bump(&mut by_source, &w.entrypoint, usd, 0);
            bump(&mut by_project, &w.project, usd, 0);
            bump(&mut by_day, &prague_date(w.hour).format("%Y-%m-%d").to_string(), usd, 0);
        }
    }

    if codex_in_view {
        for (date, tokens) in inp.codex_days {
            let Ok(d) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else { continue };
            let Some(i) = period.index_of_day(d) else { continue };
            let usd = match inp.codex_rate {
                Some(rate) => rate * *tokens as f64,
                None => {
                    codex_unpriced += tokens;
                    0.0
                }
            };
            codex.add(usd, *tokens);
            codex_pts[i].add(usd, *tokens);
            kinds.entry(HistoryKind::Codex).or_default().add(usd, *tokens);
            for map in [&mut by_model, &mut by_source, &mut by_project] {
                bump(map, CODEX_KEY, usd, *tokens);
            }
            bump(&mut by_day, date, usd, *tokens);
        }
    }

    let conv = inp.conv;
    let series: Vec<HistoryPoint> = (0..n)
        .map(|i| {
            let (start, date) = match period.bucket {
                HistoryBucket::Hour => (period.from + i as i64 * 3600, None),
                HistoryBucket::Day => {
                    let d = period.days[i];
                    (local_midnight(d), Some(d.format("%Y-%m-%d").to_string()))
                }
            };
            HistoryPoint {
                start,
                date,
                claude_display: claude_pts[i].usd * conv,
                codex_display: codex_pts[i].usd * conv,
                claude_tokens: claude_pts[i].tokens,
                codex_tokens: codex_pts[i].tokens,
            }
        })
        .collect();
    let active_buckets = series.iter().filter(|p| p.claude_tokens + p.codex_tokens > 0).count() as u32;

    let mut providers = Vec::new();
    if show_claude {
        providers.push(HistoryProvider {
            provider: Provider::Claude,
            display: claude.usd * conv,
            tokens: claude.tokens,
            unpriced_tokens: claude_unpriced,
            estimate: false,
        });
    }
    if show_codex {
        providers.push(HistoryProvider {
            provider: Provider::Codex,
            display: codex.usd * conv,
            tokens: codex.tokens,
            unpriced_tokens: codex_unpriced,
            estimate: inp.codex_rate.is_some(),
        });
    }

    let kinds = KIND_ORDER
        .iter()
        .filter_map(|k| {
            let a = kinds.get(k)?;
            Some(HistoryKindRow {
                kind: *k,
                tokens: a.tokens,
                display: a.usd * conv,
            })
        })
        .collect();

    let rows = |map: HashMap<String, Acc>, codex_row: bool| -> Vec<HistoryRow> {
        let mut out: Vec<HistoryRow> = map
            .into_iter()
            .map(|(key, a)| HistoryRow {
                provider: Some(if codex_row && key == CODEX_KEY {
                    Provider::Codex
                } else {
                    Provider::Claude
                }),
                key,
                display: a.usd * conv,
                tokens: a.tokens,
            })
            .collect();
        out.sort_by(|a, b| {
            b.display
                .total_cmp(&a.display)
                .then(b.tokens.cmp(&a.tokens))
                .then(a.key.cmp(&b.key))
        });
        out
    };
    let mut by_day: Vec<HistoryRow> = by_day
        .into_iter()
        .map(|(key, a)| HistoryRow {
            key,
            provider: None,
            display: a.usd * conv,
            tokens: a.tokens,
        })
        .collect();
    by_day.sort_by(|a, b| b.key.cmp(&a.key));

    History {
        range,
        filter,
        currency: inp.currency,
        bucket: period.bucket,
        from: period.from,
        to: now,
        first_data_at: inp.first_data_at,
        active_buckets,
        total_display: (claude.usd + codex.usd) * conv,
        total_tokens: claude.tokens + codex.tokens,
        providers,
        kinds,
        cache_savings_display: savings * conv,
        series,
        by_model: rows(by_model, true),
        by_day,
        by_source: rows(by_source, true),
        by_project: rows(by_project, true),
        codex_without_hours: show_codex && period.bucket == HistoryBucket::Hour,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-31 12:30 Prague (CET, UTC+1).
    const NOW: i64 = 1_793_446_200;

    fn row(hour: i64, model: &str, entrypoint: &str, project: &str, kind: TokenKind, tokens: u64) -> UsageRow {
        UsageRow {
            hour,
            provider: "claude".into(),
            model: model.into(),
            entrypoint: entrypoint.into(),
            project: project.into(),
            speed: "standard".into(),
            service_tier: "standard".into(),
            band: 0,
            kind,
            tokens,
        }
    }

    fn inputs<'a>(rows: &'a [UsageRow], codex_days: &'a [(String, u64)], prices: &'a PriceTable) -> Inputs<'a> {
        Inputs {
            rows,
            web: &[],
            codex_days,
            prices,
            codex_rate: Some(1e-6),
            codex_enabled: true,
            conv: 1.0,
            currency: Currency::Eur,
            first_data_at: rows.iter().map(|r| r.hour).min(),
        }
    }

    fn unit(prices: &PriceTable, kind: TokenKind) -> f64 {
        prices.lookup("claude-opus-5-5").unwrap().unit_price(kind, 0, "standard", "standard").unwrap()
    }

    #[test]
    fn periods_by_hours_and_prague_days() {
        let day = Period::new(HistoryRange::Day, NOW, None);
        assert_eq!(day.bucket, HistoryBucket::Hour);
        assert_eq!(day.from, NOW.div_euclid(3600) * 3600 - 23 * 3600);
        assert_eq!(day.query_end - day.from, 24 * 3600);

        let week = Period::new(HistoryRange::Week, NOW, None);
        assert_eq!(week.days.len(), 7);
        assert_eq!(week.days.first().unwrap().to_string(), "2026-10-25");
        // 25 October is still summer time: midnight is 22:00 UTC the day before.
        assert_eq!(week.from, 1_792_879_200);

        let all = Period::new(HistoryRange::All, NOW, Some(NOW - 40 * 86_400));
        assert_eq!(all.days.len(), 41);
        assert_eq!(Period::new(HistoryRange::All, NOW, None).days.len(), 1);
        assert_eq!(Period::new(HistoryRange::Quarter, NOW, None).days.len(), 90);
    }

    #[test]
    fn prices_kinds_breakdowns_and_cache_savings() {
        let prices = PriceTable::embedded();
        let today = NOW.div_euclid(3600) * 3600;
        let rows = vec![
            row(today, "claude-opus-5-5", "sdk-ts", "alpha", TokenKind::CacheRead, 1_000_000),
            row(today, "claude-opus-5-5", "sdk-ts", "alpha", TokenKind::Output, 10_000),
            row(today - 2 * 86_400, "claude-opus-5-5", "cli", "beta", TokenKind::Input, 2_000),
            row(today - 2 * 86_400, "claude-opus-5-5", "cli", "beta", TokenKind::CacheWrite1h, 5_000),
            row(today, "claude-unreleased-9", "cli", "beta", TokenKind::Output, 700),
            // Outside the week: ignored.
            row(today - 20 * 86_400, "claude-opus-5-5", "cli", "beta", TokenKind::Output, 99_000),
        ];
        let codex = vec![("2026-10-30".to_string(), 3_000_000), ("2026-09-01".to_string(), 1)];
        let h = build(HistoryRange::Week, HistoryFilter::All, NOW, &inputs(&rows, &codex, &prices));

        let read = unit(&prices, TokenKind::CacheRead) * 1e6;
        let out = unit(&prices, TokenKind::Output) * 1e4;
        let input = unit(&prices, TokenKind::Input) * 2e3;
        let write = unit(&prices, TokenKind::CacheWrite1h) * 5e3;
        let claude_usd = read + out + input + write;
        assert!((h.providers[0].display - claude_usd).abs() < 1e-9);
        assert_eq!(h.providers[0].tokens, 1_000_000 + 10_000 + 2_000 + 5_000 + 700);
        assert_eq!(h.providers[0].unpriced_tokens, 700);
        assert!((h.providers[1].display - 3.0).abs() < 1e-9);
        assert!(h.providers[1].estimate);
        assert!((h.total_display - claude_usd - 3.0).abs() < 1e-9);

        let kinds: Vec<HistoryKind> = h.kinds.iter().map(|k| k.kind).collect();
        assert_eq!(
            kinds,
            [HistoryKind::Input, HistoryKind::CacheRead, HistoryKind::CacheWrite, HistoryKind::Output, HistoryKind::Codex]
        );
        let saved = (unit(&prices, TokenKind::Input) - unit(&prices, TokenKind::CacheRead)) * 1e6;
        assert!((h.cache_savings_display - saved).abs() < 1e-9);

        assert_eq!(h.series.len(), 7);
        assert_eq!(h.series[6].date.as_deref(), Some("2026-10-31"));
        assert_eq!(h.series[5].codex_tokens, 3_000_000);
        assert_eq!(h.series[4].claude_tokens, 7_000);
        assert_eq!(h.active_buckets, 3);

        assert_eq!(h.by_day.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["2026-10-31", "2026-10-30", "2026-10-29"]);
        let codex_row = h.by_model.iter().find(|r| r.key == CODEX_KEY).unwrap();
        assert_eq!(codex_row.provider, Some(Provider::Codex));
        assert!(h.by_project.iter().any(|r| r.key == "beta"));
        assert!(h.by_source.iter().any(|r| r.key == "sdk-ts"));
        let sum: f64 = h.by_model.iter().map(|r| r.display).sum();
        assert!((sum - h.total_display).abs() < 1e-9, "breakdown adds up to the total");
    }

    #[test]
    fn filters_and_the_hourly_view() {
        let prices = PriceTable::embedded();
        let hour = NOW.div_euclid(3600) * 3600;
        let rows = vec![
            row(hour, "claude-opus-5-5", "cli", "alpha", TokenKind::Output, 100),
            row(hour - 23 * 3600, "claude-opus-5-5", "cli", "alpha", TokenKind::Output, 50),
            row(hour - 24 * 3600, "claude-opus-5-5", "cli", "alpha", TokenKind::Output, 25),
        ];
        let codex = vec![("2026-10-31".to_string(), 1_000)];

        let day = build(HistoryRange::Day, HistoryFilter::All, NOW, &inputs(&rows, &codex, &prices));
        assert_eq!(day.bucket, HistoryBucket::Hour);
        assert_eq!(day.series.len(), 24);
        assert_eq!(day.series[23].claude_tokens, 100);
        assert_eq!(day.series[0].claude_tokens, 50);
        assert_eq!(day.total_tokens, 150, "Codex has no hours and the 25th hour is out");
        assert!(day.codex_without_hours);

        let only_codex = build(HistoryRange::Week, HistoryFilter::Codex, NOW, &inputs(&rows, &codex, &prices));
        assert_eq!(only_codex.providers.len(), 1);
        assert_eq!(only_codex.total_tokens, 1_000);
        assert_eq!(only_codex.kinds.len(), 1);
        assert!(!only_codex.codex_without_hours);

        let only_claude = build(HistoryRange::Week, HistoryFilter::Claude, NOW, &inputs(&rows, &codex, &prices));
        assert_eq!(only_claude.total_tokens, 175);
        assert!(only_claude.by_model.iter().all(|r| r.provider == Some(Provider::Claude)));

        let mut off = inputs(&rows, &codex, &prices);
        off.codex_enabled = false;
        let h = build(HistoryRange::Week, HistoryFilter::All, NOW, &off);
        assert_eq!(h.providers.len(), 1, "Codex switched off stays out");
    }
}
