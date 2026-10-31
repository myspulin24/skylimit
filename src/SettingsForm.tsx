// Settings tab of the app window. Edits a local draft; "Uložit" sends the whole
// Settings object to the core. Click-through and widget transparency apply
// immediately because the user wants to see the widget react while the app
// window is open; so does starting at login (AutostartCheck), which lives in
// the system, not in Settings. One centred column of titled groups (two on
// wide windows), each a list of rows with the text left and the control right
// (controls.tsx, app.css "settings").

import { useEffect, useRef, useState, type CSSProperties, type FormEvent } from "react";
import { AccountsCard } from "./AccountsCard";
import { clearHistory, getSettings, setClickThrough, setSettings, setWidgetTransparency } from "./api";
import { AutostartCheck } from "./AutostartCheck";
import { ClaudeCodeCard } from "./ClaudeCodeCard";
import { Group, Row, Select, Toggle, ToggleRow } from "./controls";
import { PROVIDER_NAME, sourceLabel } from "./format";
import { FeedbackIcon } from "./icons";
import { UpdateLine } from "./update";
import type { PlanSetting, Provider, Settings, Snapshot, UpdateStatus, WidgetAmount, WidgetPeriod } from "./types";

type PlanCurrency = PlanSetting["currency"];
type PlanPeriod = PlanSetting["period"];

/** An empty amount means no plan; currency and period stay as chosen. */
interface PlanDraft {
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
  checkUpdates: boolean;
  widgetPeriod: WidgetPeriod;
  widgetAmount: WidgetAmount;
}

/** Same cap as the core (WIDGET_TRANSPARENCY_MAX). */
export const TRANSPARENCY_MAX = 90;

const PROVIDERS: Provider[] = ["claude", "codex"];

const WIDGET_PERIODS: { id: WidgetPeriod; label: string }[] = [
  { id: "month", label: "tento měsíc" },
  { id: "24h", label: "posledních 24 hodin" },
  { id: "7d", label: "posledních 7 dní" },
  { id: "30d", label: "posledních 30 dní" },
  { id: "90d", label: "posledních 90 dní" },
  { id: "all", label: "celou historii" },
];

const WIDGET_AMOUNTS: { id: WidgetAmount; label: string }[] = [
  { id: "api", label: "V cenách API" },
  { id: "withCacheSavings", label: "Vč. úspory z cache" },
];

function toPlanDraft(plan: PlanSetting | null): PlanDraft {
  return plan
    ? { amount: String(plan.amount).replace(".", ","), currency: plan.currency, period: plan.period }
    : { amount: "", currency: "USD", period: "month" };
}

function toDraft(s: Settings): Draft {
  return {
    currency: s.currency,
    plans: { claude: toPlanDraft(s.plans.claude), codex: toPlanDraft(s.plans.codex) },
    codexEstimateInScore: s.codexEstimateInScore,
    excludedEntrypoints: [...s.excludedEntrypoints],
    clickThrough: s.clickThrough,
    widgetTransparency: s.widgetTransparency,
    checkUpdates: s.checkUpdates,
    widgetPeriod: s.widgetPeriod,
    widgetAmount: s.widgetAmount,
  };
}

/** What "Uložit" would change: click-through and transparency are already applied. */
function pending(a: Draft, b: Draft): boolean {
  const saved = (d: Draft) => JSON.stringify({ ...d, clickThrough: null, widgetTransparency: null });
  return saved(a) !== saved(b);
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
    if (plan.amount.trim() === "") {
      plans[p] = null;
      continue;
    }
    const amount = parseAmount(plan.amount);
    if (amount === null) return { error: `Paušál ${PROVIDER_NAME[p]}: zadej kladnou částku, nebo pole nech prázdné.` };
    plans[p] = { amount, currency: plan.currency, period: plan.period };
  }
  return {
    currency: d.currency,
    plans,
    codexEstimateInScore: d.codexEstimateInScore,
    excludedEntrypoints: d.excludedEntrypoints,
    clickThrough: d.clickThrough,
    widgetTransparency: d.widgetTransparency,
    checkUpdates: d.checkUpdates,
    widgetPeriod: d.widgetPeriod,
    widgetAmount: d.widgetAmount,
  };
}

