// Full app window: the month in detail, a guide to every state, the settings.
// Same snapshot as the widget, more room: where the work came from, which
// model, which project.

import { useCallback, useEffect, useState } from "react";
import { onOpenFeedback } from "./api";
import { Character } from "./Character";
import { DailyChart } from "./DailyChart";
import { FeedbackDialog } from "./FeedbackDialog";
import { FeedbackIcon } from "./icons";
import {
  NBSP,
  PROVIDER_NAME,
  formatDays,
  formatMoney,
  formatMultiple,
  formatTokens,
  isUncertainScore,
  monthLabel,
  monthName,
  shortDate,
  sourceLabel,
} from "./format";
import { useSnapshot } from "./hooks";
import { SettingsForm } from "./SettingsForm";
import { StatesGuide } from "./StatesGuide";
import type { BreakdownRow, Mood, Provider, ProviderScore, Snapshot } from "./types";
import "./styles/app.css";

type Tab = "month" | "states" | "settings";

const TABS: { id: Tab; label: string }[] = [
  { id: "month", label: "Měsíc" },
  { id: "states", label: "Stavy" },
  { id: "settings", label: "Nastavení" },
];

/** Rows a breakdown card shows before "Zobrazit všechny". */
export const BREAKDOWN_ROWS = 8;

export function AppWindow() {
  const { snapshot, error } = useSnapshot();
  // `?tab=settings` / `?tab=states` lets the shell (or a dev link) open a tab directly.
  const [tab, setTab] = useState<Tab>(() => {
    const requested = new URLSearchParams(location.search).get("tab");
    return TABS.find((t) => t.id === requested)?.id ?? "month";
  });
  // `?feedback=1`: opened from the tray's "Poslat zpětnou vazbu…".
  const [feedbackOpen, setFeedbackOpen] = useState(() => new URLSearchParams(location.search).get("feedback") === "1");
  const closeFeedback = useCallback(() => setFeedbackOpen(false), []);
  useEffect(() => onOpenFeedback(() => setFeedbackOpen(true)), []);

  const mood = (p: Provider): Mood => snapshot?.moods.find((m) => m.provider === p)?.mood ?? "calm";
  const tabLabel = TABS.find((t) => t.id === tab)?.label ?? "";

  return (
    <div className="app">
      <header className="app-head">
        <div className="brand">
          <div className="brand-mascots" aria-hidden="true">
            <Character provider="claude" mood={mood("claude")} />
            <Character provider="codex" mood={mood("codex")} />
          </div>
          <div>
            <h1>Skylimit</h1>
            <p className="brand-sub">{snapshot ? monthLabel(snapshot.month) : "…"}</p>
          </div>
        </div>
        <div className="head-actions">
          <button type="button" className="head-pill" onClick={() => setFeedbackOpen(true)}>
            <FeedbackIcon />
            Zpětná vazba
          </button>
          <nav className="tabs" role="tablist" aria-label="Sekce">
            {TABS.map((t) => (
              <button
                key={t.id}
                type="button"
                role="tab"
                id={`tab-${t.id}`}
                aria-selected={tab === t.id}
                aria-controls={`panel-${t.id}`}
                className={tab === t.id ? "tab is-active" : "tab"}
                onClick={() => setTab(t.id)}
              >
                {t.label}
              </button>
            ))}
          </nav>
        </div>
      </header>
      {feedbackOpen && <FeedbackDialog location={`Okno aplikace › ${tabLabel}`} onClose={closeFeedback} />}

      <main className="app-main" role="tabpanel" id={`panel-${tab}`} aria-labelledby={`tab-${tab}`}>
        {tab === "month" ? (
          snapshot ? (
            <MonthView snapshot={snapshot} onOpenSettings={() => setTab("settings")} />
          ) : (
            <p className="app-empty">{error ? `Jádro aplikace neodpovídá: ${error}` : "Načítám…"}</p>
          )
        ) : tab === "states" ? (
          <StatesGuide snapshot={snapshot} />
        ) : (
          <SettingsForm snapshot={snapshot} onOpenFeedback={() => setFeedbackOpen(true)} />
        )}
      </main>
      <footer className="app-foot">
        Skylimit {__APP_VERSION__} &middot; &copy; 2026 Michal Jašek &middot; licence MIT
      </footer>
    </div>
  );
}

