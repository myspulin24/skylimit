// Stacked daily bars (Claude bottom, Codex top) for the whole month. Days not
// reached yet keep an empty slot so the month's progress is visible.
// Geometry is in SVG attributes only (no inline styles, CSP-safe); the
// viewBox follows the figure's width, so text keeps its size on any window.

import { useState } from "react";
import { useWidth } from "./hooks";
import { formatMoney, formatMoneyTick, shortDate } from "./format";
import type { Snapshot } from "./types";

const H = 250;
/** Width before the first measurement, and the narrowest drawing. */
export const CHART_MIN_W = 320;
const CHART_FALLBACK_W = 640;
/** Bars stay slim on wide windows. */
const BAR_MAX = 26;
const M = { top: 12, right: 6, bottom: 22, left: 50 };
const GAP = 2; // surface gap between stacked segments

function daysInMonth(month: string): number {
  const [y, m] = month.split("-").map(Number);
  if (!y || !m) return 31;
  return new Date(Date.UTC(y, m, 0)).getUTCDate();
}

/** 1, 2, 4, 5 × 10^k ceiling, so the gridlines (top and half) land on round
 * amounts: 25 would put the half line at 12.5. */
export function niceCeil(value: number): number {
  if (value <= 0) return 1;
  const exp = Math.floor(Math.log10(value));
  const base = 10 ** exp;
  for (const step of [1, 2, 4, 5, 10]) {
    if (value <= step * base) return step * base;
  }
  return 10 * base;
}

/** Bar with rounded top corners anchored to the baseline. */
function topRounded(x: number, y: number, w: number, h: number, r: number): string {
  const rr = Math.min(r, w / 2, h);
  return `M${x} ${y + h} V${y + rr} Q${x} ${y} ${x + rr} ${y} H${x + w - rr} Q${x + w} ${y} ${x + w} ${y + rr} V${y + h} Z`;
}

export function DailyChart({ snapshot }: { snapshot: Snapshot }) {
  const { daily, currency, month } = snapshot;
  const [hover, setHover] = useState<number | null>(null);
  const [ref, measured] = useWidth<HTMLElement>(CHART_FALLBACK_W);
  const W = Math.max(CHART_MIN_W, measured);
  const codexEstimate = snapshot.scores.some((s) => s.provider === "codex" && s.state === "estimate");

  const slots = Math.max(daysInMonth(month), daily.length);
  const max = niceCeil(Math.max(0, ...daily.map((d) => d.claudeDisplay + d.codexDisplay)));
  const plotW = W - M.left - M.right;
  const plotH = H - M.top - M.bottom;
  const slotW = plotW / slots;
  const barW = Math.min(BAR_MAX, Math.max(3, slotW * 0.64));
  const y = (v: number) => M.top + plotH - (v / max) * plotH;
  const ticks = [0, max / 2, max];

  const hovered = hover !== null ? daily[hover] : undefined;

  return (
    <figure className="daily-chart" ref={ref}>
      <div className="legend" aria-hidden="true">
        <span>
          <i className="dot dot-claude" /> Claude
        </span>
        <span>
          <i className="dot dot-codex" /> Codex{codexEstimate ? " (odhad)" : ""}
        </span>
      </div>
      <svg viewBox={`0 0 ${W} ${H}`} className="daily-svg" role="img" aria-label="Denně v cenách API, Claude a Codex na sobě">
        <g className="grid">
          {ticks.map((t) => (
            <g key={t}>
              <line x1={M.left} x2={W - M.right} y1={y(t)} y2={y(t)} className={t === 0 ? "baseline" : "gridline"} />
              <text x={M.left - 8} y={y(t) + 4} textAnchor="end" className="axis-label">
                {formatMoneyTick(t, currency)}
              </text>
            </g>
          ))}
        </g>

        {Array.from({ length: slots }, (_, i) => {
          const day = i + 1;
          const x = M.left + i * slotW + (slotW - barW) / 2;
          const d = daily[i];
          const showLabel = day % 7 === 1; // 1., 8., 15., 22., 29.
          const label = showLabel ? (
            <text key={`l${day}`} x={x + barW / 2} y={H - 6} textAnchor="middle" className="axis-label">
              {day}.
            </text>
          ) : null;
          if (!d) return label;

          const claudeH = (d.claudeDisplay / max) * plotH;
          const codexH = (d.codexDisplay / max) * plotH;
          const base = M.top + plotH;
          const hasCodex = codexH > 0.5;
          const claudeTop = base - claudeH;
          const codexTop = claudeTop - (hasCodex && claudeH > 0 ? GAP : 0) - codexH;
          return (
            <g key={d.date} className={hover === i ? "day is-hover" : "day"} onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
              <rect className="day-hit" x={M.left + i * slotW} y={M.top} width={slotW} height={plotH}>
                <title>{`${shortDate(d.date)}: Claude ${formatMoney(d.claudeDisplay, currency)}, Codex ${formatMoney(d.codexDisplay, currency)}`}</title>
              </rect>
              {claudeH > 0 && (
                <path className="seg seg-claude" d={hasCodex ? `M${x} ${base} V${claudeTop} H${x + barW} V${base} Z` : topRounded(x, claudeTop, barW, claudeH, 3)} />
              )}
              {hasCodex && <path className="seg seg-codex" d={topRounded(x, codexTop, barW, codexH, 3)} />}
              {label}
            </g>
          );
        })}

        {hovered && hover !== null && <Tooltip index={hover} slotW={slotW} width={W} day={hovered} currency={currency} y={y} />}
      </svg>
    </figure>
  );
}

function Tooltip({
  index,
  slotW,
  width: W,
  day,
  currency,
  y,
}: {
  index: number;
  slotW: number;
  width: number;
  day: Snapshot["daily"][number];
  currency: Snapshot["currency"];
  y: (v: number) => number;
}) {
  const boxW = 132;
  const boxH = 58;
  const cx = M.left + index * slotW + slotW / 2;
  const bx = Math.min(Math.max(cx - boxW / 2, M.left), W - M.right - boxW);
  const top = y(day.claudeDisplay + day.codexDisplay) - boxH - 8;
  const by = Math.max(M.top, top);
  return (
    <g className="chart-tooltip" pointerEvents="none">
      <rect x={bx} y={by} width={boxW} height={boxH} rx="8" />
      <text x={bx + 10} y={by + 17} className="tt-title">
        {shortDate(day.date)} · {formatMoney(day.claudeDisplay + day.codexDisplay, currency)}
      </text>
      <circle cx={bx + 13} cy={by + 31} r="3.5" className="tt-claude" />
      <text x={bx + 22} y={by + 35} className="tt-row">
        Claude {formatMoney(day.claudeDisplay, currency)}
      </text>
      <circle cx={bx + 13} cy={by + 46} r="3.5" className="tt-codex" />
      <text x={bx + 22} y={by + 50} className="tt-row">
        Codex {formatMoney(day.codexDisplay, currency)}
      </text>
    </g>
  );
}
