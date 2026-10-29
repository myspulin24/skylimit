//! Claude Code's global config (`.claude.json` in the home folder, or in
//! `CLAUDE_CONFIG_DIR` when set). Claude Code keeps two things there that
//! Skylimit shows, both written by Claude Code itself:
//!
//! - `oauthAccount`: the account it is signed in with (e-mail, organization,
//!   plan tier), for Settings › Účty.
//! - `cachedUsageUtilization`: the account's limits as Claude Code last read
//!   them for itself (about every 10 minutes while it runs in a terminal,
//!   T3 Code or an editor; dropped on logout). The third source of Claude's
//!   limits next to `t3.rs` and `claude_code.rs`, and the one that carries
//!   the weekly Fable limit (`limits[]`, kind `weekly_scoped`, model Fable).
//!
//! Read-only and offline: nothing here calls Anthropic. Only those two keys
//! are deserialized; serde skips every other key of the file without keeping
//! it. A usage cache counts only while it belongs to the signed-in account,
//! the same rule Claude Code applies.
//!
//! ```text
//!  Claude Code ─writes─▶ ~/.claude.json ─▶ ConfigFile::poll (5 s, on change)
//!                                       ─▶ ClaudeReading { 5h, week, fable }
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::Value;

use crate::claude_limits::{ClaudeReading, ClaudeWindow};
use crate::model::ClaudeAccount;

pub const FILE_NAME: &str = ".claude.json";

/// Where Claude Code keeps the file: `CLAUDE_CONFIG_DIR` when set, else the
/// home folder itself (not `~/.claude`).
pub fn path(config_dir: Option<&Path>, home: &Path) -> PathBuf {
    config_dir.unwrap_or(home).join(FILE_NAME)
}

/// The two keys Skylimit reads; serde skips all others unread.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct File {
    #[serde(default)]
    oauth_account: Option<Value>,
    #[serde(default)]
    cached_usage_utilization: Option<Value>,
}

/// One read of the file: the account and the limits Claude Code cached.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Parsed {
    pub account: Option<ClaudeAccount>,
    pub reading: Option<ClaudeReading>,
}

/// `None` when the file is missing or not JSON (also halfway through a write).
fn load(path: &Path) -> Option<Parsed> {
    let file: File = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    Some(parse(&file))
}

/// The signed-in Claude account, for Settings › Účty.
pub fn account(path: &Path) -> Option<ClaudeAccount> {
    load(path)?.account
}

fn parse(file: &File) -> Parsed {
    let Some(a) = file.oauth_account.as_ref().filter(|a| a.is_object()) else {
        // Signed out: Claude Code drops the usage cache too.
        return Parsed::default();
    };
    let uuid = text(a, "accountUuid");
    let cache = file
        .cached_usage_utilization
        .as_ref()
        .filter(|c| text(c, "accountUuid").is_none_or(|owner| uuid.as_deref() == Some(owner.as_str())));
    let extra = cache.and_then(|c| c.pointer("/utilization/extra_usage"));
    let account = ClaudeAccount {
        email: text(a, "emailAddress"),
        name: text(a, "fullName").or_else(|| text(a, "displayName")),
        organization: text(a, "organizationName"),
        organization_type: text(a, "organizationType"),
        role: text(a, "organizationRole"),
        seat_tier: text(a, "seatTier"),
        rate_limit_tier: text(a, "userRateLimitTier"),
        billing: text(a, "billingType"),
        account_created_at: text(a, "accountCreatedAt").and_then(|t| epoch(&t)),
        subscription_created_at: text(a, "subscriptionCreatedAt").and_then(|t| epoch(&t)),
        extra_usage: a.get("hasExtraUsageEnabled").and_then(Value::as_bool),
        extra_usage_blocked: extra
            .filter(|e| e.get("is_enabled").and_then(Value::as_bool) != Some(true))
            .and_then(|e| text(e, "disabled_reason")),
        profile_fetched_at: millis(a.get("profileFetchedAt")),
        limits_fetched_at: cache.and_then(|c| millis(c.get("fetchedAtMs"))),
    };
    Parsed {
        account: Some(account),
        reading: cache.and_then(reading_of),
    }
}