type Status = { kind: "idle" } | { kind: "saving" } | { kind: "saved" } | { kind: "error"; text: string };
type ClearState = "idle" | "confirm" | "busy" | "done" | "error";

export function SettingsForm({
  snapshot,
  update,
  onOpenFeedback,
}: {
  snapshot: Snapshot | null;
  update: UpdateStatus | null;
  onOpenFeedback: () => void;
}) {
  const [draft, setDraft] = useState<Draft | null>(null);
  // Last loaded or saved state: the save bar shows only while the draft differs.
  const [baseline, setBaseline] = useState<Draft | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [clear, setClear] = useState<ClearState>("idle");
  // Slider values go out one at a time, newest wins: commands run concurrently
  // in the shell, so firing every step could leave an older value saved last.
  const transparencyQueue = useRef<{ busy: boolean; next: number | null }>({ busy: false, next: null });

  useEffect(() => {
    let alive = true;
    getSettings().then(
      (s) => {
        if (!alive) return;
        setDraft(toDraft(s));
        setBaseline(toDraft(s));
      },
      (err: unknown) => alive && setLoadError(String(err)),
    );
    return () => {
      alive = false;
    };
  }, []);

  if (loadError) return <p className="app-empty">Nastavení se nepodařilo načíst: {loadError}</p>;
  if (!draft) return <p className="app-empty">Načítám nastavení…</p>;

  const update_ = (patch: Partial<Draft>) => {
    setDraft({ ...draft, ...patch });
    setStatus({ kind: "idle" });
  };
  const updatePlan = (p: Provider, patch: Partial<PlanDraft>) =>
    update_({ plans: { ...draft.plans, [p]: { ...draft.plans[p], ...patch } } });

  // Entry points seen this month plus any excluded earlier but quiet now.
  const sourceKeys = [
    ...(snapshot?.bySource.map((r) => r.key) ?? []),
    ...draft.excludedEntrypoints.filter((k) => !snapshot?.bySource.some((r) => r.key === k)),
  ];

  const setCounted = (key: string, counted: boolean) => {
    const next = counted ? draft.excludedEntrypoints.filter((k) => k !== key) : [...draft.excludedEntrypoints, key];
    update_({ excludedEntrypoints: sourceKeys.filter((k) => next.includes(k)) });
  };

  const onClickThrough = (on: boolean) => {
    update_({ clickThrough: on });
    setClickThrough(on).catch((err: unknown) => setStatus({ kind: "error", text: `Průchod kliknutí se nepřepnul: ${String(err)}` }));
  };

  const onTransparency = (value: number) => {
    update_({ widgetTransparency: value });
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
      setBaseline(draft);
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

  const dirty = baseline !== null && pending(draft, baseline);
  const statusText =
    status.kind === "error"
      ? status.text
      : status.kind === "saving"
        ? "Ukládám…"
        : dirty
          ? "Máš neuložené změny."
          : status.kind === "saved"
            ? "Uloženo."
            : "";

  // The slider track fills up to the thumb (app.css reads --fill).
  const fill = { "--fill": `${(draft.widgetTransparency / TRANSPARENCY_MAX) * 100}%` } as CSSProperties;

  return (
    <form className="settings" onSubmit={onSubmit} aria-label="Nastavení">
      <div className="settings-groups">
        <AccountsCard />

        <Group title="Paušál" footer="Kolik platíš za předplatné. Z toho se počítá násobek „vytěženo ×“. Prázdné pole znamená bez paušálu.">
          {PROVIDERS.map((p) => {
            const plan = draft.plans[p];
            const name = PROVIDER_NAME[p];
            return (
              <Row
                key={p}
                title={
                  <span className="provider-title">
                    <i className={`dot dot-${p}`} aria-hidden="true" />
                    {name}
                  </span>
                }
              >
                <div className="plan-controls">
                  <span className="input-group">
                    <input
                      className="input amount"
                      type="text"
                      inputMode="decimal"
                      aria-label={`Částka paušálu ${name}`}
                      placeholder="nezadáno"
                      value={plan.amount}
                      onChange={(e) => updatePlan(p, { amount: e.target.value })}
                    />
                    <Select
                      className="joined"
                      aria-label={`Měna paušálu ${name}`}
                      value={plan.currency}
                      onChange={(e) => updatePlan(p, { currency: e.target.value as PlanCurrency })}
                    >
                      <option value="EUR">EUR</option>
                      <option value="CZK">CZK</option>
                      <option value="USD">USD</option>
                    </Select>
                  </span>
                  <Select
                    aria-label={`Období paušálu ${name}`}
                    value={plan.period}
                    onChange={(e) => updatePlan(p, { period: e.target.value as PlanPeriod })}
                  >
                    <option value="month">měsíčně</option>
                    <option value="year">ročně</option>
                  </Select>
                </div>
              </Row>
            );
          })}
        </Group>

        <Group title="Skóre">
          <Row title="Měna skóre">
            <div className="segmented" role="radiogroup" aria-label="Měna skóre">
              {(["EUR", "CZK"] as const).map((c) => (
                <label key={c} className={draft.currency === c ? "is-on" : undefined}>
                  <input type="radio" name="currency" value={c} checked={draft.currency === c} onChange={() => update_({ currency: c })} />
                  {c === "EUR" ? "EUR (€)" : "CZK (Kč)"}
                </label>
              ))}
            </div>
          </Row>
          <ToggleRow
            title="Započítat odhad Codexu do skóre"
            desc="Codex hlásí jen denní součty tokenů, jeho cena je proto odhad."
            checked={draft.codexEstimateInScore}
            onChange={(on) => update_({ codexEstimateInScore: on })}
          />
          <Row
            stacked
            title="Co se počítá do skóre"
            desc="Skóre počítá vše z lokálních záznamů. Provoz mimo předplatné (třeba aplikaci nad SDK s vlastním API klíčem) tu můžeš vypnout."
          >
            {sourceKeys.length === 0 ? (
              <p className="row-desc">Zatím žádné zdroje.</p>
            ) : (
              <ul className="source-list">
                {sourceKeys.map((key) => (
                  <li key={key}>
                    <label className="source-item">
                      <span className="source-name">
                        {sourceLabel(key)} <span className="key">{key}</span>
                      </span>
                      <Toggle
                        label={`${sourceLabel(key)} (${key})`}
                        checked={!draft.excludedEntrypoints.includes(key)}
                        onChange={(counted) => setCounted(key, counted)}
                      />
                    </label>
                  </li>
                ))}
              </ul>
            )}
          </Row>
        </Group>

        <Group title="Widget">
          <Row title="Cena ve widgetu za" desc="Násobek paušálu se u jiného období než měsíce přepočte na jeho délku.">
            <Select
              aria-label="Cena ve widgetu za"
              value={draft.widgetPeriod}
              onChange={(e) => update_({ widgetPeriod: e.target.value as WidgetPeriod })}
            >
              {WIDGET_PERIODS.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
            </Select>
          </Row>
          <Row
            title="Cena ve widgetu"
            desc="Vč. úspory z cache: k ceně se přičte, kolik ušetřila cache, tedy kolik by stejná práce stála v cenách API bez ní."
          >
            <div className="segmented" role="radiogroup" aria-label="Cena ve widgetu">
              {WIDGET_AMOUNTS.map((a) => (
                <label key={a.id} className={draft.widgetAmount === a.id ? "is-on" : undefined}>
                  <input
                    type="radio"
                    name="widget-amount"
                    value={a.id}
                    checked={draft.widgetAmount === a.id}
                    onChange={() => update_({ widgetAmount: a.id })}
                  />
                  {a.label}
                </label>
              ))}
            </div>
          </Row>
          <div className="row">
            <div className="row-text">
              <label className="row-title" htmlFor="widget-transparency">
                Průhlednost widgetu
              </label>
              <div className="row-desc">Průhledné je jen pozadí, text, pruhy a postavičky zůstanou čitelné.</div>
            </div>
            <div className="row-control range-control">
              <input
                id="widget-transparency"
                className="range"
                type="range"
                min={0}
                max={TRANSPARENCY_MAX}
                step={5}
                style={fill}
                value={draft.widgetTransparency}
                onChange={(e) => onTransparency(Number(e.target.value))}
              />
              <output htmlFor="widget-transparency" className="range-value num">
                {draft.widgetTransparency} %
              </output>
            </div>
          </div>
          <ToggleRow
            title="Průchod kliknutí"
            desc="Widget nereaguje na myš. Vypnout jde i z ikony v oznamovací oblasti."
            checked={draft.clickThrough}
            onChange={onClickThrough}
          />
          <AutostartCheck />
        </Group>

        <ClaudeCodeCard />

        <Group title="O aplikaci">
          <Row
            title={
              <span className="about-name">
                Skylimit <span className="num">{__APP_VERSION__}</span>
              </span>
            }
            desc="Widget, který hlídá limity Claude a Codexu a počítá práci v cenách API. © 2026 Michal Jašek · licence MIT · github.com/myspulin24/skylimit"
          >
            <button type="button" className="btn with-icon" onClick={onOpenFeedback}>
              <FeedbackIcon />
              Poslat zpětnou vazbu
            </button>
          </Row>
          <UpdateLine status={update} />
          <ToggleRow
            title="Hledat novou verzi automaticky"
            desc="Po spuštění a pak každých 6 hodin."
            checked={draft.checkUpdates}
            onChange={(on) => update_({ checkUpdates: on })}
          />
        </Group>

        <Group title="Historie" tone="danger">
          {clear === "confirm" || clear === "busy" ? (
            <div className="row confirm-row" role="alertdialog" aria-label="Potvrzení smazání historie">
              <div className="row-text">
                <div className="row-title">Opravdu smazat celou historii?</div>
                <div className="row-desc">Tohle nejde vrátit.</div>
              </div>
              <div className="row-control">
                <button type="button" className="btn" disabled={clear === "busy"} onClick={() => setClear("idle")}>
                  Zrušit
                </button>
                <button type="button" className="btn danger" disabled={clear === "busy"} onClick={() => void onConfirmClear()}>
                  Ano, smazat natrvalo
                </button>
              </div>
            </div>
          ) : (
            <Row
              title="Smazat historii skóre"
              desc={
                clear === "done" ? (
                  <span className="status ok">Historie smazána.</span>
                ) : clear === "error" ? (
                  <span className="status err">Historii se nepodařilo smazat.</span>
                ) : (
                  "Historie je jediná kopie: Claude Code staré záznamy časem maže, takže smazanou historii už nepůjde obnovit."
                )
              }
            >
              <button type="button" className="btn danger-outline" onClick={() => setClear("confirm")}>
                Smazat historii
              </button>
            </Row>
          )}
        </Group>
      </div>

      <div className={dirty || status.kind !== "idle" ? "settings-actions is-shown" : "settings-actions"}>
        <span className={`status ${status.kind === "error" ? "err" : "ok"}`} role="status">
          {statusText}
        </span>
        <button type="submit" className="btn primary" disabled={status.kind === "saving" || (!dirty && status.kind !== "error")}>
          Uložit
        </button>
      </div>
    </form>
  );
}
