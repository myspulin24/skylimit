// The always-on-top widget: mascots first, limits second, score third
// (order of the approved wireframe). Renders only what the core sends.

import { useEffect, useState, type CSSProperties } from "react";
import { openApp, openFeedback, refreshUsage } from "./api";
import { Character } from "./Character";
import { FeedbackIcon } from "./icons";
import {
  CREATURE_NAME,
  MOOD_CAPTION,
  MOOD_HINT,
  NBSP,
  PROVIDER_GENITIVE,
  PROVIDER_NAME,
  barTone,
  formatAgo,
  formatCountdown,
  formatMoney,
  formatMultiple,
  formatPercent,
  isUncertainScore,
} from "./format";
import { useNow, useSnapshot } from "./hooks";
import { hasOffer, useUpdate } from "./update";
import type { LimitWindow, Mood, Provider, ProviderLimits, ProviderScore, Snapshot, WidgetPeriod } from "./types";
import "./styles/widget.css";

export function Widget() {
  const { snapshot, receivedAt, error } = useSnapshot();
  const now = useNow(15_000);
  const transparency = Math.min(90, Math.max(0, snapshot?.widgetTransparency ?? 0));
  // Only the backdrop fades; text, bars and mascots stay fully opaque.
  const style = { "--panel-alpha": String((100 - transparency) / 100) } as CSSProperties;

  return (
    <div className={transparency >= 40 ? "widget-shell see-through" : "widget-shell"} style={style}>
      <main className="panel" aria-label="Skylimit widget">
        {snapshot ? (
          <WidgetBody snapshot={snapshot} receivedAt={receivedAt} now={now} />
        ) : (
          <WidgetPlaceholder error={error} />
        )}
      </main>
    </div>
  );
}

function WidgetPlaceholder({ error }: { error: string | null }) {
  return (
    <>
      <Stage moods={{ claude: "calm", codex: "calm" }} />
      <p className="placeholder">{error ? "Jádro aplikace neodpovídá." : "Načítám…"}</p>
    </>
  );
}

interface BodyProps {
  snapshot: Snapshot;
  receivedAt: number;
  now: number;
}

function moodOf(snapshot: Snapshot, provider: Provider): Mood {
  return snapshot.moods.find((m) => m.provider === provider)?.mood ?? "calm";
}

function WidgetBody({ snapshot, receivedAt, now }: BodyProps) {
  const nowSec = Math.floor(now / 1000);
  const updatedAgo = snapshot.updatedAgoSec + Math.max(0, (now - receivedAt) / 1000);
  return (
    <>
      <Stage moods={{ claude: moodOf(snapshot, "claude"), codex: moodOf(snapshot, "codex") }} />

      <section className="limits" aria-label="Limity">
        {snapshot.limits.map((limits) => (
          <ProviderLimitRows key={limits.provider} limits={limits} nowSec={nowSec} />
        ))}
      </section>

      <ScoreBlock snapshot={snapshot} />

      {!snapshot.backfill.done && <BackfillLine backfill={snapshot.backfill} />}

      <footer className="widget-foot">
        <div className="foot-info">
          <span className="updated">aktualizováno {formatAgo(updatedAgo)}</span>
          <span className="version" title="Verze Skylimitu">
            v{__APP_VERSION__}
          </span>
        </div>
        <div className="foot-actions">
          <button
            type="button"
            className="icon-btn"
            onClick={() => void openFeedback()}
            aria-label="Poslat zpětnou vazbu"
            title="Poslat zpětnou vazbu"
          >
            <FeedbackIcon />
          </button>
          <RefreshButton />
          <GearButton />
        </div>
      </footer>

      <Celebration celebration={snapshot.celebration} />
    </>
  );
}

