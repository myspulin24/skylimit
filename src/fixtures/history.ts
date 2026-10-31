// Generated history for the browser mock and the tests. Days are a function
// of the absolute day number, so every range shows the same days: weekends
// quiet, some days off, Claude split into token types, models, sources and
// (made-up) projects the way the core splits it, Codex as daily totals at a
// blended rate. Older days come "restored from T3 Code".

import type { History, HistoryFilter, HistoryKind, HistoryKindRow, HistoryPoint, HistoryRange, HistoryRow, Provider } from "../types";
import { CZK_PER_EUR, EUR_PER_USD } from "./index";

const HOUR = 3600;
const DAY = 86_400;
/** How far back the mock's history reaches. */
const HISTORY_DAYS = 150;
/** Days older than this were "restored from T3 Code". */
const RESTORED_AFTER = 34;

/** USD per token of each Claude type, and Codex's blended rate. */
const PRICE = { input: 5e-6, cacheRead: 0.5e-6, cacheWrite: 10e-6, output: 25e-6, codex: 1.7e-6 };
const SHARE = { input: 0.00002, cacheWrite: 0.0135, output: 0.0045 };
const CACHE_READ_SHARE = 1 - SHARE.input - SHARE.cacheWrite - SHARE.output;
const SHARES: [Exclude<HistoryKind, "webSearch" | "codex">, number][] = [
  ["input", SHARE.input],
  ["cacheRead", CACHE_READ_SHARE],
  ["cacheWrite", SHARE.cacheWrite],
  ["output", SHARE.output],
];
const CLAUDE_USD_PER_TOKEN = SHARES.reduce((sum, [k, s]) => sum + s * PRICE[k], 0);

const PROJECTS = ["e-shop-api", "mobilni-aplikace", "skylimit-web", "datovy-sklad", "interni-nastroje"];

const pragueDate = new Intl.DateTimeFormat("en-CA", { timeZone: "Europe/Prague", year: "numeric", month: "2-digit", day: "2-digit" });
const pragueHour = new Intl.DateTimeFormat("en-GB", { timeZone: "Europe/Prague", hour: "2-digit", hourCycle: "h23" });

const dateOf = (epochSec: number) => pragueDate.format(new Date(epochSec * 1000));
const dayNumber = (epochSec: number) => Math.floor(Date.parse(`${dateOf(epochSec)}T00:00:00Z`) / 1000 / DAY);

function claudeTokens(n: number): number {
  const weekday = new Date(n * DAY * 1000).getUTCDay();
  if (n % 11 === 3) return 0;
  const wave = 0.3 + 0.7 * Math.abs(Math.sin(n * 1.7) * Math.cos(n * 0.45));
  return Math.round((weekday === 0 || weekday === 6 ? wave * 0.15 : wave) * 420e6);
}

function codexTokens(n: number): number {
  const weekday = new Date(n * DAY * 1000).getUTCDay();
  if (weekday === 0 || weekday === 6 || n % 5 === 1) return 0;
  return Math.round((0.2 + 0.8 * Math.abs(Math.cos(n * 0.9))) * 6e6);
}

function models(age: number): [string, number][] {
  return age > 60
    ? [["claude-opus-5", 0.92], ["claude-fable-5-1", 0.06], ["claude-sonnet-5", 0.02]]
    : [["claude-opus-5-5", 0.94], ["claude-fable-5-1", 0.05], ["claude-haiku-4-5", 0.01]];
}

function sources(age: number): [string, number][] {
  return age > RESTORED_AFTER ? [["t3-cache", 1]] : [["sdk-ts", 0.86], ["cli", 0.1], ["claude-desktop", 0.04]];
}

function projects(n: number): [string, number][] {
  const w = PROJECTS.map((_, i) => 0.2 + Math.abs(Math.sin(n * 0.3 + i * 1.3)));
  const sum = w.reduce((a, b) => a + b, 0);
  return PROJECTS.map((p, i) => [p, (w[i] ?? 0) / sum]);
}

/** Activity over the hours of a Prague day: evenings and office hours. */
function hourWeight(hour: number): number {
  if (hour < 8) return 0;
  return hour >= 13 && hour <= 14 ? 0.4 : 0.6 + 0.4 * Math.abs(Math.sin(hour));
}
const HOUR_SUM = Array.from({ length: 24 }, (_, h) => hourWeight(h)).reduce((a, b) => a + b, 0);

type Acc = Map<string, { usd: number; tokens: number; provider: Provider | null }>;
const add = (acc: Acc, key: string, usd: number, tokens: number, provider: Provider | null) => {
  const e = acc.get(key) ?? { usd: 0, tokens: 0, provider };
  e.usd += usd;
  e.tokens += tokens;
  acc.set(key, e);
};

