// Czech formatting helpers. All UI numbers go through here so that the widget,
// the app window and the tests agree on spacing, decimals and wording.

import type { CodexCredits, Mood, Provider, ScoreState, Snapshot } from "./types";

/** Non-breaking space: Czech typography keeps a number and its unit together. */
export const NBSP = "\u00a0";

const moneyCache = new Map<string, Intl.NumberFormat>();

function moneyFormat(currency: string, digits: number): Intl.NumberFormat {
  const key = `${currency}:${digits}`;
  let f = moneyCache.get(key);
  if (!f) {
    f = new Intl.NumberFormat("cs-CZ", {
      style: "currency",
      currency,
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    });
    moneyCache.set(key, f);
  }
  return f;
}

/** "930 €", "23 436 Kč"; amounts under 10 keep cents ("8,40 €"). */
export function formatMoney(value: number, currency: Snapshot["currency"] | "USD"): string {
  const abs = Math.abs(value);
  const digits = abs === 0 || abs >= 10 ? 0 : 2;
  return moneyFormat(currency, digits).format(value);
}

/** Axis tick: whole amounts without cents ("5 €"), the rest like formatMoney. */
export function formatMoneyTick(value: number, currency: Snapshot["currency"] | "USD"): string {
  return Number.isInteger(value) ? moneyFormat(currency, 0).format(value) : formatMoney(value, currency);
}

