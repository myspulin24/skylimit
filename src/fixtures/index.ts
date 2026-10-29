// Sample snapshots for the browser mock and the tests. Every fixture is a
// function of "now" (epoch seconds) so that countdowns and ages stay realistic
// whenever the dev server is opened.

import type { BreakdownRow, Settings, Snapshot } from "../types";

export const FIXTURE_NAMES = [
  "normal",
  "stale",
  "codexMissing",
  "codexGone",
  "formatChanged",
  "noPlan",
  "backfill",
  "celebration",
  "panic",
  "claudeOff",
] as const;

export type FixtureName = (typeof FIXTURE_NAMES)[number];

export const FIXTURE_LABEL: Record<FixtureName, string> = {
  normal: "běžný provoz",
  stale: "zastaralé · po resetu",
  codexMissing: "Codex chybí",
  codexGone: "Codex po restartu nenalezen",
  formatChanged: "formát Claude",
  noPlan: "bez paušálu",
  backfill: "načítání historie",
  celebration: "oslava",
  panic: "panika",
  claudeOff: "bez zdroje limitů Claude",
};

export function isFixtureName(value: string | null): value is FixtureName {
  return value !== null && (FIXTURE_NAMES as readonly string[]).includes(value);
}

/** EUR per USD used by every fixture; the mock converts with the same rate. */
export const EUR_PER_USD = 0.92;
/** CZK per EUR used when the mock switches the display currency. */
export const CZK_PER_EUR = 25.2;

const MIN = 60;
const HOUR = 3600;
const DAY = 86400;

const round2 = (n: number) => Math.round(n * 100) / 100;

/**
 * Deterministic daily series for October 2026 up to the 28th. Weekends are
 * quiet, weekdays wander; the series is scaled so it sums to the totals.
 */
function makeDaily(claudeTotal: number, codexTotal: number, days = 28): Snapshot["daily"] {
  const weights = Array.from({ length: days }, (_, i) => {
    const day = i + 1;
    const weekday = new Date(Date.UTC(2026, 9, day)).getUTCDay();
    const weekend = weekday === 0 || weekday === 6;
    const wave = 0.35 + 0.65 * Math.abs(Math.sin(day * 1.7) * Math.cos(day * 0.45));
    return weekend ? wave * 0.22 : wave;
  });
  const codexWeights = weights.map((w, i) => (i % 3 === 1 || i > 20 ? w * (0.6 + (i % 4) * 0.3) : 0));
  const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0) || 1;
  const cw = sum(weights);
  const xw = sum(codexWeights);
  return weights.map((w, i) => ({
    date: `2026-10-${String(i + 1).padStart(2, "0")}`,
    claudeDisplay: round2((w / cw) * claudeTotal),
    codexDisplay: round2(((codexWeights[i] ?? 0) / xw) * codexTotal),
  }));
}

function row(key: string, display: number, tokens: number): BreakdownRow {
  return { key, usd: round2(display / EUR_PER_USD), display, tokens };
}

/** Normal operation: Claude limits from T3 Code's log, Codex limits from app-server. */
function normal(now: number): Snapshot {
  const claudePlan = 184; // 200 USD
  const codexPlan = 18.4; // 20 USD
  return {
    generatedAt: now,
    month: "2026-10",
    currency: "EUR",
    limits: [
      {
        provider: "claude",
        state: "known",
        governing: 34,
        windows: [
          { kind: "5h", label: "5h", percent: 18, resetsAt: now + 3 * HOUR + 20 * MIN, fetchedAt: now - 4 * MIN, state: "known" },
          { kind: "week", label: "týden", percent: 34, resetsAt: now + 5 * DAY + 2 * HOUR, fetchedAt: now - 4 * MIN, state: "known" },
          { kind: "weekModel", label: "Fable", percent: 9, resetsAt: now + 5 * DAY + 2 * HOUR, fetchedAt: now - 6 * MIN, state: "known" },
        ],
        note: null,
      },
      {
        provider: "codex",
        state: "known",
        governing: 61,
        windows: [
          { kind: "5h", label: "5h", percent: 42, resetsAt: now + 100 * MIN, fetchedAt: now - 70, state: "known" },
          { kind: "week", label: "týden", percent: 61, resetsAt: now + 4 * DAY + 6 * HOUR, fetchedAt: now - 70, state: "known" },
        ],
        note: null,
      },
    ],
    scores: [
      {
        provider: "claude",
        usd: round2(841 / EUR_PER_USD),
        display: 841,
        tokens: 1_412_300_000,
        unpricedTokens: 0,
        state: "ok",
        planMonthlyDisplay: claudePlan,
        multiple: 841 / claudePlan,
      },
      {
        provider: "codex",
        usd: round2(89 / EUR_PER_USD),
        display: 89,
        tokens: 61_250_000,
        unpricedTokens: 0,
        state: "estimate",
        planMonthlyDisplay: codexPlan,
        multiple: 89 / codexPlan,
      },
    ],
    total: { display: 930, multiple: 930 / (claudePlan + codexPlan), planMonthlyDisplay: claudePlan + codexPlan, excludedNote: null },
    moods: [
      { provider: "claude", mood: "focus" },
      { provider: "codex", mood: "calm" },
    ],
    tray: "green",
    daily: makeDaily(841, 89),
    bySource: [row("sdk-ts", 612, 1_021_000_000), row("cli", 188, 318_400_000), row("codex", 89, 61_250_000), row("claude-desktop", 41, 72_900_000)],
    byModel: [row("claude-opus-5-5", 701, 1_050_600_000), row("claude-sonnet-5-5", 140, 361_700_000), row("gpt-6-codex", 89, 61_250_000)],
    byProject: [
      row("e-shop-api", 210, 310_000_000),
      row("Rezervační systém pro kliniky", 150, 228_000_000),
      row("web-katalog", 120, 185_000_000),
      row("fakturace", 100, 163_000_000),
      row("Datový sklad", 90, 146_000_000),
      row("mobilní-aplikace", 80, 133_000_000),
      row("landing-page", 60, 101_000_000),
      row("interní-wiki", 50, 84_000_000),
      row("design-system", 40, 68_000_000),
      row("reporting", 30, 51_000_000),
    ],
    streaks: [
      { provider: "claude", current: 12, longest: 19 },
      { provider: "codex", current: 4, longest: 9 },
    ],
    backfill: { done: true, filesDone: 85, filesTotal: 85 },
    celebration: null,
    updatedAgoSec: 120,
    widgetTransparency: 0,
  };
}

