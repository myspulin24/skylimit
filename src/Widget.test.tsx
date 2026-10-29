import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fixtures, nowSec } from "./fixtures";
import type { Snapshot } from "./types";

vi.mock("./api", () => ({
  isTauri: () => false,
  getSnapshot: vi.fn(),
  getSettings: vi.fn(),
  setSettings: vi.fn(),
  setClickThrough: vi.fn(),
  setWidgetTransparency: vi.fn(),
  refreshUsage: vi.fn(() => Promise.resolve()),
  openFeedback: vi.fn(() => Promise.resolve()),
  openApp: vi.fn(() => Promise.resolve()),
  clearHistory: vi.fn(),
  onSnapshot: vi.fn(() => () => {}),
  updateStatus: vi.fn(() => Promise.resolve({ phase: "upToDate", current: "0.6.0", version: null, notes: null, date: null, progress: null, error: null, checkedAt: null, auto: true })),
  checkForUpdate: vi.fn(() => Promise.resolve({ phase: "upToDate", current: "0.6.0", version: null, notes: null, date: null, progress: null, error: null, checkedAt: null, auto: true })),
  installUpdate: vi.fn(() => Promise.resolve()),
  onUpdate: vi.fn(() => () => {}),
}));

import * as api from "./api";
import { Widget } from "./Widget";

async function renderWidget(snapshot: Snapshot) {
  vi.mocked(api.getSnapshot).mockResolvedValue(snapshot);
  const view = render(<Widget />);
  await screen.findByLabelText("Skóre");
  return view;
}

const rowOf = (el: HTMLElement) => el.closest(".limit-row") as HTMLElement;

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.onSnapshot).mockImplementation(() => () => {});
});