/** Mascot stage; doubles as the window drag handle (children ignore the mouse). */
function Stage({ moods }: { moods: Record<Provider, Mood> }) {
  // Native tooltip: what each creature is doing and why, plus where the full list is.
  const hint = [
    ...(["claude", "codex"] as const).map((p) => `${CREATURE_NAME[p]} ${MOOD_CAPTION[p][moods[p]]}: ${MOOD_HINT[moods[p]]}`),
    "Všechny stavy: aplikace › Stavy",
  ].join("\n");
  return (
    <header className="stage" data-tauri-drag-region title={hint}>
      {(["claude", "codex"] as const).map((p) => (
        <div key={p} className={`stage-slot stage-${p}`}>
          <Character provider={p} mood={moods[p]} />
          <div className="stage-caption">
            <span className="who">{CREATURE_NAME[p]}</span>
            <span className="what">{MOOD_CAPTION[p][moods[p]]}</span>
          </div>
        </div>
      ))}
    </header>
  );
}

function ProviderLimitRows({ limits, nowSec }: { limits: ProviderLimits; nowSec: number }) {
  const { provider } = limits;
  if (limits.state === "off") {
    return (
      <div className="limit-off">
        <i className={`dot dot-${provider}`} aria-hidden="true" />
        <span>{limits.note ?? `limity ${PROVIDER_GENITIVE[provider]} vypnuté`}</span>
      </div>
    );
  }

  // Claude's weekly Fable limit is a row like the others, right after the week.
  const windows = limits.windows.filter(
    (w) => w.kind === "5h" || w.kind === "week" || (provider === "claude" && w.kind === "weekModel"),
  );
  const showNote = limits.note && limits.state !== "known";
  return (
    <div className={`limit-group limits-${limits.state}`}>
      {windows.length > 0 ? (
        windows.map((w) => <LimitRow key={`${w.kind}-${w.label}`} provider={provider} win={w} nowSec={nowSec} />)
      ) : (
        <LimitRow
          provider={provider}
          win={{ kind: "other", label: "", percent: null, resetsAt: null, fetchedAt: null, state: "unknown" }}
          nowSec={nowSec}
        />
      )}
      {showNote && <p className="limit-note">{limits.note}</p>}
    </div>
  );
}

function LimitRow({ provider, win, nowSec }: { provider: Provider; win: LimitWindow; nowSec: number }) {
  const hasValue = win.percent !== null && (win.state === "known" || win.state === "stale");
  const percent = hasValue ? Math.min(100, Math.max(0, win.percent ?? 0)) : 0;

  let value = "?";
  let sub = "";
  switch (win.state) {
    case "known":
      value = hasValue ? formatPercent(percent) : "?";
      // A Claude 5h window that reset with no work since opens with the next message.
      sub = win.resetsAt !== null ? formatCountdown(win.resetsAt, nowSec) : percent === 0 ? "začne první zprávou" : "";
      break;
    case "stale":
      value = hasValue ? `~${formatPercent(percent)}` : "?";
      sub = win.fetchedAt !== null ? `poslední známá · ${formatAgo(nowSec - win.fetchedAt)}` : "poslední známá";
      break;
    case "afterReset":
      sub = "po resetu, aktuální stav neznámý";
      break;
    case "unknown":
      sub = "stav neznámý";
      break;
  }

  const barClass = [
    "bar",
    hasValue ? `tone-${barTone(percent)}` : "tone-none",
    win.state === "stale" ? "bar--hatched" : "",
    win.state === "afterReset" || win.state === "unknown" ? "bar--unknown" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={`limit-row state-${win.state}`}>
      <div className="limit-head">
        <span className="limit-name">
          <i className={`dot dot-${provider}`} aria-hidden="true" />
          {win.label ? `${PROVIDER_NAME[provider]} · ${win.label}` : PROVIDER_NAME[provider]}
        </span>
        <span className="limit-sub" title={sub || undefined}>
          {sub}
        </span>
        <span className="limit-value">{value}</span>
      </div>
      <div className={barClass} aria-hidden="true">
        {hasValue && <span className="bar-fill" style={{ width: `${percent}%` }} />}
      </div>
    </div>
  );
}

