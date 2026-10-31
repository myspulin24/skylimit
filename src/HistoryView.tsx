// History tab: consumption over a chosen period with the month's breakdowns,
// the split by token type and what the cache saved, in API prices or tokens.
// The core prices each period on request (`history`); the previous answer
// stays on screen, dimmed, while the next one loads, and a new one is fetched
// whenever the data changes.

import { useEffect, useState } from "react";
import { getHistory } from "./api";
import { HistoryChart, bucketLabel, pointValues, type Metric } from "./HistoryChart";
import { NBSP, PROVIDER_NAME, dayLabel, formatDate, formatMoney, formatTokens, plural, sourceLabel } from "./format";
import type { History, HistoryFilter, HistoryKind, HistoryRange, HistoryRow, Snapshot } from "./types";

type Breakdown = "model" | "day" | "source" | "project";

const RANGES: { id: HistoryRange; label: string }[] = [
  { id: "24h", label: "24 h" },
  { id: "7d", label: "7 dní" },
  { id: "30d", label: "30 dní" },
  { id: "90d", label: "90 dní" },
  { id: "all", label: "Vše" },
];
const FILTERS: { id: HistoryFilter; label: string }[] = [
  { id: "all", label: "Vše" },
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
];
const METRICS: { id: Metric; label: string }[] = [
  { id: "cost", label: "Cena" },
  { id: "tokens", label: "Tokeny" },
];
const BREAKDOWNS: { id: Breakdown; label: string }[] = [
  { id: "model", label: "Model" },
  { id: "day", label: "Den" },
  { id: "source", label: "Zdroj" },
  { id: "project", label: "Projekt" },
];

const RANGE_CAPTION: Record<HistoryRange, string> = {
  "24h": "za posledních 24 hodin",
  "7d": "za posledních 7 dní",
  "30d": "za posledních 30 dní",
  "90d": "za posledních 90 dní",
  all: "za celou historii",
};

export const KIND_LABEL: Record<HistoryKind, string> = {
  input: "Vstup",
  cacheRead: "Čtení z cache",
  cacheWrite: "Zápis do cache",
  output: "Výstup",
  webSearch: "Vyhledávání na webu",
  codex: "Codex (bez rozpadu)",
};

/** Rows of the breakdown before "Zobrazit všechny". */
export const HISTORY_ROWS = 12;

function Segmented<T extends string>({
  label,
  name,
  options,
  value,
  onChange,
}: {
  label: string;
  name: string;
  options: { id: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <label key={o.id} className={value === o.id ? "is-on" : undefined}>
          <input type="radio" name={name} value={o.id} checked={value === o.id} onChange={() => onChange(o.id)} />
          {o.label}
        </label>
      ))}
    </div>
  );
}

