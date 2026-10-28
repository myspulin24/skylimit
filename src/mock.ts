// In-memory stand-in for the Tauri shell, used only when the UI runs in a plain
// browser (`npm run dev`). Pick a state with `?mock=<fixture>`; settings saved
// in the app window are applied to the snapshot of that tab so the flow can be
// clicked through without the Rust core.

import {
  CZK_PER_EUR,
  EUR_PER_USD,
  fixtureSettings,
  fixtures,
  isFixtureName,
  nowSec,
  type FixtureName,
} from "./fixtures";
import { PROVIDER_NAME } from "./format";
import type { FeedbackReceipt, FeedbackRequest, FeedbackStatus, PlanSetting, ProviderScore, Settings, Snapshot } from "./types";

const clone = <T,>(value: T): T => structuredClone(value);

function readFixtureName(): FixtureName {
  const raw = typeof location === "undefined" ? null : new URLSearchParams(location.search).get("mock");
  return isFixtureName(raw) ? raw : "normal";
}

function planToEur(plan: PlanSetting): number {
  const amount =
    plan.currency === "USD" ? plan.amount * EUR_PER_USD : plan.currency === "CZK" ? plan.amount / CZK_PER_EUR : plan.amount;
  return plan.period === "year" ? amount / 12 : amount;
}

const priced = (s: ProviderScore) => s.state === "ok" || s.state === "estimate" || s.state === "partial";

/** Re-derives display money, plans and multiples the way the core would. */
function applySettings(snapshot: Snapshot, prefs: Settings): Snapshot {
  const s = clone(snapshot);
  const factor = prefs.currency === "CZK" ? CZK_PER_EUR : 1;
  s.currency = prefs.currency;
  s.widgetTransparency = prefs.widgetTransparency;
  s.daily = s.daily.map((d) => ({ ...d, claudeDisplay: d.claudeDisplay * factor, codexDisplay: d.codexDisplay * factor }));
  for (const list of [s.bySource, s.byModel, s.byProject]) for (const r of list) r.display *= factor;

  const notes: string[] = [];
  let total = 0;
  let pricedTotal = 0;
  let planTotal = 0;
  for (const sc of s.scores) {
    sc.display *= factor;
    const plan = prefs.plans[sc.provider];
    sc.planMonthlyDisplay = plan ? planToEur(plan) * factor : null;
    sc.multiple = sc.planMonthlyDisplay && priced(sc) ? sc.display / sc.planMonthlyDisplay : null;

    const estimateExcluded = sc.provider === "codex" && sc.state === "estimate" && !prefs.codexEstimateInScore;
    if (sc.state === "off" || estimateExcluded) {
      if (estimateExcluded) notes.push("odhad Codexu nezapočten");
      continue;
    }
    total += sc.display;
    if (priced(sc) && sc.planMonthlyDisplay) {
      pricedTotal += sc.display;
      planTotal += sc.planMonthlyDisplay;
    } else if (!priced(sc) && sc.planMonthlyDisplay) {
      notes.push(`${PROVIDER_NAME[sc.provider]} bez ceny, paušál nezapočten`);
    }
  }
  s.total = {
    display: total,
    multiple: planTotal > 0 ? pricedTotal / planTotal : null,
    planMonthlyDisplay: planTotal > 0 ? planTotal : null,
    excludedNote: notes.length ? notes.join(" · ") : null,
  };
  return s;
}

export const fixtureName: FixtureName = readFixtureName();

let base: Snapshot = fixtures[fixtureName](nowSec());
let settings: Settings = fixtureSettings(fixtureName);
// `?wt=60` previews a see-through widget without opening the settings.
const wtParam = typeof location === "undefined" ? null : new URLSearchParams(location.search).get("wt");
if (wtParam !== null && Number.isFinite(Number(wtParam))) settings.widgetTransparency = Math.min(90, Math.max(0, Number(wtParam)));
let current: Snapshot = applySettings(base, settings);
const listeners = new Set<(s: Snapshot) => void>();

function emit() {
  current = applySettings(base, settings);
  for (const cb of listeners) cb(clone(current));
}

export async function getSnapshot(): Promise<Snapshot> {
  return clone(current);
}

export async function getSettings(): Promise<Settings> {
  return clone(settings);
}

export async function setSettings(next: Settings): Promise<void> {
  settings = clone(next);
  emit();
}

export async function setClickThrough(on: boolean): Promise<void> {
  settings = { ...settings, clickThrough: on };
  console.info(`[mock] průchod kliknutí: ${on ? "zapnutý" : "vypnutý"}`);
}

export async function setWidgetTransparency(value: number): Promise<void> {
  settings = { ...settings, widgetTransparency: value };
  emit();
}

export async function refreshUsage(): Promise<void> {
  // Same fixture, read "just now": countdowns and ages restart.
  base = { ...fixtures[fixtureName](nowSec()), widgetTransparency: base.widgetTransparency, updatedAgoSec: 0 };
  window.setTimeout(emit, 600);
}

export async function feedbackStatus(): Promise<FeedbackStatus> {
  return { available: true, host: "skylimit-feedback.michal-jasek.workers.dev", version: "0.4.0", os: "web (mock)", error: "" };
}

/** Nothing leaves the browser in mock mode; the dialog sees a receipt. */
export async function sendFeedback(request: FeedbackRequest): Promise<FeedbackReceipt> {
  console.info("[mock] zpětná vazba (neodesláno):", request);
  await new Promise((r) => setTimeout(r, 500));
  return { id: Math.random().toString(16).slice(2, 10) };
}

export async function openApp(): Promise<void> {
  const url = new URL(location.href);
  url.searchParams.set("window", "app");
  window.open(url.toString(), "_blank");
}

export async function clearHistory(): Promise<void> {
  base = clone(base);
  base.daily = base.daily.map((d) => ({ ...d, claudeDisplay: 0, codexDisplay: 0 }));
  base.bySource = [];
  base.byModel = [];
  base.byProject = [];
  base.scores = base.scores.map((sc) => ({ ...sc, usd: 0, display: 0, tokens: 0 }));
  base.streaks = base.streaks.map((st) => ({ ...st, current: 0, longest: 0 }));
  emit();
}

export function subscribe(cb: (s: Snapshot) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

// The celebration fixture fires a fresh milestone every 12 s so the burst can be
// watched repeatedly; the Rust core sends one id per milestone.
if (fixtureName === "celebration" && typeof window !== "undefined") {
  window.setInterval(() => {
    base = { ...base, celebration: { id: `multiple-5-${Date.now()}`, who: "both", text: "Tento měsíc už 5× paušálu!" } };
    emit();
  }, 12_000);
}