/** Codex app-server stopped answering: 5h window stale, week window past its reset.
 *  Claude ran where no source reports limits (Claude Desktop): its number is stale too. */
function stale(now: number): Snapshot {
  const s = normal(now);
  s.limits[0] = {
    provider: "claude",
    state: "stale",
    governing: 34,
    windows: [
      { kind: "5h", label: "5h", percent: 22, resetsAt: now + 2 * HOUR, fetchedAt: now - 52 * MIN, state: "stale" },
      { kind: "week", label: "týden", percent: 34, resetsAt: now + 5 * DAY, fetchedAt: now - 52 * MIN, state: "stale" },
      { kind: "weekModel", label: "Fable", percent: 9, resetsAt: now + 5 * DAY, fetchedAt: now - 52 * MIN, state: "known" },
    ],
    note: "Claude od té doby běžel mimo terminál i T3 Code",
  };
  s.limits[1] = {
    provider: "codex",
    state: "stale",
    governing: 64,
    windows: [
      { kind: "5h", label: "5h", percent: 64, resetsAt: now + 70 * MIN, fetchedAt: now - 47 * MIN, state: "stale" },
      { kind: "week", label: "týden", percent: null, resetsAt: now - 2 * HOUR, fetchedAt: now - 2 * DAY, state: "afterReset" },
    ],
    note: "Codex app-server neodpovídá. Čísla se obnoví sama, až se vrátí.",
  };
  s.moods = [
    { provider: "claude", mood: "sleep" },
    { provider: "codex", mood: "dozing" },
  ];
  s.tray = "greenDim";
  s.updatedAgoSec = 47 * MIN;
  return s;
}

/** Codex worked before, then the app-server is not found (macOS app relaunched
 *  without the terminal PATH): last numbers from the database plus a note. The
 *  tallest limits block the widget has to fit. */
function codexGone(now: number): Snapshot {
  const s = normal(now);
  s.limits[1] = {
    provider: "codex",
    state: "stale",
    governing: 13,
    windows: [
      { kind: "5h", label: "5h", percent: 13, resetsAt: now + 3 * HOUR, fetchedAt: now - 11 * MIN, state: "stale" },
      { kind: "week", label: "týden", percent: 2, resetsAt: now + 6 * DAY, fetchedAt: now - 11 * MIN, state: "stale" },
    ],
    note: "Codex CLI nenalezen",
  };
  s.moods = [
    { provider: "claude", mood: "sweat" },
    { provider: "codex", mood: "dozing" },
  ];
  return s;
}

/** Codex CLI is not installed: limits unknown, no Codex price. */
function codexMissing(now: number): Snapshot {
  const s = normal(now);
  s.limits[1] = {
    provider: "codex",
    state: "unknown",
    governing: null,
    windows: [],
    note: "Codex CLI nenalezen v PATH. Limity Codexu se nezobrazí, skóre Claude jde dál.",
  };
  s.scores[1] = {
    provider: "codex",
    usd: 0,
    display: 0,
    tokens: 0,
    unpricedTokens: 0,
    state: "unknown",
    planMonthlyDisplay: 18.4,
    multiple: null,
  };
  s.total = { display: 841, multiple: 841 / 184, planMonthlyDisplay: 184, excludedNote: "Codex bez ceny, paušál nezapočten" };
  s.moods = [
    { provider: "claude", mood: "focus" },
    { provider: "codex", mood: "confused" },
  ];
  s.tray = "grey";
  s.daily = makeDaily(841, 0);
  s.bySource = s.bySource.filter((r) => r.key !== "codex");
  s.byModel = s.byModel.filter((r) => !r.key.startsWith("gpt"));
  s.streaks = [
    { provider: "claude", current: 12, longest: 19 },
    { provider: "codex", current: 0, longest: 9 },
  ];
  return s;
}

