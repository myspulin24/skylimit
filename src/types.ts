// Copied verbatim from docs/snapshot-contract.md. Must match
// crates/usage-core/src/model.rs field for field; change the contract first.

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
}

// Feedback ("Zpětná vazba"), shell commands `feedback_status` / `feedback_send`.
// The address is decided in Rust (src-tauri/src/feedback.rs); the webview only
// sends the content.

export type FeedbackKind = "bug" | "change" | "improvement";

export interface FeedbackStatus {
  available: boolean;
  host: string;                      // where the message goes, shown in the dialog
  version: string;
  os: string;
  error: string;                     // why sending is unavailable
}

export interface FeedbackRequest {
  kind: FeedbackKind;
  message: string;
  location: string;                  // e.g. "Okno aplikace › Měsíc"
  author: { name: string; email: string } | null; // null = anonymous
  attachment: { name: string; type: string; data: string } | null; // data in base64
}

export interface FeedbackReceipt { id: string }
