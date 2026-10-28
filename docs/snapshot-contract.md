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
/** Governing state of a provider's limits. "off" = no source (Claude without T3 Code, Codex disabled). */
export type LimitsState = "known" | "stale" | "unknown" | "off";

export interface LimitWindow {
  kind: "5h" | "week" | "weekModel" | "other";
  label: string;              // e.g. "5h", "týden", "týden · gpt-6", "okno 120 min"
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
  note: string | null;        // Czech UI hint, e.g. "Codex CLI nenalezen", "limity Claude jen přes T3 Code (nenalezen)"
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
`clear_history()`. Event `"snapshot"` carries a full `Snapshot` on every change.

Mood priority (core computes, UI only renders): confused(Unknown) > dozing(Stale) >
panic(≥95, leave <90) > sweat(≥75, leave <70) > celebrate(4 s) > focus(activity in
last 5 min) > sleep(30 min without activity, only if an activity signal exists) > calm.
Limits `off`: never confused/dozing; mood from score + activity only.

Claude limits come from T3 Code's provider logs (`~/.t3/userdata/logs/provider`,
`rate_limit_event` of the Claude Agent SDK), never from Claude login data. Known until
Claude work appears in the JSONL more than 15 min after the reading (Stale); after
`resetsAt`: Known 0 % without work since the reset (5h: `resetsAt` null, starts with the
next message), AfterReset otherwise. No T3 Code folder: `off`; folder without a reading: `unknown`.