/** Claude Code changed its JSONL format: Claude price is unknown. */
function formatChanged(now: number): Snapshot {
  const s = normal(now);
  s.scores[0] = {
    provider: "claude",
    usd: round2(702 / EUR_PER_USD),
    display: 702,
    tokens: 1_180_000_000,
    unpricedTokens: 0,
    state: "formatChanged",
    planMonthlyDisplay: 184,
    multiple: null,
  };
  s.total = { display: 791, multiple: 89 / 18.4, planMonthlyDisplay: 18.4, excludedNote: "Claude bez ceny, paušál nezapočten" };
  s.moods = [
    { provider: "claude", mood: "confused" },
    { provider: "codex", mood: "focus" },
  ];
  return s;
}

/** No flat fee entered yet: no multiples, the widget asks for the plan. */
function noPlan(now: number): Snapshot {
  const s = normal(now);
  s.scores = s.scores.map((sc) => ({ ...sc, planMonthlyDisplay: null, multiple: null }));
  s.total = { display: 930, multiple: null, planMonthlyDisplay: null, excludedNote: null };
  return s;
}

/** First pass over 1,6 GB of history is still running. */
function backfill(now: number): Snapshot {
  const s = normal(now);
  s.scores[0] = { ...s.scores[0]!, usd: round2(214 / EUR_PER_USD), display: 214, tokens: 352_000_000, multiple: 214 / 184 };
  s.total = { display: 303, multiple: 303 / 202.4, planMonthlyDisplay: 202.4, excludedNote: null };
  s.backfill = { done: false, filesDone: 12, filesTotal: 85 };
  s.daily = makeDaily(214, 89);
  s.moods = [
    { provider: "claude", mood: "focus" },
    { provider: "codex", mood: "focus" },
  ];
  s.updatedAgoSec = 3;
  return s;
}

/** Milestone reached: both creatures celebrate for a few seconds. */
function celebration(now: number): Snapshot {
  const s = normal(now);
  s.scores[0] = { ...s.scores[0]!, usd: round2(923 / EUR_PER_USD), display: 923, multiple: 923 / 184 };
  s.total = { display: 1012, multiple: 1012 / 202.4, planMonthlyDisplay: 202.4, excludedNote: null };
  s.celebration = { id: `multiple-5-${now}`, who: "both", text: "Tento měsíc už 5× paušálu!" };
  s.moods = [
    { provider: "claude", mood: "celebrate" },
    { provider: "codex", mood: "celebrate" },
  ];
  s.updatedAgoSec = 1;
  return s;
}

/** Codex 5h window almost exhausted, Claude's getting there. */
function panic(now: number): Snapshot {
  const s = normal(now);
  s.limits[0] = {
    provider: "claude",
    state: "known",
    governing: 78,
    windows: [
      { kind: "5h", label: "5h", percent: 78, resetsAt: now + 52 * MIN, fetchedAt: now - 90, state: "known" },
      { kind: "week", label: "týden", percent: 41, resetsAt: now + 5 * DAY, fetchedAt: now - 90, state: "known" },
    ],
    note: null,
  };
  s.limits[1] = {
    provider: "codex",
    state: "known",
    governing: 97,
    windows: [
      { kind: "5h", label: "5h", percent: 97, resetsAt: now + 38 * MIN, fetchedAt: now - 40, state: "known" },
      { kind: "week", label: "týden", percent: 81, resetsAt: now + 2 * DAY + 3 * HOUR, fetchedAt: now - 40, state: "known" },
    ],
    note: null,
  };
  s.moods = [
    { provider: "claude", mood: "sweat" },
    { provider: "codex", mood: "panic" },
  ];
  s.tray = "red";
  s.updatedAgoSec = 40;
  return s;
}

/** Neither Claude Code connected nor T3 Code: Claude limits have no source, mood from activity only. */
function claudeOff(now: number): Snapshot {
  const s = normal(now);
  s.limits[0] = { provider: "claude", state: "off", governing: null, windows: [], note: "limity Claude: připoj Claude Code v Nastavení" };
  return s;
}

export const fixtures: Record<FixtureName, (now: number) => Snapshot> = {
  normal,
  stale,
  codexMissing,
  codexGone,
  formatChanged,
  noPlan,
  backfill,
  celebration,
  panic,
  claudeOff,
};

export function defaultSettings(): Settings {
  return {
    currency: "EUR",
    plans: {
      claude: { amount: 200, currency: "USD", period: "month" },
      codex: { amount: 20, currency: "USD", period: "month" },
    },
    codexEstimateInScore: true,
    excludedEntrypoints: [],
    clickThrough: false,
    widgetTransparency: 0,
    checkUpdates: true,
  };
}

/** Settings that match a fixture (the no-plan fixture has no plans). */
export function fixtureSettings(name: FixtureName): Settings {
  const s = defaultSettings();
  if (name === "noPlan") s.plans = { claude: null, codex: null };
  return s;
}

export const nowSec = () => Math.floor(Date.now() / 1000);
