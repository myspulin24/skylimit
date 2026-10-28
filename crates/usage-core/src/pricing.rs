//! Price list: embedded LiteLLM snapshot plus documented overrides.
//!
//! Prices are applied at query time to stored token counts, so a newer price
//! list reprices the past without rereading any logs. A model without a price
//! is never priced as 0: its tokens are reported as `unpricedTokens`.

use std::collections::HashMap;

use serde_json::{Map, Value};

/// LiteLLM price list (MIT). Snapshot fetched 2026-10-28 and filtered by
/// the same rules as [`PriceTable::from_litellm_json`].
const EMBEDDED: &str = include_str!("../data/prices.json");
/// Models missing from LiteLLM. Only entries with a `source` are accepted.
const OVERRIDES: &str = include_str!("../data/price-overrides.json");

pub(crate) const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// Anthropic server-side web search: USD 10 per 1,000 searches, all models.
/// Source: https://platform.claude.com/docs/en/about-claude/pricing (the same
/// page LiteLLM cites). Used only when the model entry has no per-query price.
pub(crate) const ANTHROPIC_WEB_SEARCH_USD: f64 = 0.01;

/// Codex cost estimate (spike result): `account/usage/read` only returns daily
/// token totals without a model or input/output split, so Codex is priced with
/// a blended per-token rate:
///
/// `rate = 0.70 × cached input + 0.25 × uncached input + 0.05 × output`
///
/// of [`CODEX_BLEND_MODEL`], the model in 4 of the 5 most recent Codex threads
/// on this machine (thread-list fixture, 2026-10-28). With the LiteLLM snapshot
/// of 2026-10-28 (gpt-6-astra: input 10, cached 1, output 50 USD / 1M tokens):
/// 0.70 × 1 + 0.25 × 10 + 0.05 × 50 = **5.70 USD per 1M tokens**
/// (5.7e-6 USD per token). Computed from the table at runtime, so a newer price
/// list moves the estimate as well.
pub(crate) const CODEX_BLEND_MODEL: &str = "gpt-6-astra";
pub(crate) const CODEX_BLEND_CACHED: f64 = 0.70;
pub(crate) const CODEX_BLEND_UNCACHED: f64 = 0.25;
pub(crate) const CODEX_BLEND_OUTPUT: f64 = 0.05;

/// Input-side token thresholds (thousands) recorded per request. A request
/// stores the largest threshold it exceeds (0 = none). LiteLLM bands seen:
/// 100k (claude-haiku-5-5), 200k (older Claude), 272k (OpenAI).
pub(crate) const BAND_THRESHOLDS_K: [u32; 3] = [100, 200, 272];