describe("Widget", () => {
  it("normal: shows percent and a live reset countdown", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    const row = rowOf(screen.getByText("Codex · 5h"));
    expect(within(row).getByText("42 %")).toBeInTheDocument();
    expect(within(row).getByText("reset za 1 h 40 min")).toBeInTheDocument();
    expect(row.querySelector(".bar")).toHaveClass("tone-ok");
    expect(screen.getByText("930 €")).toBeInTheDocument();
    expect(screen.getByText("tento měsíc v cenách API")).toBeInTheDocument();
    expect(screen.getByText("4,6×")).toBeInTheDocument();
  });

  it("Claude limits from T3 Code: 5h and week rows like Codex", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    const five = rowOf(screen.getByText("Claude · 5h"));
    expect(within(five).getByText("18 %")).toBeInTheDocument();
    expect(within(five).getByText("reset za 3 h 20 min")).toBeInTheDocument();
    const week = rowOf(screen.getByText("Claude · týden"));
    expect(within(week).getByText("34 %")).toBeInTheDocument();
  });

  it("Claude's weekly Fable limit is a row like the others, right after the week; Codex's model limits do not show", async () => {
    const now = nowSec();
    const s = fixtures.normal(now);
    s.limits[1]!.windows.push({ kind: "weekModel", label: "týden · gpt-6-spark", percent: 50, resetsAt: now + 3600, fetchedAt: now, state: "known" });
    await renderWidget(s);
    const fable = rowOf(screen.getByText("Claude · Fable"));
    const week = rowOf(screen.getByText("Claude · týden"));
    expect(fable.className).toBe(week.className);
    expect(within(fable).getByText("9 %")).toBeInTheDocument();
    expect(within(fable).getByText("reset za 5 d 2 h")).toBeInTheDocument();
    expect(fable.querySelector(".dot-claude")).toBeInTheDocument();
    expect(fable.previousElementSibling).toBe(week);
    expect(screen.queryByText(/gpt-6-spark/)).not.toBeInTheDocument();
  });

  it("a stale Fable reading says how old it is", async () => {
    const now = nowSec();
    const s = fixtures.normal(now);
    s.limits[0]!.windows[2] = { kind: "weekModel", label: "Fable", percent: 61, resetsAt: now + 2 * 86_400, fetchedAt: now - 2 * 3600, state: "stale" };
    await renderWidget(s);
    const fable = rowOf(screen.getByText("Claude · Fable"));
    expect(within(fable).getByText("~61 %")).toBeInTheDocument();
    expect(within(fable).getByText("poslední známá · před 2 h")).toBeInTheDocument();
    expect(fable.querySelector(".bar")).toHaveClass("bar--hatched");
  });

  it("Claude 5h window after a quiet reset: 0 % until the next message", async () => {
    const s = fixtures.normal(nowSec());
    s.limits[0]!.windows[0] = { kind: "5h", label: "5h", percent: 0, resetsAt: null, fetchedAt: nowSec() - 7200, state: "known" };
    await renderWidget(s);
    const row = rowOf(screen.getByText("Claude · 5h"));
    expect(within(row).getByText("0 %")).toBeInTheDocument();
    expect(within(row).getByText("začne první zprávou")).toBeInTheDocument();
  });

  it("without a Claude limits source: one muted line with the reason instead of rows", async () => {
    await renderWidget(fixtures.claudeOff(nowSec()));
    expect(screen.getByText("limity Claude: připoj Claude Code v Nastavení")).toBeInTheDocument();
    expect(screen.queryByText(/Claude · 5h/)).not.toBeInTheDocument();
  });

  it("transparency fades only the backdrop and adds a text shadow from 40 %", async () => {
    const view = await renderWidget(fixtures.normal(nowSec()));
    const shell = view.container.querySelector(".widget-shell") as HTMLElement;
    expect(shell.style.getPropertyValue("--panel-alpha")).toBe("1");
    expect(shell).not.toHaveClass("see-through");
    view.unmount();

    const again = await renderWidget({ ...fixtures.normal(nowSec()), widgetTransparency: 60 });
    const faded = again.container.querySelector(".widget-shell") as HTMLElement;
    expect(faded.style.getPropertyValue("--panel-alpha")).toBe("0.4");
    expect(faded).toHaveClass("see-through");
  });

  it("refresh button asks the core to read everything now, once per spin", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    const button = screen.getByRole("button", { name: "Obnovit data" });
    fireEvent.click(button);
    expect(api.refreshUsage).toHaveBeenCalledTimes(1);
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(button).toHaveClass("is-busy");
    fireEvent.click(button);
    expect(api.refreshUsage).toHaveBeenCalledTimes(1);
    // It sits next to the gear in the footer.
    const actions = button.closest(".foot-actions") as HTMLElement;
    expect(within(actions).getByRole("button", { name: "Otevřít aplikaci" })).toBeInTheDocument();
  });

  it("the speech bubble opens the app with the feedback dialog", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    const button = screen.getByRole("button", { name: "Poslat zpětnou vazbu" });
    expect(button.closest(".foot-actions")).not.toBeNull();
    fireEvent.click(button);
    expect(api.openFeedback).toHaveBeenCalledTimes(1);
    expect(api.openApp).not.toHaveBeenCalled();
  });

  it("a dot on the gear says a new version is waiting in the app", async () => {
    vi.mocked(api.updateStatus).mockResolvedValueOnce({
      phase: "available", current: "0.6.0", version: "0.7.0", notes: null, date: null,
      progress: null, error: null, checkedAt: null, auto: true,
    });
    await renderWidget(fixtures.normal(nowSec()));
    const gear = await screen.findByRole("button", { name: "Otevřít aplikaci: je k dispozici Skylimit 0.7.0" });
    expect(gear).toHaveClass("has-update");
    expect(gear.querySelector(".update-dot")).not.toBeNull();
    fireEvent.click(gear);
    expect(api.openApp).toHaveBeenCalledTimes(1);
  });

  it("stage tooltip explains what each creature is doing", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    const title = (document.querySelector(".stage") as HTMLElement).getAttribute("title") ?? "";
    expect(title).toContain("Sunnie počítá: práce v posledních 5 minutách");
    expect(title).toContain("Dropie pije kafe: vše v pořádku");
    expect(title).toContain("Všechny stavy: aplikace › Stavy");
  });

  it("stale: hatched bar and 'poslední známá'", async () => {
    await renderWidget(fixtures.stale(nowSec()));
    const row = rowOf(screen.getByText(/poslední známá · před 47 min/));
    expect(row.querySelector(".bar")).toHaveClass("bar--hatched");
    expect(within(row).getByText("~64 %")).toBeInTheDocument();
  });

  it("afterReset: says the current state is unknown", async () => {
    await renderWidget(fixtures.stale(nowSec()));
    const row = rowOf(screen.getByText("po resetu, aktuální stav neznámý"));
    expect(within(row).getByText("?")).toBeInTheDocument();
    expect(row.querySelector(".bar-fill")).toBeNull();
  });

  it("unknown: '?' plus the provider note", async () => {
    await renderWidget(fixtures.codexMissing(nowSec()));
    expect(screen.getByText(/Codex CLI nenalezen v PATH/)).toBeInTheDocument();
    const row = document.querySelector(".limit-row.state-unknown") as HTMLElement;
    expect(within(row).getByText("?")).toBeInTheDocument();
    expect(screen.getByText("Codex bez ceny, paušál nezapočten")).toBeInTheDocument();
  });

  it("Codex estimate is marked with ≈ and 'odhad'", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    const chip = document.querySelector(".chip-codex") as HTMLElement;
    expect(within(chip).getByText("≈ 89 €")).toBeInTheDocument();
    expect(within(chip).getByText("odhad")).toBeInTheDocument();
  });

  it("formatChanged: question mark and the Claude format message", async () => {
    await renderWidget(fixtures.formatChanged(nowSec()));
    expect(screen.getByText("formát záznamů Claude se změnil")).toBeInTheDocument();
    const chip = document.querySelector(".chip-claude") as HTMLElement;
    expect(within(chip).getByText("?")).toBeInTheDocument();
    expect(document.querySelector(".score-q")).toBeInTheDocument();
  });

  it("no plan: the call to action opens the app", async () => {
    await renderWidget(fixtures.noPlan(nowSec()));
    expect(screen.queryByText("4,6×")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Zadej, kolik platíš" }));
    expect(api.openApp).toHaveBeenCalledTimes(1);
  });

  it("backfill: shows progress while history loads", async () => {
    await renderWidget(fixtures.backfill(nowSec()));
    expect(screen.getByText("načítám historii 12/85")).toBeInTheDocument();
  });

  it("backfill line is hidden once done", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    expect(screen.queryByText(/načítám historii/)).not.toBeInTheDocument();
  });

  it("footer: 'aktualizováno před 2 min', the version and the gear opens the app", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    expect(screen.getByText("aktualizováno před 2 min")).toBeInTheDocument();
    expect(screen.getByTitle("Verze Skylimitu")).toHaveTextContent(`v${__APP_VERSION__}`);
    fireEvent.click(screen.getByRole("button", { name: "Otevřít aplikaci" }));
    expect(api.openApp).toHaveBeenCalledTimes(1);
  });

  it("stage carries the Tauri drag region and both mascots", async () => {
    await renderWidget(fixtures.normal(nowSec()));
    const stage = document.querySelector(".stage") as HTMLElement;
    expect(stage).toHaveAttribute("data-tauri-drag-region");
    expect(within(stage).getByRole("img", { name: "Sunnie: soustředění" })).toHaveClass("mood-focus");
    expect(within(stage).getByRole("img", { name: "Dropie: klid" })).toHaveClass("mood-calm");
  });

  it("celebration: a new id shows the burst and its text", async () => {
    await renderWidget(fixtures.celebration(nowSec()));
    expect(screen.getByText("Tento měsíc už 5× paušálu!")).toBeInTheDocument();
    expect(document.querySelectorAll(".particle").length).toBeGreaterThan(0);
  });

  it("applies snapshots pushed through the snapshot event", async () => {
    let push: (s: Snapshot) => void = () => {};
    vi.mocked(api.onSnapshot).mockImplementation((cb) => {
      push = cb;
      return () => {};
    });
    const now = nowSec();
    await renderWidget(fixtures.normal(now));
    act(() => push({ ...fixtures.panic(now), generatedAt: now + 5 }));
    expect(screen.getByText("97 %")).toBeInTheDocument();
    expect(rowOf(screen.getByText("97 %")).querySelector(".bar")).toHaveClass("tone-crit");
    expect(screen.getByRole("img", { name: "Dropie: panika" })).toHaveClass("mood-panic");
  });
});