const multipleFormat = new Intl.NumberFormat("cs-CZ", {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

/** "4,6×" */
export function formatMultiple(multiple: number): string {
  return `${multipleFormat.format(multiple)}×`;
}

/** "42 %" */
export function formatPercent(percent: number): string {
  return `${Math.round(percent)}${NBSP}%`;
}

const decimal1 = new Intl.NumberFormat("cs-CZ", { maximumFractionDigits: 1 });
const integer = new Intl.NumberFormat("cs-CZ", { maximumFractionDigits: 0 });

/** "12,4 mil.", "845 tis.", "1,2 mld." */
export function formatTokens(tokens: number): string {
  if (tokens >= 1e9) return `${decimal1.format(tokens / 1e9)}${NBSP}mld.`;
  if (tokens >= 1e6) return `${decimal1.format(tokens / 1e6)}${NBSP}mil.`;
  if (tokens >= 1e3) return `${integer.format(tokens / 1e3)}${NBSP}tis.`;
  return integer.format(tokens);
}

/** Positive duration, rounded to minutes: "40 min", "1 h 40 min", "4 d 6 h". */
export function formatDuration(seconds: number): string {
  const totalMin = Math.round(Math.max(0, seconds) / 60);
  if (totalMin < 1) return `<1${NBSP}min`;
  if (totalMin < 60) return `${totalMin}${NBSP}min`;
  if (totalMin < 24 * 60) {
    const h = Math.floor(totalMin / 60);
    const m = totalMin % 60;
    return m ? `${h}${NBSP}h ${m}${NBSP}min` : `${h}${NBSP}h`;
  }
  let d = Math.floor(totalMin / 1440);
  let h = Math.round((totalMin % 1440) / 60);
  if (h === 24) {
    d += 1;
    h = 0;
  }
  return h ? `${d}${NBSP}d ${h}${NBSP}h` : `${d}${NBSP}d`;
}

/** "reset za 1 h 40 min" from an epoch-seconds reset time. */
export function formatCountdown(resetsAt: number, nowSec: number): string {
  const left = resetsAt - nowSec;
  if (left <= 30) return "reset právě teď";
  return `reset za ${formatDuration(left)}`;
}

/** "právě teď", "před 2 min", "před 3 h", "před 2 d". */
export function formatAgo(seconds: number): string {
  const s = Math.max(0, seconds);
  if (s < 45) return "právě teď";
  if (s < 3600) return `před ${Math.max(1, Math.round(s / 60))}${NBSP}min`;
  if (s < 86400) return `před ${Math.round(s / 3600)}${NBSP}h`;
  return `před ${Math.round(s / 86400)}${NBSP}d`;
}

/** Czech plural: 1 den, 2–4 dny, 5+ dní (and 0 dní). */
export function plural(n: number, one: string, few: string, many: string): string {
  const abs = Math.abs(n);
  if (abs === 1) return one;
  if (abs >= 2 && abs <= 4) return few;
  return many;
}

export function formatDays(n: number): string {
  return `${n}${NBSP}${plural(n, "den", "dny", "dní")}`;
}

export type BarTone = "ok" | "warn" | "crit";

/** Same thresholds as the tray: green < 75, amber ≥ 75, red ≥ 95. */
export function barTone(percent: number): BarTone {
  if (percent >= 95) return "crit";
  if (percent >= 75) return "warn";
  return "ok";
}

export const PROVIDER_NAME: Record<Provider, string> = {
  claude: "Claude",
  codex: "Codex",
};

/** Genitive for sentences like "limity Claude vypnuté" / "limity Codexu vypnuté". */
export const PROVIDER_GENITIVE: Record<Provider, string> = {
  claude: "Claude",
  codex: "Codexu",
};

export const CREATURE_NAME: Record<Provider, string> = {
  claude: "Sunnie",
  codex: "Dropie",
};

/** Mood names as used in the design doc; also the mascot aria-label. */
export const MOOD_LABEL: Record<Mood, string> = {
  calm: "klid",
  focus: "soustředění",
  sweat: "pot",
  panic: "panika",
  celebrate: "oslava",
  dozing: "dřímání",
  sleep: "spánek",
  confused: "zmatek",
};

/** Short caption under each mascot in the widget. */
export const MOOD_CAPTION: Record<Provider, Record<Mood, string>> = {
  claude: {
    calm: "v klidu",
    focus: "počítá",
    sweat: "potí se",
    panic: "panikaří",
    celebrate: "slaví",
    dozing: "dřímá",
    sleep: "spí",
    confused: "nechápe",
  },
  codex: {
    calm: "pije kafe",
    focus: "píše kód",
    sweat: "potí se",
    panic: "panikaří",
    celebrate: "slaví",
    dozing: "dřímá",
    sleep: "spí",
    confused: "nechápe",
  },
};

/** One-line reason per mood (widget tooltip); the full rules live in the "Stavy" tab. */
export const MOOD_HINT: Record<Mood, string> = {
  confused: "nevím, kolik limitu zbývá",
  dozing: "znám jen starší číslo limitu",
  panic: "limit je na 95 % a víc",
  sweat: "limit je na 75 % a víc",
  celebrate: "nový milník",
  focus: "práce v posledních 5 minutách",
  sleep: "30 minut bez práce",
  calm: "vše v pořádku",
};

const SOURCE_LABEL: Record<string, string> = {
  cli: "Terminál",
  "sdk-ts": "T3 Code a další SDK",
  "claude-desktop": "Claude Desktop",
  codex: "Codex",
  "t3-cache": "Obnoveno z T3 Code",
  unknown: "Neznámý zdroj",
};

/** Entrypoint key → readable label; unknown keys are shown as they are. */
export function sourceLabel(key: string): string {
  return SOURCE_LABEL[key] ?? key;
}

const MONTHS = [
  "leden", "únor", "březen", "duben", "květen", "červen",
  "červenec", "srpen", "září", "říjen", "listopad", "prosinec",
];

/** "2026-10" → "říjen 2026" */
export function monthLabel(month: string): string {
  const [y, m] = month.split("-");
  const name = MONTHS[Number(m) - 1];
  return name ? `${name} ${y}` : month;
}

/** "2026-10" → "říjen" */
export function monthName(month: string): string {
  const name = MONTHS[Number(month.split("-")[1]) - 1];
  return name ?? month;
}

/** "2026-10-07" → "7. 10." */
export function shortDate(date: string): string {
  const [, m, d] = date.split("-");
  return `${Number(d)}.${NBSP}${Number(m)}.`;
}

const WEEKDAYS = ["ne", "po", "út", "st", "čt", "pá", "so"];

/** "2026-10-31" → "so 31. 10." (with the year when asked: "so 31. 10. 2026"). */
export function dayLabel(date: string, withYear = false): string {
  const [y, m, d] = date.split("-").map(Number);
  if (!y || !m || !d) return date;
  const weekday = WEEKDAYS[new Date(Date.UTC(y, m - 1, d)).getUTCDay()];
  return `${weekday} ${shortDate(date)}${withYear ? `${NBSP}${y}` : ""}`;
}

const hourFormat = new Intl.DateTimeFormat("cs-CZ", { timeZone: "Europe/Prague", hour: "numeric", minute: "2-digit", hourCycle: "h23" });

/** Epoch seconds → "14:00" (Prague). */
export function hourLabel(epochSec: number): string {
  return hourFormat.format(new Date(epochSec * 1000));
}

/** Score states that make the number uncertain enough to show a "?" on it. */
export function isUncertainScore(state: ScoreState): boolean {
  return state === "unknown" || state === "formatChanged";
}

// ---------- Settings › Účty: raw values from Claude Code and Codex in Czech

/** "28. 3. 2026" from epoch seconds (Prague calendar). */
export function formatDate(epochSec: number): string {
  const d = new Date(epochSec * 1000);
  const parts = new Intl.DateTimeFormat("cs-CZ", { timeZone: "Europe/Prague", day: "numeric", month: "numeric", year: "numeric" })
    .formatToParts(d)
    .filter((p) => p.type !== "literal")
    .map((p) => p.value);
  return parts.join(`.${NBSP}`);
}

/** Unknown values stay readable instead of hidden: "team_tier_1" → "team tier 1". */
export function humanize(raw: string): string {
  return raw.replace(/[_-]+/g, " ").trim();
}

const CLAUDE_PLAN: Record<string, string> = {
  claude_free: "Free",
  claude_pro: "Pro",
  claude_max: "Max",
  claude_team: "Team",
  claude_enterprise: "Enterprise",
};

/** Claude Code's `organizationType` → plan name ("claude_team" → "Team"). */
export function claudePlanLabel(organizationType: string | null): string | null {
  if (!organizationType) return null;
  return CLAUDE_PLAN[organizationType] ?? humanize(organizationType.replace(/^claude_/, ""));
}

/** The usage tier behind the plan: "default_claude_max_5x" → "Max 5×" (a Team premium seat has one too). */
export function claudeTierLabel(tier: string | null): string | null {
  const m = tier ? /max_(\d+)x/.exec(tier) : null;
  return m ? `Max ${m[1]}×` : null;
}

const CLAUDE_ROLE: Record<string, string> = {
  primary_owner: "hlavní vlastník",
  owner: "vlastník",
  admin: "správce",
  membership_admin: "správce členů",
  billing: "fakturace",
  developer: "vývojář",
  claude_code_user: "uživatel Claude Code",
  user: "člen",
  member: "člen",
};

export function claudeRoleLabel(role: string | null): string | null {
  return role ? (CLAUDE_ROLE[role] ?? humanize(role)) : null;
}

const BILLING: Record<string, string> = {
  stripe_subscription: "přes web (Stripe)",
  apple_subscription: "přes App Store",
  google_play_subscription: "přes Google Play",
  aws_marketplace: "přes AWS Marketplace",
};

export function billingLabel(billing: string | null): string | null {
  return billing ? (BILLING[billing] ?? humanize(billing)) : null;
}

const EXTRA_BLOCKED: Record<string, string> = {
  out_of_credits: "kredit vyčerpaný",
  org_level_disabled: "vypnuté organizací",
  member_level_disabled: "vypnuté pro tvůj účet",
  overage_not_provisioned: "pro účet není k dispozici",
};

/** Extra usage past the plan's limits: "zapnuté, kredit vyčerpaný". */
export function extraUsageLabel(on: boolean | null, blocked: string | null): string | null {
  if (on === null) return null;
  if (!on) return "vypnuté";
  return blocked ? `zapnuté, ${EXTRA_BLOCKED[blocked] ?? humanize(blocked)}` : "zapnuté";
}

const CODEX_PLAN: Record<string, string> = {
  free: "Free",
  go: "Go",
  plus: "Plus",
  pro: "Pro",
  prolite: "Pro Lite",
  promax: "Pro Max",
  team: "Team",
  business: "Business",
  enterprise: "Enterprise",
  edu: "Edu",
};

/** ChatGPT `planType` → plan name ("team" → "Team"); "unknown" says nothing. */
export function codexPlanLabel(plan: string | null): string | null {
  if (!plan || plan === "unknown") return null;
  return CODEX_PLAN[plan] ?? humanize(plan);
}

const CODEX_KIND: Record<string, string> = {
  chatgpt: "účet ChatGPT",
  apiKey: "API klíč OpenAI",
  amazonBedrock: "Amazon Bedrock",
  gcp: "Google Cloud Vertex AI",
  none: "nepřihlášeno",
};

export function codexKindLabel(kind: string): string {
  return CODEX_KIND[kind] ?? humanize(kind);
}

/** Prepaid Codex credits: "neomezené", "žádné", or the balance as Codex reports it. */
export function codexCreditsLabel(credits: CodexCredits | null): string | null {
  if (!credits) return null;
  if (credits.unlimited) return "neomezené";
  if (!credits.hasCredits) return "žádné";
  return credits.balance ?? "ano";
}

/** "1 reset", "2 resety", "5 resetů" */
export function formatResets(n: number): string {
  return `${n}${NBSP}${plural(n, "reset", "resety", "resetů")}`;
}
