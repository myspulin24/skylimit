//! The data contract between the core and the UI.
//!
//! Mirrors `docs/snapshot-contract.md` field for field (TypeScript twin:
//! `src/types.ts`). Times are Unix epoch seconds, `*Usd` fields are US
//! dollars, `display*` fields are already in the display currency.
//! Nothing in here may ever carry a token, a header or message text.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}

/// Freshness of one limit window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowState {
    Known,
    Stale,
    Unknown,
    AfterReset,
}

/// Governing state of a provider's limits. `Off` = deliberately disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitsState {
    Known,
    Stale,
    Unknown,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowKind {
    #[serde(rename = "5h")]
    FiveHour,
    #[serde(rename = "week")]
    Week,
    #[serde(rename = "weekModel")]
    WeekModel,
    #[serde(rename = "other")]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub kind: WindowKind,
    pub label: String,
    pub percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub fetched_at: Option<i64>,
    pub state: WindowState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLimits {
    pub provider: Provider,
    pub state: LimitsState,
    pub governing: Option<f64>,
    pub windows: Vec<LimitWindow>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScoreState {
    Ok,
    Estimate,
    Partial,
    Unknown,
    FormatChanged,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderScore {
    pub provider: Provider,
    pub usd: f64,
    pub display: f64,
    pub tokens: u64,
    pub unpriced_tokens: u64,
    pub state: ScoreState,
    pub plan_monthly_display: Option<f64>,
    pub multiple: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mood {
    Confused,
    Dozing,
    Panic,
    Sweat,
    Celebrate,
    Focus,
    Sleep,
    Calm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownRow {
    pub key: String,
    pub usd: f64,
    pub display: f64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Currency {
    #[serde(rename = "EUR")]
    Eur,
    #[serde(rename = "CZK")]
    Czk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Total {
    pub display: f64,
    pub multiple: Option<f64>,
    pub plan_monthly_display: Option<f64>,
    pub excluded_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMood {
    pub provider: Provider,
    pub mood: Mood,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tray {
    Green,
    Orange,
    Red,
    GreenDim,
    OrangeDim,
    RedDim,
    Grey,
    Mono,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyRow {
    pub date: String,
    pub claude_display: f64,
    pub codex_display: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Streak {
    pub provider: Provider,
    pub current: u32,
    pub longest: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backfill {
    pub done: bool,
    pub files_done: u32,
    pub files_total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CelebrationWho {
    Claude,
    Codex,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Celebration {
    pub id: String,
    pub who: CelebrationWho,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub generated_at: i64,
    pub month: String,
    pub currency: Currency,
    pub limits: Vec<ProviderLimits>,
    pub scores: Vec<ProviderScore>,
    pub total: Total,
    pub moods: Vec<ProviderMood>,
    pub tray: Tray,
    pub daily: Vec<DailyRow>,
    pub by_source: Vec<BreakdownRow>,
    pub by_model: Vec<BreakdownRow>,
    pub by_project: Vec<BreakdownRow>,
    pub streaks: Vec<Streak>,
    pub backfill: Backfill,
    pub celebration: Option<Celebration>,
    pub updated_ago_sec: i64,
    /// Widget background transparency in percent, from settings (display only).
    pub widget_transparency: u8,
    /// The widget's score for the period and amount from Settings › Widget.
    pub widget: WidgetScore,
}

/// Settings › Widget: the period of the widget's score. `Month` is the
/// calendar month of `scores` and `total`; the rest end now, like the
/// History tab's periods.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WidgetPeriod {
    #[default]
    #[serde(rename = "month")]
    Month,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Days30,
    #[serde(rename = "90d")]
    Days90,
    #[serde(rename = "all")]
    All,
}

/// Settings › Widget: the score in API prices, or with what the cache saved
/// added on top (what the same work would cost at API prices without it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetAmount {
    #[default]
    Api,
    WithCacheSavings,
}

/// What the widget's score block shows. For the month in API prices it
/// equals `scores` and `total`; otherwise the same fields for the chosen
/// period, with the plan pro-rated to the period's days
/// (`planMonthlyDisplay` then holds the plan for the period).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetScore {
    pub period: WidgetPeriod,
    pub amount: WidgetAmount,
    /// Providers shown in the widget (Codex is missing from 24 h: it reports days).
    pub scores: Vec<ProviderScore>,
    pub total: Total,
    /// Trend for the sparkline per hour (24 h) or day, display currency.
    pub spark: Vec<f64>,
}

impl Snapshot {
    /// Snapshot shown before the core has computed anything.
    pub fn empty(now: i64, currency: Currency) -> Self {
        Snapshot {
            generated_at: now,
            month: String::new(),
            currency,
            limits: Vec::new(),
            scores: Vec::new(),
            total: Total {
                display: 0.0,
                multiple: None,
                plan_monthly_display: None,
                excluded_note: None,
            },
            moods: Vec::new(),
            tray: Tray::Grey,
            daily: Vec::new(),
            by_source: Vec::new(),
            by_model: Vec::new(),
            by_project: Vec::new(),
            streaks: Vec::new(),
            backfill: Backfill {
                done: false,
                files_done: 0,
                files_total: 0,
            },
            celebration: None,
            updated_ago_sec: 0,
            widget_transparency: 0,
            widget: WidgetScore {
                period: WidgetPeriod::Month,
                amount: WidgetAmount::Api,
                scores: Vec::new(),
                total: Total {
                    display: 0.0,
                    multiple: None,
                    plan_monthly_display: None,
                    excluded_note: None,
                },
                spark: Vec::new(),
            },
        }
    }

    pub fn limits_for(&self, provider: Provider) -> Option<&ProviderLimits> {
        self.limits.iter().find(|l| l.provider == provider)
    }

    pub fn score_for(&self, provider: Provider) -> Option<&ProviderScore> {
        self.scores.iter().find(|s| s.provider == provider)
    }

    pub fn mood_for(&self, provider: Provider) -> Option<Mood> {
        self.moods
            .iter()
            .find(|m| m.provider == provider)
            .map(|m| m.mood)
    }
}

/// The Claude account Claude Code is signed in with (Settings › Účty), from
/// `oauthAccount` in Claude Code's global config (`claude_config.rs`). Enum
/// values stay raw (`claude_team`, `default_claude_max_5x`, …); the UI
/// labels them. Not part of the snapshot: it never reaches the widget.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAccount {
    pub email: Option<String>,
    pub name: Option<String>,
    pub organization: Option<String>,
    pub organization_type: Option<String>,
    pub role: Option<String>,
    pub seat_tier: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub billing: Option<String>,
    pub account_created_at: Option<i64>,
    pub subscription_created_at: Option<i64>,
    /// Extra usage (paid past the plan's limits) switched on for the account.
    pub extra_usage: Option<bool>,
    /// Why extra usage cannot be used right now (`out_of_credits`, …).
    pub extra_usage_blocked: Option<String>,
    /// When Claude Code last refreshed the profile and the limits.
    pub profile_fetched_at: Option<i64>,
    pub limits_fetched_at: Option<i64>,
}

/// Prepaid Codex credits (`credits` of `account/rateLimits/read`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

/// The account the Codex CLI is signed in with, from the app-server
/// (`account/read`, plus what the limits and usage reads add).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccount {
    /// `chatgpt`, `apiKey`, `amazonBedrock`, `gcp`, or `none` when signed out.
    pub kind: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub workspace: Option<String>,
    pub cli_version: Option<String>,
    /// Limit resets the account can still use (`rateLimitResetCredits`).
    pub reset_credits: Option<u32>,
    pub credits: Option<CodexCredits>,
    pub lifetime_tokens: Option<u64>,
    pub read_at: i64,
}

/// Settings › Účty: who Skylimit reads for each provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accounts {
    pub claude: Option<ClaudeAccount>,
    pub codex: Option<CodexAccount>,
    /// Why there is no Codex account (CLI missing, app-server down, …).
    pub codex_note: Option<String>,
}

/// History tab: the period it shows. 24 h goes by hours, the rest by
/// calendar days in Europe/Prague, today included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryRange {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "90d")]
    Quarter,
    /// From the first day with any data.
    #[serde(rename = "all")]
    All,
}

/// History tab: which provider it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryFilter {
    All,
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryBucket {
    Hour,
    Day,
}

/// A part of the "by type" bar. Codex reports daily totals only, so its
/// tokens are a part of their own; web searches are priced per request and
/// carry no tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryKind {
    Input,
    CacheRead,
    CacheWrite,
    Output,
    WebSearch,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryKindRow {
    pub kind: HistoryKind,
    pub tokens: u64,
    pub display: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryProvider {
    pub provider: Provider,
    pub display: f64,
    pub tokens: u64,
    /// Tokens of models without a price (never priced as 0).
    pub unpriced_tokens: u64,
    /// Codex: daily totals × blended rate.
    pub estimate: bool,
}

/// One bar of the chart: an hour (24 h) or a day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    /// Epoch s of the bucket's start (local midnight for days).
    pub start: i64,
    /// `YYYY-MM-DD` for day buckets.
    pub date: Option<String>,
    pub claude_display: f64,
    pub codex_display: f64,
    pub claude_tokens: u64,
    pub codex_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    /// Model, entrypoint, project folder or `YYYY-MM-DD`; `codex` for
    /// Codex's one row (it has no models, sources or projects).
    pub key: String,
    /// `None` for days, which mix both.
    pub provider: Option<Provider>,
    pub display: f64,
    pub tokens: u64,
}

/// History tab (command `history`): consumption over a period with the same
/// breakdowns as the month, priced now. Not part of the snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub range: HistoryRange,
    pub filter: HistoryFilter,
    pub currency: Currency,
    pub bucket: HistoryBucket,
    /// Start of the first bucket and the time of the query (epoch s).
    pub from: i64,
    pub to: i64,
    /// Oldest record of the shown providers: how far back history reaches.
    pub first_data_at: Option<i64>,
    /// Buckets with any tokens (active days, or hours in the 24 h view).
    pub active_buckets: u32,
    pub total_display: f64,
    pub total_tokens: u64,
    pub providers: Vec<HistoryProvider>,
    pub kinds: Vec<HistoryKindRow>,
    /// What reading from the cache saved against the full input price.
    pub cache_savings_display: f64,
    pub series: Vec<HistoryPoint>,
    pub by_model: Vec<HistoryRow>,
    /// Newest first.
    pub by_day: Vec<HistoryRow>,
    pub by_source: Vec<HistoryRow>,
    pub by_project: Vec<HistoryRow>,
    /// 24 h view with Codex on: Codex reports days, not hours.
    pub codex_without_hours: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanCurrency {
    #[serde(rename = "EUR")]
    Eur,
    #[serde(rename = "CZK")]
    Czk,
    #[serde(rename = "USD")]
    Usd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanPeriod {
    Month,
    Year,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSetting {
    pub amount: f64,
    pub currency: PlanCurrency,
    pub period: PlanPeriod,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plans {
    pub claude: Option<PlanSetting>,
    pub codex: Option<PlanSetting>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub currency: Currency,
    pub plans: Plans,
    pub codex_estimate_in_score: bool,
    pub excluded_entrypoints: Vec<String>,
    pub click_through: bool,
    /// Widget background transparency in percent, 0..=WIDGET_TRANSPARENCY_MAX.
    pub widget_transparency: u8,
    /// Look for a new version on GitHub after start and every 6 hours.
    pub check_updates: bool,
    /// The period of the widget's score.
    pub widget_period: WidgetPeriod,
    /// The widget's score in API prices or with the cache savings.
    pub widget_amount: WidgetAmount,
}

/// Above this the panel would vanish and the text lose its backdrop.
pub const WIDGET_TRANSPARENCY_MAX: u8 = 90;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            currency: Currency::Eur,
            plans: Plans::default(),
            codex_estimate_in_score: true,
            excluded_entrypoints: Vec::new(),
            click_through: false,
            widget_transparency: 0,
            check_updates: true,
            widget_period: WidgetPeriod::Month,
            widget_amount: WidgetAmount::Api,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_default_matches_contract() {
        let value = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(
            value,
            json!({
                "currency": "EUR",
                "plans": { "claude": null, "codex": null },
                "codexEstimateInScore": true,
                "excludedEntrypoints": [],
                "clickThrough": false,
                "widgetTransparency": 0,
                "checkUpdates": true,
                "widgetPeriod": "month",
                "widgetAmount": "api"
            })
        );
    }

    #[test]
    fn snapshot_uses_contract_field_names() {
        let mut snap = Snapshot::empty(10, Currency::Czk);
        snap.limits.push(ProviderLimits {
            provider: Provider::Codex,
            state: LimitsState::Stale,
            governing: Some(40.0),
            windows: vec![LimitWindow {
                kind: WindowKind::FiveHour,
                label: "5h".into(),
                percent: None,
                resets_at: Some(5),
                fetched_at: Some(1),
                state: WindowState::AfterReset,
            }],
            note: None,
        });
        snap.scores.push(ProviderScore {
            provider: Provider::Claude,
            usd: 1.0,
            display: 2.0,
            tokens: 3,
            unpriced_tokens: 4,
            state: ScoreState::FormatChanged,
            plan_monthly_display: None,
            multiple: None,
        });
        snap.celebration = Some(Celebration {
            id: "x".into(),
            who: CelebrationWho::Both,
            text: "t".into(),
        });
        snap.tray = Tray::OrangeDim;
        let v = serde_json::to_value(&snap).unwrap();
        for key in [
            "generatedAt",
            "month",
            "currency",
            "limits",
            "scores",
            "total",
            "moods",
            "tray",
            "daily",
            "bySource",
            "byModel",
            "byProject",
            "streaks",
            "backfill",
            "celebration",
            "updatedAgoSec",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["currency"], "CZK");
        assert_eq!(v["tray"], "orangeDim");
        assert_eq!(v["limits"][0]["windows"][0]["kind"], "5h");
        assert_eq!(v["limits"][0]["windows"][0]["state"], "afterReset");
        assert_eq!(v["limits"][0]["windows"][0]["resetsAt"], 5);
        assert_eq!(v["limits"][0]["windows"][0]["fetchedAt"], 1);
        assert_eq!(v["scores"][0]["state"], "formatChanged");
        assert_eq!(v["scores"][0]["unpricedTokens"], 4);
        assert!(v["scores"][0]["planMonthlyDisplay"].is_null());
        assert!(v["total"].get("excludedNote").is_some());
        assert!(v["total"].get("planMonthlyDisplay").is_some());
        assert_eq!(v["backfill"]["filesDone"], 0);
        assert_eq!(v["backfill"]["filesTotal"], 0);
        assert_eq!(v["celebration"]["who"], "both");
        // Round trip.
        let back: Snapshot = serde_json::from_value(v).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn plan_setting_wire_format() {
        let p: PlanSetting =
            serde_json::from_value(json!({"amount": 540, "currency": "USD", "period": "year"}))
                .unwrap();
        assert_eq!(p.currency, PlanCurrency::Usd);
        assert_eq!(p.period, PlanPeriod::Year);
    }
}
