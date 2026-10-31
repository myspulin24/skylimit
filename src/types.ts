// Copied verbatim from docs/snapshot-contract.md. Must match
// crates/usage-core/src/model.rs field for field; change the contract first.

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

// Updates from GitHub Releases, shell commands `update_status` / `update_check`
// / `update_install` and the event "update" (src-tauri/src/updater.rs).

export type UpdatePhase = "idle" | "checking" | "upToDate" | "available" | "downloading" | "installing" | "error";

export interface UpdateStatus {
  phase: UpdatePhase;
  current: string;                   // running version
  version: string | null;            // version on offer
  notes: string | null;              // release notes (Markdown from CHANGELOG.md)
  date: number | null;               // release date, epoch s
  progress: number | null;           // 0..1 while downloading
  error: string | null;              // plain-language reason (manual check or install)
  checkedAt: number | null;          // epoch s of the last finished check
  auto: boolean;                     // automatic checks on (setting and environment)
}

/** Settings › Widget: the system starts Skylimit at login (its own startup list). */
export interface AutostartStatus {
  /** "moved": the system starts a Skylimit at another path; turning it on switches to this one. */
  state: "on" | "off" | "moved";
  problem: string;                   // why it cannot be turned on (macOS app not in Applications)
}

/** Settings › Claude Code: Skylimit as Claude Code's status line (limits from any terminal). */
export interface ClaudeCodeStatus {
  /** "missing": no ~/.claude here; "moved": connected to a Skylimit at another path. */
  state: "missing" | "off" | "on" | "moved";
  settingsPath: string;              // Claude Code's settings.json
  ownStatusLine: boolean;            // the user's own status line keeps running inside Skylimit's
  lastReadingAt: number | null;      // when Claude Code last delivered limits (epoch s)
  problem: string;                   // why connecting cannot work (macOS app not in Applications)
}

// Settings › Účty, shell command `accounts` (src-tauri/src/commands.rs). Enum
// values stay raw (plans, roles, tiers); format.ts labels them. Never part of
// the snapshot, so the e-mails never reach the widget.

/** The account Claude Code is signed in with (`oauthAccount` in its .claude.json). */
export interface ClaudeAccount {
  email: string | null;
  name: string | null;
  organization: string | null;
  organizationType: string | null;   // "claude_max", "claude_team", …
  role: string | null;               // "admin", "membership_admin", …
  seatTier: string | null;           // "team_tier_1", …
  rateLimitTier: string | null;      // "default_claude_max_5x", …
  billing: string | null;            // "stripe_subscription", …
  accountCreatedAt: number | null;   // epoch s
  subscriptionCreatedAt: number | null;
  extraUsage: boolean | null;        // extra usage past the plan's limits switched on
  extraUsageBlocked: string | null;  // why it cannot be used now ("out_of_credits", …)
  profileFetchedAt: number | null;   // when Claude Code last refreshed the profile
  limitsFetchedAt: number | null;    // … and the limits
}

export interface CodexCredits {
  hasCredits: boolean;
  unlimited: boolean;
  balance: string | null;
}

/** The account the Codex CLI is signed in with (app-server `account/read`). */
export interface CodexAccount {
  kind: string;                      // "chatgpt" | "apiKey" | "amazonBedrock" | "gcp" | "none"
  email: string | null;
  plan: string | null;               // "plus", "pro", "team", …
  workspace: string | null;
  cliVersion: string | null;
  resetCredits: number | null;       // limit resets still available
  credits: CodexCredits | null;
  lifetimeTokens: number | null;
  readAt: number;                    // epoch s
}

export interface Accounts {
  claude: ClaudeAccount | null;
  codex: CodexAccount | null;
  codexNote: string | null;          // why there is no Codex account ("Codex CLI nenalezen", …)
}

// History tab, shell command `history` (src-tauri/src/commands.rs). Priced by
// the core on demand from stored tokens; never part of the snapshot.

/** 24 h goes by hours, the rest by calendar days (Europe/Prague), today included. */
export type HistoryRange = "24h" | "7d" | "30d" | "90d" | "all";
export type HistoryFilter = "all" | "claude" | "codex";
/** "codex": Codex reports daily totals only, without types; "webSearch" carries no tokens. */
export type HistoryKind = "input" | "cacheRead" | "cacheWrite" | "output" | "webSearch" | "codex";

export interface HistoryKindRow { kind: HistoryKind; tokens: number; display: number }

export interface HistoryProvider {
  provider: Provider;
  display: number;
  tokens: number;
  unpricedTokens: number;            // tokens of models without a price (never priced as 0)
  estimate: boolean;                 // Codex: daily totals × blended rate
}

/** One bar: an hour (24 h) or a day. */
export interface HistoryPoint {
  start: number;                     // epoch s of the bucket's start (local midnight for days)
  date: string | null;               // "2026-10-07" for day buckets
  claudeDisplay: number;
  codexDisplay: number;
  claudeTokens: number;
  codexTokens: number;
}

/** key = model, entrypoint, project folder or "2026-10-07"; "codex" is Codex's one row. */
export interface HistoryRow { key: string; provider: Provider | null; display: number; tokens: number }

export interface History {
  range: HistoryRange;
  filter: HistoryFilter;
  currency: "EUR" | "CZK";
  bucket: "hour" | "day";
  from: number;                      // start of the first bucket, epoch s
  to: number;                        // time of the query
  firstDataAt: number | null;        // oldest record of the shown providers
  activeBuckets: number;             // days (or hours in 24 h) with any tokens
  totalDisplay: number;
  totalTokens: number;
  providers: HistoryProvider[];      // claude first; only the shown ones
  kinds: HistoryKindRow[];           // input, cacheRead, cacheWrite, output, webSearch, codex (present ones)
  cacheSavingsDisplay: number;       // cache reads against the full input price
  series: HistoryPoint[];
  byModel: HistoryRow[];
  byDay: HistoryRow[];               // newest first
  bySource: HistoryRow[];            // "t3-cache" = restored from T3 Code's usage cache
  byProject: HistoryRow[];
  codexWithoutHours: boolean;        // 24 h with Codex on: Codex reports days, not hours
}