pub(crate) fn context_band(input_side_tokens: u64) -> u32 {
    BAND_THRESHOLDS_K
        .iter()
        .rev()
        .copied()
        .find(|t| input_side_tokens > u64::from(*t) * 1000)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum TokenKind {
    Input,
    Output,
    CacheRead,
    CacheWrite5m,
    CacheWrite1h,
}

impl TokenKind {
    pub(crate) const ALL: [TokenKind; 5] = [
        TokenKind::Input,
        TokenKind::Output,
        TokenKind::CacheRead,
        TokenKind::CacheWrite5m,
        TokenKind::CacheWrite1h,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TokenKind::Input => "input",
            TokenKind::Output => "output",
            TokenKind::CacheRead => "cache_read",
            TokenKind::CacheWrite5m => "cache_write_5m",
            TokenKind::CacheWrite1h => "cache_write_1h",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<TokenKind> {
        TokenKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct KindPrices {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write_5m: Option<f64>,
    pub cache_write_1h: Option<f64>,
}

impl KindPrices {
    fn get(&self, kind: TokenKind) -> Option<f64> {
        match kind {
            TokenKind::Input => self.input,
            TokenKind::Output => self.output,
            TokenKind::CacheRead => self.cache_read,
            TokenKind::CacheWrite5m => self.cache_write_5m,
            TokenKind::CacheWrite1h => self.cache_write_1h,
        }
    }

    /// LiteLLM field names with an optional suffix (`_above_200k_tokens`,
    /// `_priority`).
    fn from_fields(map: &Map<String, Value>, suffix: &str) -> KindPrices {
        let f = |name: &str| map.get(&format!("{name}{suffix}")).and_then(Value::as_f64);
        KindPrices {
            input: f("input_cost_per_token"),
            output: f("output_cost_per_token"),
            cache_read: f("cache_read_input_token_cost"),
            cache_write_5m: f("cache_creation_input_token_cost"),
            cache_write_1h: f("cache_creation_input_token_cost_above_1hr"),
        }
    }

    fn is_empty(&self) -> bool {
        *self == KindPrices::default()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ModelPrice {
    pub provider: String,
    pub base: KindPrices,
    /// (threshold in thousands of input-side tokens, prices above it), ascending.
    pub bands: Vec<(u32, KindPrices)>,
    pub priority: Option<KindPrices>,
    /// `provider_specific_entry.fast`: price multiplier for fast mode.
    pub fast_multiplier: Option<f64>,
    pub web_search_usd: Option<f64>,
}

impl ModelPrice {
    fn from_entry(map: &Map<String, Value>) -> Option<ModelPrice> {
        let provider = map.get("litellm_provider")?.as_str()?.to_string();
        let base = KindPrices::from_fields(map, "");
        if base.input.is_none() && base.output.is_none() {
            return None;
        }
        let mut thresholds: Vec<u32> = map
            .keys()
            .filter_map(|k| {
                let stem = k.strip_suffix("k_tokens")?;
                let idx = stem.rfind("_above_")?;
                stem[idx + "_above_".len()..].parse::<u32>().ok()
            })
            .collect();
        thresholds.sort_unstable();
        thresholds.dedup();
        let bands = thresholds
            .into_iter()
            .map(|t| (t, KindPrices::from_fields(map, &format!("_above_{t}k_tokens"))))
            .filter(|(_, p)| !p.is_empty())
            .collect();
        let priority = Some(KindPrices::from_fields(map, "_priority")).filter(|p| !p.is_empty());
        let fast_multiplier = map
            .get("provider_specific_entry")
            .and_then(|e| e.get("fast"))
            .and_then(Value::as_f64);
        let web_search_usd = map.get("search_context_cost_per_query").and_then(|s| {
            s.get("search_context_size_medium")
                .or_else(|| s.get("search_context_size_low"))
                .and_then(Value::as_f64)
        });
        Some(ModelPrice {
            provider,
            base,
            bands,
            priority,
            fast_multiplier,
            web_search_usd,
        })
    }

    /// USD per token, or `None` when this combination has no known price.
    ///
    /// - band: the model's highest band whose threshold is ≤ the stored band;
    ///   a kind missing in the band falls back to the base price (LiteLLM lists
    ///   only the kinds that carry a surcharge).
    /// - service tier `priority`: priority prices when listed, else the list
    ///   price (Anthropic's Priority Tier is contract-priced, not public).
    /// - speed `fast`: base × `provider_specific_entry.fast`; without a known
    ///   multiplier the tokens stay unpriced.
    pub(crate) fn unit_price(&self, kind: TokenKind, band: u32, speed: &str, tier: &str) -> Option<f64> {
        let mut price = if tier == "priority" {
            self.priority.as_ref().and_then(|p| p.get(kind))
        } else {
            None
        };
        if price.is_none() {
            let banded = if band > 0 {
                self.bands
                    .iter()
                    .rev()
                    .find(|(t, _)| *t <= band)
                    .and_then(|(_, p)| p.get(kind))
            } else {
                None
            };
            price = banded.or_else(|| self.base.get(kind));
        }
        let price = price?;
        match speed {
            "fast" => self.fast_multiplier.map(|m| price * m),
            _ => Some(price),
        }
    }

    pub(crate) fn web_search_price(&self) -> Option<f64> {
        self.web_search_usd.or(if self.provider == "anthropic" {
            Some(ANTHROPIC_WEB_SEARCH_USD)
        } else {
            None
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PriceTable {
    models: HashMap<String, ModelPrice>,
    /// normalized id → key in `models` (undated entry preferred)
    by_normalized: HashMap<String, String>,
}

impl PriceTable {
    /// Embedded snapshot plus overrides. Both are compile-time data covered by
    /// tests, so a parse failure only yields an empty (all-unpriced) table.
    pub(crate) fn embedded() -> PriceTable {
        let mut table = PriceTable::from_litellm_json(EMBEDDED).unwrap_or_default();
        table.apply_overrides(OVERRIDES);
        table
    }

    /// Parse a LiteLLM-format price list, keeping Anthropic `claude*` and
    /// OpenAI `gpt*`/`o*`/`*codex*` chat models.
    pub(crate) fn from_litellm_json(text: &str) -> Result<PriceTable, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| crate::diag::error_category(&e))?;
        let obj = root.as_object().ok_or("price list is not an object")?;
        let mut table = PriceTable::default();
        for (raw_key, entry) in obj {
            let Some(map) = entry.as_object() else { continue };
            if !wanted(raw_key, map) {
                continue;
            }
            if let Some(price) = ModelPrice::from_entry(map) {
                table.insert(strip_provider(&raw_key.to_ascii_lowercase()), price, false);
            }
        }
        if table.models.is_empty() {
            return Err("no models in price list".into());
        }
        Ok(table)
    }

    fn apply_overrides(&mut self, text: &str) {
        let Ok(root) = serde_json::from_str::<Value>(text) else { return };
        let Some(models) = root.get("models").and_then(Value::as_object) else { return };
        for (key, entry) in models {
            let Some(map) = entry.as_object() else { continue };
            let sourced = map
                .get("source")
                .and_then(Value::as_str)
                .is_some_and(|s| s.starts_with("http"));
            if !sourced {
                continue;
            }
            if let Some(price) = ModelPrice::from_entry(map) {
                self.insert(&key.to_ascii_lowercase(), price, true);
            }
        }
    }

    fn insert(&mut self, key: &str, price: ModelPrice, overwrite: bool) {
        if !overwrite && self.models.contains_key(key) {
            return;
        }
        let norm = normalize_model(key);
        let prefer = norm == key || !self.by_normalized.contains_key(&norm);
        if prefer {
            self.by_normalized.insert(norm, key.to_string());
        }
        self.models.insert(key.to_string(), price);
    }

    pub(crate) fn lookup(&self, model: &str) -> Option<&ModelPrice> {
        let lower = model.trim().to_ascii_lowercase();
        let key = strip_provider(lower.strip_suffix("[1m]").unwrap_or(&lower));
        if let Some(p) = self.models.get(key) {
            return Some(p);
        }
        let norm = normalize_model(key);
        self.by_normalized.get(&norm).and_then(|k| self.models.get(k))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.models.len()
    }

    /// Codex blended USD per token (see [`CODEX_BLEND_MODEL`]).
    pub(crate) fn codex_blended_usd_per_token(&self) -> Option<f64> {
        let p = &self.lookup(CODEX_BLEND_MODEL)?.base;
        Some(
            CODEX_BLEND_CACHED * p.cache_read?
                + CODEX_BLEND_UNCACHED * p.input?
                + CODEX_BLEND_OUTPUT * p.output?,
        )
    }

    /// Sanity check for a downloaded list: it must contain models and no
    /// model known to both lists may move by more than 10× (design rule for
    /// the optional weekly refresh).
    pub(crate) fn plausible_successor(&self, newer: &PriceTable) -> bool {
        if newer.models.len() < self.models.len() / 2 {
            return false;
        }
        self.models.iter().all(|(k, old)| match (newer.models.get(k), old.base.input) {
            (Some(new), Some(old_in)) => match new.base.input {
                Some(new_in) if old_in > 0.0 => {
                    let r = new_in / old_in;
                    (0.1..=10.0).contains(&r)
                }
                Some(_) => true,
                None => false,
            },
            _ => true,
        })
    }
}

fn wanted(raw_key: &str, map: &Map<String, Value>) -> bool {
    let key = strip_provider(&raw_key.to_ascii_lowercase()).to_string();
    let provider = map.get("litellm_provider").and_then(Value::as_str).unwrap_or("");
    let mode = map.get("mode").and_then(Value::as_str).unwrap_or("");
    match provider {
        "anthropic" => key.starts_with("claude") && mode == "chat",
        "openai" => {
            let family = key.starts_with("gpt-")
                || (key.starts_with('o') && key[1..].starts_with(|c: char| c.is_ascii_digit()))
                || key.contains("codex");
            let skip = ["audio", "realtime", "transcribe", "tts", "image", "embedding", "moderation", "search-preview"]
                .iter()
                .any(|s| key.contains(s));
            family && !skip && (mode == "chat" || mode == "responses")
        }
        _ => false,
    }
}

fn strip_provider(key: &str) -> &str {
    key.strip_prefix("anthropic/")
        .or_else(|| key.strip_prefix("openai/"))
        .unwrap_or(key)
}

/// Lowercase, strip `anthropic/` / `openai/`, a `[1m]` suffix and a date
/// suffix (`-20250514`, `-2026-04-23`).
pub(crate) fn normalize_model(id: &str) -> String {
    let lower = id.trim().to_ascii_lowercase();
    let mut s = strip_provider(lower.strip_suffix("[1m]").unwrap_or(&lower)).to_string();
    let bytes = s.as_bytes();
    let digits = |b: &[u8]| b.iter().all(u8::is_ascii_digit);
    if bytes.len() > 9 && bytes[bytes.len() - 9] == b'-' && digits(&bytes[bytes.len() - 8..]) {
        s.truncate(s.len() - 9);
    } else if bytes.len() > 11 {
        let tail = &bytes[bytes.len() - 11..];
        if tail[0] == b'-'
            && digits(&tail[1..5])
            && tail[5] == b'-'
            && digits(&tail[6..8])
            && tail[8] == b'-'
            && digits(&tail[9..11])
        {
            s.truncate(s.len() - 11);
        }
    }
    s
}

/// Optional weekly refresh of the LiteLLM list. OFF by default in the MVP
/// (`Config::pricing_refresh`); the downloaded list replaces the embedded one
/// in memory only when [`PriceTable::plausible_successor`] accepts it.
pub(crate) fn fetch_litellm() -> Result<PriceTable, String> {
    let text = crate::http_get_text(LITELLM_URL)?;
    PriceTable::from_litellm_json(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_table_parses_and_has_local_models() {
        let t = PriceTable::embedded();
        assert!(t.len() > 50, "only {} models", t.len());
        // Models seen on this machine on 2026-10-28.
        for m in [
            "claude-opus-5-5",
            "claude-opus-4-7",
            "claude-fable-5-1",
            "claude-haiku-4-5-20251001",
            "gpt-6-astra",
            "gpt-5.6-sol",
        ] {
            assert!(t.lookup(m).is_some(), "{m} unpriced");
        }
    }

    #[test]
    fn normalizes_model_ids() {
        assert_eq!(normalize_model("anthropic/claude-sonnet-4-5-20250929"), "claude-sonnet-4-5");
        assert_eq!(normalize_model("openai/gpt-5.5-2026-04-23"), "gpt-5.5");
        assert_eq!(normalize_model("claude-opus-4-6[1m]"), "claude-opus-4-6");
        assert_eq!(normalize_model("Claude-Opus-5-5"), "claude-opus-5-5");
        assert_eq!(normalize_model("o3"), "o3");
        let t = PriceTable::embedded();
        // A dated id not in the table resolves through its undated family.
        assert!(t.lookup("claude-opus-5-5-20270101").is_some());
        assert!(t.lookup("anthropic/claude-opus-5-5").is_some());
    }

    #[test]
    fn unknown_model_has_no_price() {
        let t = PriceTable::embedded();
        assert!(t.lookup("claude-unreleased-9").is_none());
        assert!(t.lookup("<synthetic>").is_none());
    }

    #[test]
    fn prices_by_kind_band_speed() {
        let t = PriceTable::embedded();
        let opus = t.lookup("claude-opus-5-5").unwrap();
        assert_eq!(opus.unit_price(TokenKind::Input, 0, "standard", "standard"), Some(4e-6));
        assert_eq!(opus.unit_price(TokenKind::CacheWrite1h, 0, "standard", "standard"), Some(8e-6));
        // Flat pricing model: the long-context band changes nothing.
        assert_eq!(opus.unit_price(TokenKind::Output, 272, "standard", "standard"), Some(2e-5));
        // Fast mode multiplier 2 from provider_specific_entry.
        assert_eq!(opus.unit_price(TokenKind::Output, 0, "fast", "standard"), Some(4e-5));
        // No fast multiplier known → unpriced, never guessed.
        let fable = t.lookup("claude-fable-5-1").unwrap();
        assert_eq!(fable.unit_price(TokenKind::Output, 0, "fast", "standard"), None);
        // 100k band (claude-haiku-5-5).
        let haiku = t.lookup("claude-haiku-5-5").unwrap();
        assert_eq!(haiku.unit_price(TokenKind::Input, 0, "standard", "standard"), Some(1e-7));
        assert_eq!(haiku.unit_price(TokenKind::Input, 100, "standard", "standard"), Some(5e-7));
        assert_eq!(haiku.unit_price(TokenKind::Input, 200, "standard", "standard"), Some(5e-7));
        assert_eq!(opus.web_search_price(), Some(0.01));
    }

    #[test]
    fn context_bands() {
        assert_eq!(context_band(100_000), 0);
        assert_eq!(context_band(100_001), 100);
        assert_eq!(context_band(200_001), 200);
        assert_eq!(context_band(300_000), 272);
    }

    #[test]
    fn codex_blended_rate() {
        let rate = PriceTable::embedded().codex_blended_usd_per_token().unwrap();
        assert!((rate - 5.7e-6).abs() < 1e-12, "rate {rate}");
    }

    #[test]
    fn overrides_need_a_source() {
        let mut t = PriceTable::default();
        t.apply_overrides(
            r#"{"models":{
                "made-up-1":{"litellm_provider":"openai","input_cost_per_token":1e-6,"output_cost_per_token":2e-6},
                "documented-1":{"litellm_provider":"openai","input_cost_per_token":1e-6,"output_cost_per_token":2e-6,"source":"https://example.com/pricing"}
            }}"#,
        );
        assert!(t.lookup("made-up-1").is_none());
        assert!(t.lookup("documented-1").is_some());
    }

    #[test]
    fn refresh_sanity_check() {
        let old = PriceTable::embedded();
        assert!(old.plausible_successor(&old.clone()));
        let mut wild = old.clone();
        if let Some(p) = wild.models.get_mut("claude-opus-5-5") {
            p.base.input = p.base.input.map(|x| x * 50.0);
        }
        assert!(!old.plausible_successor(&wild));
        assert!(!old.plausible_successor(&PriceTable::default()));
    }
}