function MonthView({ snapshot, onOpenSettings }: { snapshot: Snapshot; onOpenSettings: () => void }) {
  const { currency } = snapshot;
  return (
    <>
      {!snapshot.backfill.done && (
        <p className="banner" role="status">
          Načítám historii {snapshot.backfill.filesDone}/{snapshot.backfill.filesTotal} souborů. Čísla ještě porostou.
        </p>
      )}
      <div className="month-grid">
        <ScoreCard snapshot={snapshot} onOpenSettings={onOpenSettings} />

        <section className="card card-daily">
          <h2>Denně v cenách API</h2>
          <DailyChart snapshot={snapshot} />
          <details className="chart-table">
            <summary>Tabulka po dnech</summary>
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">Den</th>
                  <th scope="col" className="r">
                    Claude
                  </th>
                  <th scope="col" className="r">
                    Codex
                  </th>
                </tr>
              </thead>
              <tbody>
                {snapshot.daily.map((d) => (
                  <tr key={d.date}>
                    <td>{shortDate(d.date)}</td>
                    <td className="r num">{formatMoney(d.claudeDisplay, currency)}</td>
                    <td className="r num">{formatMoney(d.codexDisplay, currency)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </details>
        </section>

        <StreaksCard snapshot={snapshot} />

        <div className="breakdowns">
          <BreakdownCard title="Odkud práce přišla" rows={snapshot.bySource} currency={currency} label={sourceLabel} showKey />
          <BreakdownCard title="Modely" rows={snapshot.byModel} currency={currency} />
          <BreakdownCard title="Projekty" rows={snapshot.byProject} currency={currency} />
        </div>
      </div>
      <p className="footnote">
        Ceny podle ceníku API (snímek LiteLLM) a kurzu ČNB. Skóre počítá vše z lokálních záznamů Claude Code a z Codexu.
      </p>
    </>
  );
}

function scoreValue(score: ProviderScore, currency: Snapshot["currency"]): string {
  if (isUncertainScore(score.state)) return "?";
  const amount = formatMoney(score.display, currency);
  return score.state === "estimate" ? `≈${NBSP}${amount}` : amount;
}

function ScoreCard({ snapshot, onOpenSettings }: { snapshot: Snapshot; onOpenSettings: () => void }) {
  const { total, currency, scores } = snapshot;
  const visible = scores.filter((s) => s.state !== "off");
  const uncertain = visible.some((s) => isUncertainScore(s.state));
  const noPlan = scores.every((s) => s.planMonthlyDisplay === null);
  const unpriced = visible.filter((s) => s.unpricedTokens > 0);

  return (
    <section className="card card-score">
      <h2>Skóre: {monthName(snapshot.month)}</h2>
      <div className="hero">
        <span className="hero-value">{formatMoney(total.display, currency)}</span>
        {uncertain && <span className="score-q">?</span>}
      </div>
      <p className="hero-cap">tento měsíc v cenách API</p>
      {total.multiple !== null ? (
        <p className="hero-multiple">
          vytěženo <b>{formatMultiple(total.multiple)}</b> paušálu
          {total.planMonthlyDisplay !== null && (
            <span className="muted"> ({formatMoney(total.planMonthlyDisplay, currency)} měsíčně)</span>
          )}
        </p>
      ) : noPlan ? (
        <p className="hero-multiple">
          <button type="button" className="btn small" onClick={onOpenSettings}>
            Zadej, kolik platíš
          </button>
        </p>
      ) : null}

      <table className="table provider-table">
        <thead>
          <tr>
            <th scope="col">Poskytovatel</th>
            <th scope="col" className="r">
              v cenách API
            </th>
            <th scope="col" className="r">
              paušál
            </th>
            <th scope="col" className="r">
              násobek
            </th>
          </tr>
        </thead>
        <tbody>
          {visible.map((s) => (
            <tr key={s.provider}>
              <th scope="row">
                <span className="cell-name">
                  <i className={`dot dot-${s.provider}`} aria-hidden="true" />
                  {PROVIDER_NAME[s.provider]}
                  {s.state === "estimate" && <span className="tag">odhad</span>}
                  {s.state === "partial" && <span className="tag">částečně</span>}
                </span>
              </th>
              <td className="r num">{scoreValue(s, currency)}</td>
              <td className="r num">{s.planMonthlyDisplay !== null ? formatMoney(s.planMonthlyDisplay, currency) : "nezadáno"}</td>
              <td className="r num">{s.multiple !== null ? formatMultiple(s.multiple) : "–"}</td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <th scope="row">Celkem</th>
            <td className="r num">{formatMoney(total.display, currency)}</td>
            <td className="r num">{total.planMonthlyDisplay !== null ? formatMoney(total.planMonthlyDisplay, currency) : "–"}</td>
            <td className="r num">{total.multiple !== null ? formatMultiple(total.multiple) : "–"}</td>
          </tr>
        </tfoot>
      </table>

      {scores.some((s) => s.provider === "claude" && s.state === "formatChanged") && (
        <p className="score-warn">formát záznamů Claude se změnil</p>
      )}
      {total.excludedNote && <p className="note">{total.excludedNote}</p>}
      {unpriced.map((s) => (
        <p key={s.provider} className="note">
          {PROVIDER_NAME[s.provider]}: {formatTokens(s.unpricedTokens)} tokenů bez ceny (nenaceněno, nikdy jako 0)
        </p>
      ))}
    </section>
  );
}

function StreaksCard({ snapshot }: { snapshot: Snapshot }) {
  return (
    <section className="card card-streaks">
      <h2>Série dnů</h2>
      <ul className="streaks">
        {snapshot.streaks.map((s) => (
          <li key={s.provider}>
            <span className="cell-name">
              <i className={`dot dot-${s.provider}`} aria-hidden="true" />
              {PROVIDER_NAME[s.provider]}
            </span>
            <span className="streak-now num">{formatDays(s.current)}</span>
            <span className="muted num">nejdelší {formatDays(s.longest)}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function BreakdownCard({
  title,
  rows,
  currency,
  label = (k) => k,
  showKey = false,
}: {
  title: string;
  rows: BreakdownRow[];
  currency: Snapshot["currency"];
  label?: (key: string) => string;
  showKey?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const sorted = [...rows].sort((a, b) => b.display - a.display);
  const sum = sorted.reduce((acc, r) => acc + r.display, 0);
  const shown = expanded ? sorted : sorted.slice(0, BREAKDOWN_ROWS);
  const hidden = sorted.length - shown.length;
  return (
    <section className="card card-breakdown">
      <h2>{title}</h2>
      {sorted.length === 0 ? (
        <p className="muted">Zatím nic.</p>
      ) : (
        <>
          {/* Fixed layout: long names get an ellipsis instead of pushing the amounts out of the card. */}
          <table className="table breakdown">
            <colgroup>
              <col />
              <col className="col-tokens" />
              <col className="col-amount" />
            </colgroup>
            <thead>
              <tr>
                <th scope="col">Název</th>
                <th scope="col" className="r">
                  tokeny
                </th>
                <th scope="col" className="r">
                  částka
                </th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => {
                const share = sum > 0 ? (r.display / sum) * 100 : 0;
                const name = label(r.key);
                const withKey = showKey && name.toLowerCase() !== r.key.toLowerCase();
                return (
                  <tr key={r.key}>
                    <th scope="row">
                      <span className="row-name" title={withKey ? `${name} (${r.key})` : name}>
                        {name}
                        {withKey && <span className="key"> ({r.key})</span>}
                      </span>
                      <span className="share" aria-hidden="true">
                        <span className="share-fill" style={{ width: `${share}%` }} />
                      </span>
                    </th>
                    <td className="r num muted">{formatTokens(r.tokens)}</td>
                    <td className="r num">{formatMoney(r.display, currency)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {sorted.length > BREAKDOWN_ROWS && (
            <button type="button" className="more-btn" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>
              {expanded ? "Zobrazit méně" : `Zobrazit všechny (${sorted.length}) · ještě ${hidden}`}
            </button>
          )}
        </>
      )}
    </section>
  );
}
