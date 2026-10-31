# Snapshot contract (usage-core ⇄ UI)

Single source of truth for the data the Rust core emits and the React UI renders.
Rust: `crates/usage-core/src/model.rs` with `#[serde(rename_all = "camelCase")]`.
TypeScript: `src/types.ts`. Both MUST match this file field for field.

Times are Unix epoch **seconds** (number). Money fields named `*Usd` are US dollars;
`display*` fields are already converted to the display currency (`currency`).

```ts
export type Provider = "claude" | "codex";

/** Freshness of one limit window. */
export type WindowState = "known" | "stale" | "unknown" | "afterReset";
/** Governing state of a provider's limits. "off" = no source (Claude with neither Claude Code connected nor T3 Code, Codex disabled). */
export type LimitsState = "known" | "stale" | "unknown" | "off";

export interface LimitWindow {
  kind: "5h" | "week" | "weekModel" | "other";
  label: string;              // e.g. "5h", "týden", "Fable" (Claude weekModel), "týden · gpt-6", "okno 120 min"
  percent: number | null;     // 0..100, null when unknown or afterReset
  resetsAt: number | null;    // epoch s
  fetchedAt: number | null;   // epoch s of last successful read
  state: WindowState;
}

export interface ProviderLimits {
  provider: Provider;
  state: LimitsState;
  governing: number | null;   // max percent over 5h + week windows that have a value
  windows: LimitWindow[];
  note: string | null;        // Czech UI hint, e.g. "Codex CLI nenalezen", "limity Claude: připoj Claude Code v Nastavení"
}

/** "estimate": Codex cost from daily token totals × blended rate. */
export type ScoreState = "ok" | "estimate" | "partial" | "unknown" | "formatChanged" | "off";

export interface ProviderScore {
  provider: Provider;
  usd: number;                // API-equivalent cost this month
  display: number;            // usd converted to display currency
  tokens: number;             // all tokens this month
  unpricedTokens: number;     // tokens of models without a price (never priced as 0)
  state: ScoreState;
  planMonthlyDisplay: number | null;  // monthly flat fee in display currency, null = not set
  multiple: number | null;    // display / planMonthlyDisplay
}

export type Mood =
  | "confused" | "dozing" | "panic" | "sweat" | "celebrate" | "focus" | "sleep" | "calm";

export interface BreakdownRow { key: string; usd: number; display: number; tokens: number }

export interface Snapshot {
  generatedAt: number;
  month: string;                     // "2026-10" (Europe/Prague)
  currency: "EUR" | "CZK";
  limits: ProviderLimits[];          // always both providers, claude first
  scores: ProviderScore[];           // always both providers, claude first
  total: {
    display: number;
    multiple: number | null;         // only providers with a price and a plan count
    planMonthlyDisplay: number | null;
    excludedNote: string | null;     // e.g. "Codex bez ceny, paušál nezapočten"
  };
  moods: { provider: Provider; mood: Mood }[];
  tray: "green" | "orange" | "red" | "greenDim" | "orangeDim" | "redDim" | "grey" | "mono";
  daily: { date: string; claudeDisplay: number; codexDisplay: number }[]; // every day of month so far
  bySource: BreakdownRow[];          // key = entrypoint: "cli" | "sdk-ts" | "claude-desktop" | "codex" | other
  byModel: BreakdownRow[];
  byProject: BreakdownRow[];         // key = project folder name (last path segment of cwd)
  streaks: { provider: Provider; current: number; longest: number }[];
  backfill: { done: boolean; filesDone: number; filesTotal: number };
  celebration: { id: string; who: Provider | "both"; text: string } | null;
  updatedAgoSec: number;             // seconds since last data change
  widgetTransparency: number;        // 0..90 %, widget background (from settings)
}

export interface PlanSetting { amount: number; currency: "EUR" | "CZK" | "USD"; period: "month" | "year" }

export interface Settings {
  currency: "EUR" | "CZK";
  plans: { claude: PlanSetting | null; codex: PlanSetting | null };
  codexEstimateInScore: boolean;     // default true
  excludedEntrypoints: string[];     // default []
  clickThrough: boolean;             // default false
  widgetTransparency: number;        // 0..90 (percent), default 0; applied immediately
  checkUpdates: boolean;             // default true; automatic update checks
}
```

