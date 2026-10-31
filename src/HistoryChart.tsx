// History tab chart: stacked bars per hour (24 h) or day (Claude bottom,
// Codex top), in money or tokens. Same marks as the month's DailyChart; the
// bar count follows the period, so labels thin out to about seven.
// Geometry is in SVG attributes only (no inline styles, CSP-safe); the
// viewBox follows the figure's width like the month's chart.

import { useState } from "react";
import { CHART_MIN_W, niceCeil } from "./DailyChart";
import { useWidth } from "./hooks";
import { dayLabel, formatMoney, formatMoneyTick, formatTokens, hourLabel, shortDate } from "./format";
import type { History, HistoryPoint } from "./types";

const H = 240;
const M = { top: 12, right: 6, bottom: 22, left: 58 };
const GAP = 2; // surface gap between stacked segments
const LABELS = 7;

export type Metric = "cost" | "tokens";

/** Claude and Codex value of a point in the chosen metric. */
export function pointValues(p: HistoryPoint, metric: Metric): [number, number] {
  return metric === "cost" ? [p.claudeDisplay, p.codexDisplay] : [p.claudeTokens, p.codexTokens];
}

export function bucketLabel(p: HistoryPoint, long = false): string {
  if (p.date) return long ? dayLabel(p.date) : shortDate(p.date);
  return hourLabel(p.start);
}

function topRounded(x: number, y: number, w: number, h: number, r: number): string {
  const rr = Math.min(r, w / 2, h);
  return `M${x} ${y + h} V${y + rr} Q${x} ${y} ${x + rr} ${y} H${x + w - rr} Q${x + w} ${y} ${x + w} ${y + rr} V${y + h} Z`;
}

export function HistoryChart({ history, metric }: { history: History; metric: Metric }) {
  const [hover, setHover] = useState<number | null>(null);
  const [ref, measured] = useWidth<HTMLElement>(640);
  const W = Math.max(CHART_MIN_W, measured);
  const { series, currency } = history;
  const fmt = (v: number) => (metric === "cost" ? formatMoney(v, currency) : formatTokens(v));
  const codexShown = history.providers.some((p) => p.provider === "codex") && !history.codexWithoutHours;
  const codexEstimate = history.providers.some((p) => p.provider === "codex" && p.estimate);
  const claudeShown = history.providers.some((p) => p.provider === "claude");

  const n = Math.max(series.length, 1);
  const max = niceCeil(Math.max(0, ...series.map((p) => pointValues(p, metric)[0] + pointValues(p, metric)[1])));
  const plotW = W - M.left - M.right;
  const plotH = H - M.top - M.bottom;
  const slotW = plotW / n;
  const barW = Math.max(1, Math.min(slotW * 0.64, 22));
  const y = (v: number) => M.top + plotH - (v / max) * plotH;
  const ticks = [0, max / 2, max];
  // About one label per 90 px, at most seven.
  const every = Math.max(1, Math.ceil(n / Math.min(LABELS, Math.max(3, Math.floor(plotW / 90)))));
  const base = M.top + plotH;
  const hovered = hover !== null ? series[hover] : undefined;
  const label = history.bucket === "hour" ? "po hodinách" : "po dnech";

  return (
    <figure className="daily-chart history-chart" ref={ref}>
      <div className="legend" aria-hidden="true">
        {claudeShown && (
          <span>
            <i className="dot dot-claude" /> Claude
          </span>
        )}
        {codexShown && (
          <span>
            <i className="dot dot-codex" /> Codex{codexEstimate && metric === "cost" ? " (odhad)" : ""}
          </span>
        )}
      </div>
      <svg viewBox={`0 0 ${W} ${H}`} className="daily-svg" role="img" aria-label={`${metric === "cost" ? "Cena" : "Tokeny"} ${label}, Claude a Codex na sobě`}>
        <g className="grid">
          {ticks.map((t) => (
            <g key={t}>
              <line x1={M.left} x2={W - M.right} y1={y(t)} y2={y(t)} className={t === 0 ? "baseline" : "gridline"} />
              <text x={M.left - 8} y={y(t) + 4} textAnchor="end" className="axis-label">
                {metric === "cost" ? formatMoneyTick(t, currency) : formatTokens(t)}
              </text>
            </g>
          ))}
        </g>

        {series.map((p, i) => {
          const [claude, codex] = pointValues(p, metric);
          const x = M.left + i * slotW + (slotW - barW) / 2;
          // Labels from the newest bar backwards, so today is always named.
          const showLabel = (n - 1 - i) % every === 0;
          const claudeH = (claude / max) * plotH;
          const codexH = (codex / max) * plotH;
          const hasCodex = codexH > 0.5;
          const claudeTop = base - claudeH;
          const codexTop = claudeTop - (hasCodex && claudeH > 0 ? GAP : 0) - codexH;
          return (
            <g key={p.start} className={hover === i ? "day is-hover" : "day"} onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
              <rect className="day-hit" x={M.left + i * slotW} y={M.top} width={slotW} height={plotH}>
                <title>{`${bucketLabel(p, true)}: ${fmt(claude + codex)}`}</title>
              </rect>
              {claudeH > 0 && (
                <path
                  className="seg seg-claude"
                  d={hasCodex ? `M${x} ${base} V${claudeTop} H${x + barW} V${base} Z` : topRounded(x, claudeTop, barW, claudeH, Math.min(3, barW / 2))}
                />
              )}
              {hasCodex && <path className="seg seg-codex" d={topRounded(x, codexTop, barW, codexH, Math.min(3, barW / 2))} />}
              {showLabel && (
                <text x={x + barW / 2} y={H - 6} textAnchor="middle" className="axis-label">
                  {bucketLabel(p)}
                </text>
              )}
            </g>
          );
        })}

        {hovered && hover !== null && (
          <Tooltip index={hover} slotW={slotW} width={W} point={hovered} metric={metric} fmt={fmt} y={y} claudeShown={claudeShown} codexShown={codexShown} />
        )}
      </svg>
    </figure>
  );
}

function Tooltip({
  index,
  slotW,
  width: W,
  point,
  metric,
  fmt,
  y,
  claudeShown,
  codexShown,
}: {
  index: number;
  slotW: number;
  width: number;
  point: HistoryPoint;
  metric: Metric;
  fmt: (v: number) => string;
  y: (v: number) => number;
  claudeShown: boolean;
  codexShown: boolean;
}) {
  const [claude, codex] = pointValues(point, metric);
  const rows: { cls: string; text: string }[] = [];
  if (claudeShown) rows.push({ cls: "tt-claude", text: `Claude ${fmt(claude)}` });
  if (codexShown) rows.push({ cls: "tt-codex", text: `Codex ${fmt(codex)}` });
  const boxW = 150;
  const boxH = 28 + rows.length * 15;
  const cx = M.left + index * slotW + slotW / 2;
  const bx = Math.min(Math.max(cx - boxW / 2, M.left), W - M.right - boxW);
  const by = Math.max(M.top, y(claude + codex) - boxH - 8);
  return (
    <g className="chart-tooltip" pointerEvents="none">
      <rect x={bx} y={by} width={boxW} height={boxH} rx="8" />
      <text x={bx + 10} y={by + 17} className="tt-title">
        {bucketLabel(point, true)} · {fmt(claude + codex)}
      </text>
      {rows.map((r, i) => (
        <g key={r.cls}>
          <circle cx={bx + 13} cy={by + 31 + i * 15} r="3.5" className={r.cls} />
          <text x={bx + 22} y={by + 35 + i * 15} className="tt-row">
            {r.text}
          </text>
        </g>
      ))}
    </g>
  );
}