fn reading_of(cache: &Value) -> Option<ClaudeReading> {
    let at = millis(cache.get("fetchedAtMs"))?;
    let u = cache.get("utilization")?;
    let top = |key: &str| u.get(key).and_then(|w| window(w.get("utilization"), w.get("resets_at")));
    let fable = u
        .get("limits")
        .and_then(Value::as_array)
        .and_then(|limits| {
            limits.iter().find(|l| {
                l.get("kind").and_then(Value::as_str) == Some("weekly_scoped")
                    && l.pointer("/scope/model/display_name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.trim().eq_ignore_ascii_case("fable"))
            })
        })
        .and_then(|l| window(l.get("percent"), l.get("resets_at")));
    let r = ClaudeReading {
        at,
        five_hour: top("five_hour"),
        seven_day: top("seven_day"),
        fable,
    };
    (r.five_hour.is_some() || r.seven_day.is_some() || r.fable.is_some()).then_some(r)
}

/// `percent` is 0–100 here (unlike the SDK's fraction); `resets_at` is ISO 8601.
fn window(percent: Option<&Value>, resets_at: Option<&Value>) -> Option<ClaudeWindow> {
    let percent = percent?.as_f64().filter(|p| p.is_finite())?;
    Some(ClaudeWindow {
        percent: ((percent * 10.0).round() / 10.0).clamp(0.0, 100.0),
        resets_at: resets_at.and_then(Value::as_str).and_then(epoch),
    })
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn millis(v: Option<&Value>) -> Option<i64> {
    v?.as_f64().filter(|ms| ms.is_finite() && *ms > 0.0).map(|ms| (ms / 1000.0) as i64)
}

fn epoch(iso: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(iso).ok().map(|d| d.timestamp())
}

// ---------------------------------------------------------------- polling

/// Re-reads the file only when its size or modification time moved.
#[derive(Default)]
pub(crate) struct ConfigFile {
    stamp: Option<(SystemTime, u64)>,
}

impl ConfigFile {
    /// The limits Claude Code cached, when the file changed since the last
    /// poll. A file caught halfway through a write is read again next time.
    pub(crate) fn poll(&mut self, path: &Path) -> Option<ClaudeReading> {
        let meta = fs::metadata(path).ok()?;
        let stamp = (meta.modified().ok()?, meta.len());
        if self.stamp == Some(stamp) {
            return None;
        }
        let parsed = load(path)?;
        self.stamp = Some(stamp);
        parsed.reading
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Shape of Claude Code 2.1.312's file, trimmed; secrets never appear in it,
    /// but the test proves unrelated keys are not read either.
    fn sample() -> Value {
        json!({
            "numStartups": 412,
            "primaryApiKey": "must-never-be-read",
            "projects": { "C:/work": { "history": [{ "display": "secret prompt" }] } },
            "oauthAccount": {
                "accountUuid": "acc-1",
                "emailAddress": "jana@example.com",
                "organizationUuid": "org-1",
                "hasExtraUsageEnabled": true,
                "billingType": "stripe_subscription",
                "accountCreatedAt": "2026-03-30T04:34:23.655342Z",
                "subscriptionCreatedAt": "2026-03-28T19:53:32.539118Z",
                "seatTier": "team_tier_1",
                "displayName": "Jana",
                "fullName": "Jana Nováková",
                "profileFetchedAt": 1_793_273_253_277u64,
                "organizationRole": "membership_admin",
                "workspaceRole": null,
                "organizationName": "Firma",
                "organizationType": "claude_team",
                "userRateLimitTier": "default_claude_max_5x"
            },
            "cachedUsageUtilization": {
                "fetchedAtMs": 1_793_305_805_533u64,
                "accountUuid": "acc-1",
                "utilization": {
                    "five_hour": { "utilization": 6, "resets_at": "2026-10-30T01:10:00.001631+00:00" },
                    "seven_day": { "utilization": 23, "resets_at": "2026-11-03T17:00:00.001655+00:00" },
                    "seven_day_opus": null,
                    "extra_usage": { "is_enabled": false, "disabled_reason": "out_of_credits" },
                    "limits": [
                        { "kind": "session", "group": "session", "percent": 6, "resets_at": "2026-10-30T01:10:00.001631+00:00", "scope": null },
                        { "kind": "weekly_all", "group": "weekly", "percent": 23, "resets_at": "2026-11-03T17:00:00.001655+00:00", "scope": null },
                        { "kind": "weekly_scoped", "group": "weekly", "percent": 12.34, "resets_at": "2026-11-03T17:00:00+00:00",
                          "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null } }
                    ]
                }
            }
        })
    }

    fn parsed(v: Value) -> Parsed {
        parse(&serde_json::from_value(v).unwrap())
    }

    #[test]
    fn reads_account_and_all_three_windows() {
        let p = parsed(sample());
        let a = p.account.unwrap();
        assert_eq!(a.email.as_deref(), Some("jana@example.com"));
        assert_eq!(a.name.as_deref(), Some("Jana Nováková"));
        assert_eq!(a.organization.as_deref(), Some("Firma"));
        assert_eq!(a.organization_type.as_deref(), Some("claude_team"));
        assert_eq!(a.role.as_deref(), Some("membership_admin"));
        assert_eq!(a.rate_limit_tier.as_deref(), Some("default_claude_max_5x"));
        assert_eq!(a.subscription_created_at, Some(1_774_727_612));
        assert_eq!(a.extra_usage, Some(true));
        assert_eq!(a.extra_usage_blocked.as_deref(), Some("out_of_credits"));
        assert_eq!(a.profile_fetched_at, Some(1_793_273_253));
        assert_eq!(a.limits_fetched_at, Some(1_793_305_805));

        let r = p.reading.unwrap();
        assert_eq!(r.at, 1_793_305_805);
        assert_eq!(r.five_hour.as_ref().unwrap().percent, 6.0);
        assert_eq!(r.five_hour.unwrap().resets_at, Some(1_793_322_600));
        assert_eq!(r.seven_day.unwrap().percent, 23.0);
        let fable = r.fable.unwrap();
        assert_eq!(fable.percent, 12.3);
        assert_eq!(fable.resets_at, Some(1_793_725_200));
    }

    #[test]
    fn nothing_else_of_the_file_is_kept() {
        let p = parsed(sample());
        let text = format!("{p:?}");
        assert!(!text.contains("must-never-be-read"));
        assert!(!text.contains("secret prompt"));
        assert!(!text.contains("acc-1"), "ids are compared, never kept");
    }

    #[test]
    fn cache_of_another_account_or_signed_out_is_ignored() {
        let mut other = sample();
        other["cachedUsageUtilization"]["accountUuid"] = json!("acc-2");
        let p = parsed(other);
        assert!(p.account.is_some());
        assert_eq!(p.reading, None);
        assert_eq!(p.account.unwrap().limits_fetched_at, None);

        let mut out = sample();
        out.as_object_mut().unwrap().remove("oauthAccount");
        assert_eq!(parsed(out), Parsed::default());
    }

    #[test]
    fn plans_without_a_fable_limit_and_odd_shapes() {
        let mut v = sample();
        v["cachedUsageUtilization"]["utilization"]["limits"] = json!([]);
        let r = parsed(v).reading.unwrap();
        assert_eq!(r.fable, None);
        assert!(r.seven_day.is_some());

        let mut odd = sample();
        odd["cachedUsageUtilization"]["utilization"] = json!({ "five_hour": { "utilization": "x" }, "limits": "nope" });
        assert_eq!(parsed(odd).reading, None);

        assert_eq!(parse(&serde_json::from_str::<File>("[]").unwrap_or_default()), Parsed::default());
        assert_eq!(parsed(json!({})), Parsed::default());
    }

    #[test]
    fn polls_only_on_change_and_survives_a_half_written_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        let mut cfg = ConfigFile::default();
        assert_eq!(cfg.poll(&file), None, "missing file");

        fs::write(&file, "{\"oauthAccount\": {").unwrap();
        assert_eq!(cfg.poll(&file), None, "half written");
        fs::write(&file, sample().to_string()).unwrap();
        assert!(cfg.poll(&file).is_some(), "read once complete");
        assert_eq!(cfg.poll(&file), None, "unchanged");
        assert_eq!(account(&file).unwrap().email.as_deref(), Some("jana@example.com"));
        assert_eq!(path(None, dir.path()), file);
        assert_eq!(path(Some(Path::new("/cfg")), dir.path()), Path::new("/cfg").join(FILE_NAME));
    }
}