export function HistoryView({ snapshot }: { snapshot: Snapshot | null }) {
  const [range, setRange] = useState<HistoryRange>("30d");
  const [filter, setFilter] = useState<HistoryFilter>("all");
  const [metric, setMetric] = useState<Metric>("cost");
  const [history, setHistory] = useState<History | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // Changes with new data or another currency, not with every periodic snapshot.
  const dataVersion = snapshot ? `${snapshot.generatedAt - snapshot.updatedAgoSec}:${snapshot.currency}` : "";

  useEffect(() => {
    let live = true;
    setLoading(true);
    getHistory(range, filter)
      .then((h) => {
        if (!live) return;
        setHistory(h);
        setError(null);
      })
      .catch((e: unknown) => {
        if (live) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [range, filter, dataVersion]);

  return (
    <>
      <div className="history-filters">
        <Segmented label="Období" name="history-range" options={RANGES} value={range} onChange={setRange} />
        <Segmented label="Poskytovatel" name="history-filter" options={FILTERS} value={filter} onChange={setFilter} />
        <Segmented label="Zobrazit" name="history-metric" options={METRICS} value={metric} onChange={setMetric} />
      </div>
      {error && history && (
        <p className="banner" role="alert">
          Historii se nepodařilo obnovit: {error}
        </p>
      )}
      {!history ? (
        <p className="app-empty">{error ? `Historii se nepodařilo načíst: ${error}` : "Načítám…"}</p>
      ) : (
        <div className={loading ? "history-grid is-loading" : "history-grid"} aria-busy={loading}>
          <SummaryCard history={history} metric={metric} />
          <section className="card card-hchart">
            <h2>
              {metric === "cost" ? "V cenách API" : "Tokeny"} {history.bucket === "hour" ? "po hodinách" : "po dnech"}
            </h2>
            <HistoryChart history={history} metric={metric} />
            <ChartTable history={history} metric={metric} />
          </section>
          <TotalsCard history={history} />
          <KindsCard history={history} metric={metric} />
          <BreakdownCard history={history} metric={metric} />
        </div>
      )}
      {history && <Notes history={history} />}
    </>
  );
}

function amount(value: number, metric: Metric, currency: History["currency"]): string {
  return metric === "cost" ? formatMoney(value, currency) : formatTokens(value);
}

function percent(part: number, whole: number): string {
  if (whole <= 0 || part <= 0) return "–";
  const p = (part / whole) * 100;
  if (p < 0.1) return `<0,1${NBSP}%`;
  return `${p.toLocaleString("cs-CZ", { maximumFractionDigits: 1 })}${NBSP}%`;
}

function SummaryCard({ history, metric }: { history: History; metric: Metric }) {
  const total = metric === "cost" ? history.totalDisplay : history.totalTokens;
  const unit = history.bucket === "hour" ? plural(history.activeBuckets, "aktivní hodina", "aktivní hodiny", "aktivních hodin") : plural(history.activeBuckets, "aktivní den", "aktivní dny", "aktivních dnů");
  return (
    <section className="card card-hsum">
      <h2>Celkem {RANGE_CAPTION[history.range]}</h2>
      <div className="hero">
        <span className="hero-value">{amount(total, metric, history.currency)}</span>
      </div>
      <p className="hero-cap">
        {metric === "cost" ? "v cenách API" : "zpracovaných tokenů"} · {history.activeBuckets}
        {NBSP}
        {unit}
      </p>
      <ul className="history-providers">
        {/* Codex has no hours: in 24 h a note says so instead of a zero row. */}
        {history.providers.filter((p) => !(p.provider === "codex" && history.codexWithoutHours)).map((p) => {
          const value = metric === "cost" ? p.display : p.tokens;
          const other = metric === "cost" ? formatTokens(p.tokens) + " tokenů" : formatMoney(p.display, history.currency);
          return (
            <li key={p.provider}>
              <span className="cell-name">
                <i className={`dot dot-${p.provider}`} aria-hidden="true" />
                {PROVIDER_NAME[p.provider]}
                {p.estimate && metric === "cost" && <span className="tag">odhad</span>}
              </span>
              <span className="num">{amount(value, metric, history.currency)}</span>
              <span className="muted num">{percent(value, total)}</span>
              <span className="muted history-provider-other">{other}</span>
            </li>
          );
        })}
      </ul>
      {history.firstDataAt !== null && <p className="note history-since">Záznamy od {formatDate(history.firstDataAt)}</p>}
    </section>
  );
}

function ChartTable({ history, metric }: { history: History; metric: Metric }) {
  const rows = [...history.series].reverse();
  return (
    <details className="chart-table">
      <summary>Tabulka {history.bucket === "hour" ? "po hodinách" : "po dnech"}</summary>
      <table className="table">
        <thead>
          <tr>
            <th scope="col">{history.bucket === "hour" ? "Hodina" : "Den"}</th>
            <th scope="col" className="r">
              Claude
            </th>
            <th scope="col" className="r">
              Codex
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => {
            const [claude, codex] = pointValues(p, metric);
            return (
              <tr key={p.start}>
                <td>{bucketLabel(p, true)}</td>
                <td className="r num">{amount(claude, metric, history.currency)}</td>
                <td className="r num">{amount(codex, metric, history.currency)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </details>
  );
}

function TotalsCard({ history }: { history: History }) {
  const kind = (k: HistoryKind) => history.kinds.find((r) => r.kind === k);
  const claudeShown = history.providers.some((p) => p.provider === "claude");
  const tokens = (k: HistoryKind) => (claudeShown ? formatTokens(kind(k)?.tokens ?? 0) : "–");
  const tiles: { label: string; value: string; hint?: string }[] = [
    { label: "Zpracované tokeny", value: formatTokens(history.totalTokens) },
    { label: "Z cache", value: tokens("cacheRead"), hint: "vstup přečtený z cache" },
    { label: "Bez cache", value: tokens("input"), hint: "vstup za plnou cenu" },
    { label: "Zápis do cache", value: tokens("cacheWrite") },
    { label: "Výstup", value: tokens("output") },
    {
      label: "Úspora díky cache",
      value: claudeShown ? formatMoney(history.cacheSavingsDisplay, history.currency) : "–",
      hint: "proti plné ceně vstupu",
    },
  ];
  return (
    <section className="card card-htotals" aria-label="Součty">
      <dl className="tiles">
        {tiles.map((t) => (
          <div key={t.label} className="tile" title={t.hint}>
            <dt>{t.label}</dt>
            <dd>{t.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function KindsCard({ history, metric }: { history: History; metric: Metric }) {
  const rows = history.kinds.filter((k) => (metric === "cost" ? k.display : k.tokens) > 0);
  const whole = rows.reduce((sum, k) => sum + (metric === "cost" ? k.display : k.tokens), 0);
  return (
    <section className="card card-hkinds">
      <h2>{metric === "cost" ? "Cena podle typu" : "Tokeny podle typu"}</h2>
      {rows.length === 0 ? (
        <p className="muted">V tomto období nic.</p>
      ) : (
        <>
          <div className="kind-bar" aria-hidden="true">
            {rows.map((k) => {
              const value = metric === "cost" ? k.display : k.tokens;
              return <span key={k.kind} className={`kind-seg kind-${k.kind}`} style={{ flexGrow: value / whole }} />;
            })}
          </div>
          <ul className="kind-legend">
            {rows.map((k) => {
              const value = metric === "cost" ? k.display : k.tokens;
              return (
                <li key={k.kind}>
                  <i className={`kind-dot kind-${k.kind}`} aria-hidden="true" />
                  <span className="muted">{KIND_LABEL[k.kind]}</span>
                  <b className="num">{amount(value, metric, history.currency)}</b>
                  <span className="muted num">{percent(value, whole)}</span>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </section>
  );
}

function rowLabel(row: HistoryRow, breakdown: Breakdown, withYear: boolean): string {
  if (breakdown === "day") return dayLabel(row.key, withYear);
  if (row.provider === "codex" && row.key === "codex") {
    if (breakdown === "model") return "Codex (bez rozpadu na modely)";
    if (breakdown === "project") return "Codex (bez rozpadu na projekty)";
    return "Codex";
  }
  if (breakdown === "source") return sourceLabel(row.key);
  return row.key;
}

function BreakdownCard({ history, metric }: { history: History; metric: Metric }) {
  const [breakdown, setBreakdown] = useState<Breakdown>("model");
  const [expanded, setExpanded] = useState(false);
  const source: Record<Breakdown, HistoryRow[]> = {
    model: history.byModel,
    day: history.byDay,
    source: history.bySource,
    project: history.byProject,
  };
  const value = (r: HistoryRow) => (metric === "cost" ? r.display : r.tokens);
  const all = breakdown === "day" ? source.day : [...source[breakdown]].sort((a, b) => value(b) - value(a) || b.tokens - a.tokens);
  const whole = all.reduce((sum, r) => sum + value(r), 0);
  const top = all.reduce((m, r) => Math.max(m, value(r)), 0);
  const shown = expanded ? all : all.slice(0, HISTORY_ROWS);
  const withYear = history.range === "all" || history.range === "90d";

  return (
    <section className="card card-hbreak">
      <div className="card-head">
        <h2>Rozpad</h2>
        <Segmented
          label="Rozpad podle"
          name="history-breakdown"
          options={BREAKDOWNS}
          value={breakdown}
          onChange={(b) => {
            setBreakdown(b);
            setExpanded(false);
          }}
        />
      </div>
      {all.length === 0 ? (
        <p className="muted">V tomto období nic.</p>
      ) : (
        <>
          <table className="table history-table">
            <colgroup>
              <col className="col-rank" />
              <col />
              <col className="col-amount" />
              <col className="col-share" />
              <col className="col-tokens" />
            </colgroup>
            <thead>
              <tr>
                <th scope="col">#</th>
                <th scope="col">{BREAKDOWNS.find((b) => b.id === breakdown)?.label}</th>
                <th scope="col" className="r">
                  Cena
                </th>
                <th scope="col" className="r">
                  Podíl
                </th>
                <th scope="col" className="r">
                  Tokeny
                </th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r, i) => {
                const label = rowLabel(r, breakdown, withYear);
                const raw = breakdown === "source" && label !== r.key ? ` (${r.key})` : "";
                return (
                  <tr key={r.key}>
                    <td className="muted num">{i + 1}</td>
                    <th scope="row">
                      <span className="row-name" title={label + raw}>
                        {r.provider && <i className={`dot dot-${r.provider}`} aria-hidden="true" />}
                        <span className="row-text">{label}</span>
                      </span>
                      <span className="share" aria-hidden="true">
                        <span className={`share-fill share-${r.provider ?? "day"}`} style={{ width: `${top > 0 ? (value(r) / top) * 100 : 0}%` }} />
                      </span>
                    </th>
                    <td className="r num">{formatMoney(r.display, history.currency)}</td>
                    <td className="r num muted">{percent(value(r), whole)}</td>
                    <td className="r num muted">{formatTokens(r.tokens)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {all.length > HISTORY_ROWS && (
            <button type="button" className="more-btn" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>
              {expanded ? "Zobrazit méně" : `Zobrazit všechny (${all.length}) · ještě ${all.length - shown.length}`}
            </button>
          )}
        </>
      )}
    </section>
  );
}

function Notes({ history }: { history: History }) {
  const notes: string[] = [];
  if (history.totalTokens === 0) notes.push("V tomto období není žádná spotřeba.");
  if (history.bySource.some((r) => r.key === "t3-cache")) {
    notes.push(
      "Starší historie Claude je obnovená z mezipaměti T3 Code, protože Claude Code své záznamy po 30 dnech maže. Výstupní tokeny v ní můžou být nižší: T3 si u zprávy pamatuje první, ještě neúplný údaj.",
    );
  }
  if (history.codexWithoutHours) notes.push("Codex hlásí jen denní součty, v pohledu 24 h proto chybí.");
  if (history.providers.some((p) => p.provider === "codex" && p.estimate && p.tokens > 0)) {
    notes.push("Cena Codexu je odhad z denních součtů tokenů, bez rozpadu na modely a typy.");
  }
  for (const p of history.providers) {
    if (p.unpricedTokens > 0) {
      notes.push(`${PROVIDER_NAME[p.provider]}: ${formatTokens(p.unpricedTokens)} tokenů bez ceny (nenaceněno, nikdy jako 0).`);
    }
  }
  return (
    <>
      {notes.map((n) => (
        <p key={n} className="note history-note">
          {n}
        </p>
      ))}
      <p className="footnote">Ceny podle ceníku API (snímek LiteLLM) a kurzu ČNB. Spočítáno z lokálních záznamů Claude Code a z Codexu.</p>
    </>
  );
}