const PERIOD_CAPTION: Record<WidgetPeriod, string> = {
  month: "tento měsíc",
  "24h": "za 24 hodin",
  "7d": "za 7 dní",
  "30d": "za 30 dní",
  "90d": "za 90 dní",
  all: "za celou historii",
};

/** "tento měsíc v cenách API", "za 7 dní vč. úspory z cache" */
export function scoreCaption(widget: Snapshot["widget"]): string {
  return `${PERIOD_CAPTION[widget.period]} ${widget.amount === "api" ? "v cenách API" : "vč. úspory z cache"}`;
}

function ScoreBlock({ snapshot }: { snapshot: Snapshot }) {
  const { currency, widget } = snapshot;
  const { total, scores } = widget;
  const visible = scores.filter((s) => s.state !== "off");
  const uncertain = visible.some((s) => isUncertainScore(s.state));
  const noPlan = scores.every((s) => s.planMonthlyDisplay === null);
  const claude = scores.find((s) => s.provider === "claude");

  return (
    <section className="score" aria-label="Skóre">
      <div className="score-top">
        <div className="score-main">
          <div className="score-total">
            <span className="score-value">{formatMoney(total.display, currency)}</span>
            {uncertain && (
              <span className="score-q" title="Část skóre je nejistá">
                ?
              </span>
            )}
          </div>
          <div className="score-caption">{scoreCaption(widget)}</div>
        </div>
        <div className="score-side">
          {total.multiple !== null ? (
            <div className="score-multiple">
              <b>{formatMultiple(total.multiple)}</b> paušál
            </div>
          ) : noPlan ? (
            <button type="button" className="plan-cta" onClick={() => void openApp()}>
              Zadej, kolik platíš
            </button>
          ) : null}
          <Sparkline values={widget.spark} />
        </div>
      </div>

      <ul className="score-providers">
        {visible.map((s) => (
          <ProviderScoreChip key={s.provider} score={s} currency={currency} />
        ))}
      </ul>

      {claude?.state === "formatChanged" && <p className="score-warn">formát záznamů Claude se změnil</p>}
      {total.excludedNote && <p className="score-note">{total.excludedNote}</p>}
    </section>
  );
}

function ProviderScoreChip({ score, currency }: { score: ProviderScore; currency: Snapshot["currency"] }) {
  const amount = formatMoney(score.display, currency);
  let value: string;
  let tag: string | null = null;
  switch (score.state) {
    case "estimate":
      value = `≈${NBSP}${amount}`;
      tag = "odhad";
      break;
    case "partial":
      value = amount;
      tag = "částečně";
      break;
    case "unknown":
    case "formatChanged":
      value = "?";
      break;
    default:
      value = amount;
  }
  return (
    <li className={`chip chip-${score.provider} chip-${score.state}`}>
      <i className={`dot dot-${score.provider}`} aria-hidden="true" />
      <span className="chip-name">{PROVIDER_NAME[score.provider]}</span>
      <span className="chip-value">{value}</span>
      {tag && <span className="tag">{tag}</span>}
    </li>
  );
}

