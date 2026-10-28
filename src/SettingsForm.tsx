// Settings tab of the app window. Edits a local draft; "Uložit" sends the whole
// Settings object to the core. Click-through and widget transparency apply
// immediately because the user wants to see the widget react while the app
// window is open. Two columns on wide windows, one below 980 px (app.css).

import { useEffect, useRef, useState, type FormEvent } from "react";
import { clearHistory, getSettings, setClickThrough, setSettings, setWidgetTransparency } from "./api";
import { PROVIDER_NAME, sourceLabel } from "./format";
import { FeedbackIcon } from "./icons";
import type { PlanSetting, Provider, Settings, Snapshot } from "./types";

type PlanCurrency = PlanSetting["currency"];
type PlanPeriod = PlanSetting["period"];

interface PlanDraft {
  unset: boolean;
  amount: string;
  currency: PlanCurrency;
  period: PlanPeriod;
}

interface Draft {
  currency: Settings["currency"];
  plans: Record<Provider, PlanDraft>;
  codexEstimateInScore: boolean;
  excludedEntrypoints: string[];
  clickThrough: boolean;
  widgetTransparency: number;
}

/** Same cap as the core (WIDGET_TRANSPARENCY_MAX). */
export const TRANSPARENCY_MAX = 90;

const PROVIDERS: Provider[] = ["claude", "codex"];

function toPlanDraft(plan: PlanSetting | null): PlanDraft {
  return plan
    ? { unset: false, amount: String(plan.amount).replace(".", ","), currency: plan.currency, period: plan.period }
    : { unset: true, amount: "", currency: "USD", period: "month" };
}

function toDraft(s: Settings): Draft {
  return {
    currency: s.currency,
    plans: { claude: toPlanDraft(s.plans.claude), codex: toPlanDraft(s.plans.codex) },
    codexEstimateInScore: s.codexEstimateInScore,
    excludedEntrypoints: [...s.excludedEntrypoints],
    clickThrough: s.clickThrough,
    widgetTransparency: s.widgetTransparency,
  };
}

/** "1 234,50" → 1234.5; null when not a positive number. */
export function parseAmount(text: string): number | null {
  // \s also covers the (narrow) non-breaking spaces Czech number formats use.
  const cleaned = text.replace(/\s/g, "").replace(",", ".");
  if (!/^\d+(\.\d+)?$/.test(cleaned)) return null;
  const value = Number(cleaned);
  return Number.isFinite(value) && value > 0 ? value : null;
}

function toSettings(d: Draft): Settings | { error: string } {
  const plans = {} as Settings["plans"];
  for (const p of PROVIDERS) {
    const plan = d.plans[p];
    if (plan.unset) {
      plans[p] = null;
      continue;
    }
    const amount = parseAmount(plan.amount);
    if (amount === null) return { error: `Paušál ${PROVIDER_NAME[p]}: zadej kladnou částku, nebo zaškrtni „nezadáno“.` };
    plans[p] = { amount, currency: plan.currency, period: plan.period };
  }
  return {
    currency: d.currency,
    plans,
    codexEstimateInScore: d.codexEstimateInScore,
    excludedEntrypoints: d.excludedEntrypoints,
    clickThrough: d.clickThrough,
    widgetTransparency: d.widgetTransparency,
  };
}

type Status = { kind: "idle" } | { kind: "saving" } | { kind: "saved" } | { kind: "error"; text: string };
type ClearState = "idle" | "confirm" | "busy" | "done" | "error";