Tauri commands (shell): `get_snapshot() -> Snapshot`, `get_settings() -> Settings`,
`set_settings(settings: Settings)`, `set_click_through(on: bool)`,
`set_widget_transparency(value: u8)`, `refresh_usage()` (read every source now), `open_app()`,
`clear_history()`, `claude_code_status() / claude_code_connect() / claude_code_disconnect()
-> ClaudeCodeStatus` (Settings › Claude Code; only the app window), `autostart_status() /
autostart_set(on: bool) -> AutostartStatus` (Settings › Widget, the system's startup list; only the app window),
`accounts() -> Accounts` (Settings › Účty; only the app window, never part of the snapshot),
`history(range: HistoryRange, filter: HistoryFilter) -> History` (History tab; only the app window, never part of the snapshot). Event `"snapshot"` carries a full `Snapshot` on every change.

```ts
/** The account Claude Code is signed in with (`oauthAccount` in its .claude.json). Raw enum values. */
export interface ClaudeAccount {
  email: string | null;
  name: string | null;
  organization: string | null;
  organizationType: string | null;   // "claude_max", "claude_team", …
  role: string | null;               // "admin", "membership_admin", …
  seatTier: string | null;
  rateLimitTier: string | null;      // "default_claude_max_5x", …
  billing: string | null;            // "stripe_subscription", …
  accountCreatedAt: number | null;
  subscriptionCreatedAt: number | null;
  extraUsage: boolean | null;
  extraUsageBlocked: string | null;  // "out_of_credits", …
  profileFetchedAt: number | null;
  limitsFetchedAt: number | null;
}

export interface CodexCredits { hasCredits: boolean; unlimited: boolean; balance: string | null }

/** The account the Codex CLI is signed in with (app-server `account/read` + limits and usage reads). */
export interface CodexAccount {
  kind: string;                      // "chatgpt" | "apiKey" | "amazonBedrock" | "gcp" | "none"
  email: string | null;
  plan: string | null;               // ChatGPT planType
  workspace: string | null;
  cliVersion: string | null;
  resetCredits: number | null;
  credits: CodexCredits | null;
  lifetimeTokens: number | null;
  readAt: number;
}

export interface Accounts {
  claude: ClaudeAccount | null;
  codex: CodexAccount | null;
  codexNote: string | null;          // why there is no Codex account
}
```

History tab: the core prices a period on request from the stored tokens, with the
month's breakdowns. 24 h goes by hours, the rest by calendar days (Europe/Prague),
today included; `all` starts on the first day with data. Codex reports daily totals
only: it is one row (`key: "codex"`) in every breakdown, one part (`codex`) of the
type split, priced at the blended rate (`estimate`), and absent from the 24 h view
(`codexWithoutHours`). Cache savings = cache-read tokens × (input price − cache-read
price). Claude history whose transcripts Claude Code already deleted (it keeps 30 days)
is restored once from T3 Code's usage cache (`~/.t3/userdata/usage-scan-cache*.json`)
after a layout check against the messages Skylimit counted itself; it carries the
source `t3-cache`, and its output tokens are a lower bound (T3 keeps the first,
partial count of a message).

```ts
export type HistoryRange = "24h" | "7d" | "30d" | "90d" | "all";
export type HistoryFilter = "all" | "claude" | "codex";
export type HistoryKind = "input" | "cacheRead" | "cacheWrite" | "output" | "webSearch" | "codex";

export interface HistoryKindRow { kind: HistoryKind; tokens: number; display: number }

export interface HistoryProvider {
  provider: Provider;
  display: number;
  tokens: number;
  unpricedTokens: number;            // never priced as 0
  estimate: boolean;                 // Codex: daily totals × blended rate
}

export interface HistoryPoint {
  start: number;                     // epoch s of the bucket's start (local midnight for days)
  date: string | null;               // "2026-10-07" for day buckets
  claudeDisplay: number;
  codexDisplay: number;
  claudeTokens: number;
  codexTokens: number;
}

export interface HistoryRow { key: string; provider: Provider | null; display: number; tokens: number }

export interface History {
  range: HistoryRange;
  filter: HistoryFilter;
  currency: "EUR" | "CZK";
  bucket: "hour" | "day";
  from: number;                      // start of the first bucket
  to: number;                        // time of the query
  firstDataAt: number | null;        // oldest record of the shown providers
  activeBuckets: number;             // days (hours in 24 h) with any tokens
  totalDisplay: number;
  totalTokens: number;
  providers: HistoryProvider[];      // claude first; only the shown ones
  kinds: HistoryKindRow[];           // fixed order, present ones only
  cacheSavingsDisplay: number;
  series: HistoryPoint[];
  byModel: HistoryRow[];
  byDay: HistoryRow[];               // newest first
  bySource: HistoryRow[];            // "t3-cache" = restored from T3 Code
  byProject: HistoryRow[];
  codexWithoutHours: boolean;
}
```

Mood priority (core computes, UI only renders): confused(Unknown) > dozing(Stale) >
panic(≥95, leave <90) > sweat(≥75, leave <70) > celebrate(4 s) > focus(activity in
last 5 min) > sleep(30 min without activity, only if an activity signal exists) > calm.
Limits `off`: never confused/dozing; mood from score + activity only.

Claude limits come from three passive sources, never from Claude login data; the newest
reading wins: the limits Claude Code caches for itself in `~/.claude.json`
(`cachedUsageUtilization`, only while it belongs to the signed-in `oauthAccount`),
Claude Code's status line (`rate_limits`, recorded by `skylimit statusline`
in `<data>/claude-code/` once connected) and T3 Code's provider logs
(`~/.t3/userdata/logs/provider`, `rate_limit_event` of the Claude Agent SDK).
The weekly Fable limit is a third Claude window, `kind: "weekModel"`, `label: "Fable"`,
after the week window: from the cache's `limits[]` entry `weekly_scoped` with model
"Fable", or the SDK's `seven_day_overage_included` window. It keeps its own reading
time, never counts toward `governing`, and only Fable work (model containing "fable"
in the JSONL) makes it Stale or AfterReset; it is absent when no source reports one. Known until
Claude work appears in the JSONL more than 15 min after the reading (Stale); after
`resetsAt`: Known 0 % without work since the reset (5h: `resetsAt` null, starts with the
next message), AfterReset otherwise. Neither source present: `off`; a source without a
reading yet: `unknown`.