/** Tiny SVG trend of the score per day (or hour); geometry lives in attributes. */
export function Sparkline({ values }: { values: number[] }) {
  if (values.length < 2) return null;
  const w = 100;
  const h = 28;
  const max = Math.max(...values, 0.0001);
  const step = w / (values.length - 1);
  const pts = values.map((v, i) => [i * step, h - 2 - (v / max) * (h - 5)] as const);
  const line = pts.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`).join(" ");
  const area = `${line} L${w} ${h} L0 ${h} Z`;
  const last = pts[pts.length - 1]!;
  return (
    <svg className="sparkline" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img" aria-label="Průběh po dnech tento měsíc">
      <path className="spark-area" d={area} />
      <path className="spark-line" d={line} vectorEffect="non-scaling-stroke" />
      <circle className="spark-dot" cx={last[0]} cy={last[1]} r="1.6" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

function BackfillLine({ backfill }: { backfill: Snapshot["backfill"] }) {
  const pct = backfill.filesTotal > 0 ? (backfill.filesDone / backfill.filesTotal) * 100 : 0;
  return (
    <div className="backfill" role="status">
      <span>
        načítám historii {backfill.filesDone}/{backfill.filesTotal}
      </span>
      <span className="backfill-track" aria-hidden="true">
        <span className="backfill-fill" style={{ width: `${pct}%` }} />
      </span>
    </div>
  );
}

const PARTICLES = 16;

/** Short coin/confetti burst whenever a new celebration id arrives. */
function Celebration({ celebration }: { celebration: Snapshot["celebration"] }) {
  const [active, setActive] = useState<NonNullable<Snapshot["celebration"]> | null>(null);
  const [seen, setSeen] = useState<string | null>(null);

  if (celebration && celebration.id !== seen) {
    // Derive state during render (React's recommended pattern for "on prop change").
    setSeen(celebration.id);
    setActive(celebration);
  }

  useEffect(() => {
    if (!active) return;
    const t = window.setTimeout(() => setActive(null), 4200);
    return () => window.clearTimeout(t);
  }, [active]);

  if (!active) return null;
  return (
    <div className={`celebration who-${active.who}`} key={active.id} role="status" aria-live="polite">
      <div className="burst" aria-hidden="true">
        {Array.from({ length: PARTICLES }, (_, i) => (
          <i key={i} className={`particle p${i + 1} ${i % 3 === 0 ? "coin" : `confetti c${i % 4}`}`} />
        ))}
      </div>
      <div className="celebration-text">{active.text}</div>
    </div>
  );
}

/** How long the icon spins: the request returns at once, fresh numbers follow
 *  within a few seconds as snapshots (the core keeps a 10 s gap per source). */
export const REFRESH_SPIN_MS = 1500;

/** "Obnovit": asks the core to read Codex, the Claude logs, Claude Code and T3 Code now. */
function RefreshButton() {
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!busy) return;
    const t = window.setTimeout(() => setBusy(false), REFRESH_SPIN_MS);
    return () => window.clearTimeout(t);
  }, [busy]);

  const onClick = () => {
    if (busy) return;
    setBusy(true);
    refreshUsage().catch((err: unknown) => console.error("refresh failed", err));
  };

  return (
    <button
      type="button"
      className={busy ? "icon-btn refresh is-busy" : "icon-btn refresh"}
      onClick={onClick}
      aria-label="Obnovit data"
      title="Obnovit data teď"
      aria-busy={busy}
    >
      <RefreshIcon />
    </button>
  );
}

function RefreshIcon() {
  return (
    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true" focusable="false">
      <g fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round">
        <path d="M15.6 7.4A6.2 6.2 0 1 0 16.2 11" />
        <path d="M16.2 3.6v4h-4" />
      </g>
    </svg>
  );
}

/** Opens the app; a dot when a new version is waiting there. */
function GearButton() {
  const update = useUpdate();
  const offer = hasOffer(update);
  const label = offer ? `Otevřít aplikaci: je k dispozici Skylimit ${update?.version}` : "Otevřít aplikaci";
  return (
    <button type="button" className={offer ? "icon-btn has-update" : "icon-btn"} onClick={() => void openApp()} aria-label={label} title={label}>
      <GearIcon />
      {offer && <span className="update-dot" aria-hidden="true" />}
    </button>
  );
}

function GearIcon() {
  return (
    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true" focusable="false">
      {/* ten teeth from a dashed thick ring */}
      <g fill="none" stroke="currentColor">
        <circle cx="10" cy="10" r="7.2" strokeWidth="2.6" strokeDasharray="2.4 2.124" />
        <circle cx="10" cy="10" r="5" strokeWidth="2.2" />
      </g>
    </svg>
  );
}