export function SettingsForm({ snapshot, onOpenFeedback }: { snapshot: Snapshot | null; onOpenFeedback: () => void }) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [clear, setClear] = useState<ClearState>("idle");
  // Slider values go out one at a time, newest wins: commands run concurrently
  // in the shell, so firing every step could leave an older value saved last.
  const transparencyQueue = useRef<{ busy: boolean; next: number | null }>({ busy: false, next: null });

  useEffect(() => {
    let alive = true;
    getSettings().then(
      (s) => alive && setDraft(toDraft(s)),
      (err: unknown) => alive && setLoadError(String(err)),
    );
    return () => {
      alive = false;
    };
  }, []);

  if (loadError) return <p className="app-empty">Nastavení se nepodařilo načíst: {loadError}</p>;
  if (!draft) return <p className="app-empty">Načítám nastavení…</p>;

  const update = (patch: Partial<Draft>) => {
    setDraft({ ...draft, ...patch });
    setStatus({ kind: "idle" });
  };
  const updatePlan = (p: Provider, patch: Partial<PlanDraft>) =>
    update({ plans: { ...draft.plans, [p]: { ...draft.plans[p], ...patch } } });

  // Entry points seen this month plus any excluded earlier but quiet now.
  const sourceKeys = [
    ...(snapshot?.bySource.map((r) => r.key) ?? []),
    ...draft.excludedEntrypoints.filter((k) => !snapshot?.bySource.some((r) => r.key === k)),
  ];

  const toggleExcluded = (key: string, excluded: boolean) => {
    const next = excluded ? [...draft.excludedEntrypoints, key] : draft.excludedEntrypoints.filter((k) => k !== key);
    update({ excludedEntrypoints: sourceKeys.filter((k) => next.includes(k)) });
  };

  const onClickThrough = (on: boolean) => {
    update({ clickThrough: on });
    setClickThrough(on).catch((err: unknown) => setStatus({ kind: "error", text: `Průchod kliknutí se nepřepnul: ${String(err)}` }));
  };

  const onTransparency = (value: number) => {
    update({ widgetTransparency: value });
    const queue = transparencyQueue.current;
    queue.next = value;
    if (queue.busy) return;
    queue.busy = true;
    void (async () => {
      while (queue.next !== null) {
        const v = queue.next;
        queue.next = null;
        try {
          await setWidgetTransparency(v);
        } catch (err) {
          setStatus({ kind: "error", text: `Průhlednost se nepřepnula: ${String(err)}` });
        }
      }
      queue.busy = false;
    })();
  };

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const result = toSettings(draft);
    if ("error" in result) {
      setStatus({ kind: "error", text: result.error });
      return;
    }
    setStatus({ kind: "saving" });
    try {
      await setSettings(result);
      setStatus({ kind: "saved" });
    } catch (err) {
      setStatus({ kind: "error", text: `Uložení selhalo: ${String(err)}` });
    }
  };

  const onConfirmClear = async () => {
    setClear("busy");
    try {
      await clearHistory();
      setClear("done");
    } catch {
      setClear("error");
    }
  };

  return (
    <form className="settings" onSubmit={onSubmit} aria-label="Nastavení">
      <div className="settings-col">
        <section className="card settings-card">
          <h2>Paušál</h2>
          <p className="hint">Kolik platíš za předplatné. Z toho se počítá násobek „vytěženo ×“.</p>
          {PROVIDERS.map((p) => {
            const plan = draft.plans[p];
            const name = PROVIDER_NAME[p];
            return (
              <div key={p} className="plan-row">
                <span className="plan-name">
                  <i className={`dot dot-${p}`} aria-hidden="true" />
                  {name}
                </span>
                <input
                  className="input amount"
                  type="text"
                  inputMode="decimal"
                  aria-label={`Částka paušálu ${name}`}
                  placeholder="částka"
                  value={plan.amount}
                  disabled={plan.unset}
                  onChange={(e) => updatePlan(p, { amount: e.target.value })}
                />
                <select
                  className="input"
                  aria-label={`Měna paušálu ${name}`}
                  value={plan.currency}
                  disabled={plan.unset}
                  onChange={(e) => updatePlan(p, { currency: e.target.value as PlanCurrency })}
                >
                  <option value="EUR">EUR</option>
                  <option value="CZK">CZK</option>
                  <option value="USD">USD</option>
                </select>
                <select
                  className="input"
                  aria-label={`Období paušálu ${name}`}
                  value={plan.period}
                  disabled={plan.unset}
                  onChange={(e) => updatePlan(p, { period: e.target.value as PlanPeriod })}
                >
                  <option value="month">za měsíc</option>
                  <option value="year">za rok</option>
                </select>
                <label className="check compact">
                  <input
                    type="checkbox"
                    aria-label={`Paušál ${name} nezadáno`}
                    checked={plan.unset}
                    onChange={(e) => updatePlan(p, { unset: e.target.checked })}
                  />
                  nezadáno
                </label>
              </div>
            );
          })}
        </section>

        <section className="card settings-card">
          <h2>Skóre</h2>
          <label className="check">
            <input
              type="checkbox"
              checked={draft.codexEstimateInScore}
              onChange={(e) => update({ codexEstimateInScore: e.target.checked })}
            />
            Započítat odhad Codexu do skóre
          </label>
          <fieldset className="field">
            <legend>Nepočítat do skóre</legend>
            <p className="hint">
              Skóre počítá vše z lokálních záznamů. Provoz mimo předplatné (třeba aplikace nad SDK s vlastním API klíčem) tu
              můžeš vyřadit.
            </p>
            {sourceKeys.length === 0 ? (
              <p className="hint">Zatím žádné zdroje.</p>
            ) : (
              <div className="check-list">
                {sourceKeys.map((key) => (
                  <label key={key} className="check">
                    <input
                      type="checkbox"
                      checked={draft.excludedEntrypoints.includes(key)}
                      onChange={(e) => toggleExcluded(key, e.target.checked)}
                    />
                    {sourceLabel(key)} <span className="key">({key})</span>
                  </label>
                ))}
              </div>
            )}
          </fieldset>
        </section>
      </div>

      <div className="settings-col">
        <section className="card settings-card">
          <h2>Zobrazení</h2>
          <fieldset className="field">
            <legend>Měna skóre</legend>
            <div className="segmented">
              {(["EUR", "CZK"] as const).map((c) => (
                <label key={c} className={draft.currency === c ? "is-on" : undefined}>
                  <input type="radio" name="currency" value={c} checked={draft.currency === c} onChange={() => update({ currency: c })} />
                  {c === "EUR" ? "EUR (€)" : "CZK (Kč)"}
                </label>
              ))}
            </div>
          </fieldset>
        </section>

        <section className="card settings-card">
          <h2>Widget</h2>
          <div className="range-field">
            <label htmlFor="widget-transparency">Průhlednost widgetu</label>
            <output htmlFor="widget-transparency" className="range-value num">
              {draft.widgetTransparency} %
            </output>
            <input
              id="widget-transparency"
              className="range"
              type="range"
              min={0}
              max={TRANSPARENCY_MAX}
              step={5}
              value={draft.widgetTransparency}
              onChange={(e) => onTransparency(Number(e.target.value))}
            />
          </div>
          <p className="hint">Průhledné je jen pozadí; text, pruhy a postavičky zůstanou čitelné. Projeví se hned.</p>
          <label className="check">
            <input type="checkbox" checked={draft.clickThrough} onChange={(e) => onClickThrough(e.target.checked)} />
            Průchod kliknutí (widget nereaguje na myš)
          </label>
          <p className="hint">Vypnout jde i z ikony v oznamovací oblasti.</p>
        </section>

        <section className="card settings-card about">
          <h2>O aplikaci</h2>
          <p className="about-name">
            <b>Skylimit</b> <span className="num">{__APP_VERSION__}</span>
          </p>
          <p className="hint">
            Widget, který hlídá limity Claude a Codexu a počítá práci v cenách API.
            <br />
            &copy; 2026 Michal Jašek &middot; licence MIT &middot; github.com/myspulin24/skylimit
          </p>
          <button type="button" className="btn with-icon" onClick={onOpenFeedback}>
            <FeedbackIcon />
            Poslat zpětnou vazbu
          </button>
        </section>

        <section className="card settings-card danger-zone">
          <h2>Historie</h2>
          <p className="hint">
            Historie skóre je jediná kopie: Claude Code staré záznamy časem maže, takže smazanou historii už nepůjde obnovit.
          </p>
          {clear === "confirm" || clear === "busy" ? (
            <div className="confirm" role="alertdialog" aria-label="Potvrzení smazání historie">
              <span>Opravdu smazat celou historii? Tohle nejde vrátit.</span>
              <button type="button" className="btn danger" disabled={clear === "busy"} onClick={() => void onConfirmClear()}>
                Ano, smazat natrvalo
              </button>
              <button type="button" className="btn" disabled={clear === "busy"} onClick={() => setClear("idle")}>
                Zrušit
              </button>
            </div>
          ) : (
            <div className="confirm">
              <button type="button" className="btn danger-outline" onClick={() => setClear("confirm")}>
                Smazat historii
              </button>
              {clear === "done" && <span className="status ok">Historie smazána.</span>}
              {clear === "error" && <span className="status err">Historii se nepodařilo smazat.</span>}
            </div>
          )}
        </section>
      </div>

      <div className="settings-actions">
        <span className={`status ${status.kind === "error" ? "err" : "ok"}`} role="status">
          {status.kind === "saved" ? "Uloženo." : status.kind === "saving" ? "Ukládám…" : status.kind === "error" ? status.text : ""}
        </span>
        <button type="submit" className="btn primary" disabled={status.kind === "saving"}>
          Uložit
        </button>
      </div>
    </form>
  );
}
