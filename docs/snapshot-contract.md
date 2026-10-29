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
`accounts() -> Accounts` (Settings › Účty; only the app window, never part of the snapshot). Event `"snapshot"` carries a full `Snapshot` on every change.

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