export function mockHistory(
  range: HistoryRange,
  filter: HistoryFilter,
  currency: "EUR" | "CZK",
  now: number,
  opts: { empty?: boolean; codexOff?: boolean } = {},
): History {
  const conv = EUR_PER_USD * (currency === "CZK" ? CZK_PER_EUR : 1);
  const showClaude = filter !== "codex";
  const showCodex = filter !== "claude" && !opts.codexOff;
  const today = dayNumber(now);
  const firstDay = today - HISTORY_DAYS;
  const hourly = range === "24h";
  const codexInView = showCodex && !hourly;

  const buckets: { start: number; date: string | null; day: number; share: number }[] = [];
  if (hourly) {
    const from = Math.floor(now / HOUR) * HOUR - 23 * HOUR;
    for (let i = 0; i < 24; i++) {
      const start = from + i * HOUR;
      buckets.push({ start, date: null, day: dayNumber(start), share: hourWeight(Number(pragueHour.format(new Date(start * 1000)))) / HOUR_SUM });
    }
  } else {
    const back = range === "7d" ? 6 : range === "30d" ? 29 : range === "90d" ? 89 : today - firstDay;
    for (let n = today - back; n <= today; n++) {
      buckets.push({ start: n * DAY - HOUR, date: new Date(n * DAY * 1000).toISOString().slice(0, 10), day: n, share: 1 });
    }
  }

  const byModel: Acc = new Map();
  const bySource: Acc = new Map();
  const byProject: Acc = new Map();
  const byDay: Acc = new Map();
  const kinds = new Map<HistoryKind, { usd: number; tokens: number }>();
  const series: HistoryPoint[] = [];
  let claudeUsd = 0, claudeTok = 0, codexUsd = 0, codexTok = 0, savings = 0;

  for (const b of buckets) {
    const live = !opts.empty && b.day >= firstDay;
    const age = today - b.day;
    const cTok = live && showClaude ? Math.round(claudeTokens(b.day) * b.share) : 0;
    const xTok = live && codexInView ? codexTokens(b.day) : 0;
    const cUsd = cTok * CLAUDE_USD_PER_TOKEN;
    const xUsd = xTok * PRICE.codex;
    series.push({ start: b.start, date: b.date, claudeDisplay: cUsd * conv, codexDisplay: xUsd * conv, claudeTokens: cTok, codexTokens: xTok });
    const date = new Date(b.day * DAY * 1000).toISOString().slice(0, 10);
    if (cTok > 0) {
      claudeUsd += cUsd;
      claudeTok += cTok;
      for (const [k, s] of SHARES) {
        const e = kinds.get(k) ?? { usd: 0, tokens: 0 };
        e.tokens += cTok * s;
        e.usd += cTok * s * PRICE[k];
        kinds.set(k, e);
      }
      savings += cTok * CACHE_READ_SHARE * (PRICE.input - PRICE.cacheRead);
      for (const [m, s] of models(age)) add(byModel, m, cUsd * s, cTok * s, "claude");
      for (const [m, s] of sources(age)) add(bySource, m, cUsd * s, cTok * s, "claude");
      for (const [m, s] of projects(b.day)) add(byProject, m, cUsd * s, cTok * s, "claude");
      add(byDay, date, cUsd, cTok, null);
    }
    if (xTok > 0) {
      codexUsd += xUsd;
      codexTok += xTok;
      const e = kinds.get("codex") ?? { usd: 0, tokens: 0 };
      e.tokens += xTok;
      e.usd += xUsd;
      kinds.set("codex", e);
      for (const acc of [byModel, bySource, byProject]) add(acc, "codex", xUsd, xTok, "codex");
      add(byDay, date, xUsd, xTok, null);
    }
  }

  const rows = (acc: Acc): HistoryRow[] =>
    [...acc]
      .map(([key, e]) => ({ key, provider: e.provider, display: e.usd * conv, tokens: Math.round(e.tokens) }))
      .sort((a, b) => b.display - a.display || b.tokens - a.tokens);
  const order: HistoryKind[] = ["input", "cacheRead", "cacheWrite", "output", "webSearch", "codex"];
  const kindRows: HistoryKindRow[] = order.flatMap((k) => {
    const e = kinds.get(k);
    return e ? [{ kind: k, tokens: Math.round(e.tokens), display: e.usd * conv }] : [];
  });

  const providers: History["providers"] = [];
  if (showClaude) providers.push({ provider: "claude", display: claudeUsd * conv, tokens: claudeTok, unpricedTokens: 0, estimate: false });
  if (showCodex) providers.push({ provider: "codex", display: codexUsd * conv, tokens: codexTok, unpricedTokens: 0, estimate: true });

  return {
    range,
    filter,
    currency,
    bucket: hourly ? "hour" : "day",
    from: buckets[0]?.start ?? now,
    to: now,
    firstDataAt: opts.empty ? null : firstDay * DAY - HOUR,
    activeBuckets: series.filter((p) => p.claudeTokens + p.codexTokens > 0).length,
    totalDisplay: (claudeUsd + codexUsd) * conv,
    totalTokens: claudeTok + codexTok,
    providers,
    kinds: kindRows,
    cacheSavingsDisplay: savings * conv,
    series,
    byModel: rows(byModel),
    byDay: rows(byDay).sort((a, b) => b.key.localeCompare(a.key)),
    bySource: rows(bySource),
    byProject: rows(byProject),
    codexWithoutHours: showCodex && hourly,
  };
}
