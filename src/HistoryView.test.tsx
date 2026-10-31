import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fixtures, nowSec } from "./fixtures";
import { mockHistory } from "./fixtures/history";
import type { HistoryFilter, HistoryRange } from "./types";

vi.mock("./api", () => ({
  getHistory: vi.fn(),
}));

import * as api from "./api";
import { HISTORY_ROWS, HistoryView } from "./HistoryView";

// Intl and our helpers use (narrow) non-breaking spaces; compare as plain text.
const plain = (s: string | null) => (s ?? "").replace(/\s/g, " ");

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getHistory).mockImplementation((range: HistoryRange, filter: HistoryFilter) =>
    Promise.resolve(mockHistory(range, filter, "EUR", nowSec())),
  );
});

const card = (heading: RegExp | string) => screen.getByRole("heading", { name: heading }).closest("section") as HTMLElement;

describe("HistoryView", () => {
  it("shows the last 30 days: total, providers, totals, types and models", async () => {
    render(<HistoryView snapshot={fixtures.normal(nowSec())} />);
    expect(await screen.findByText("Celkem za posledních 30 dní")).toBeInTheDocument();
    expect(api.getHistory).toHaveBeenCalledWith("30d", "all");

    const summary = card("Celkem za posledních 30 dní");
    expect(within(summary).getByText("Claude")).toBeInTheDocument();
    expect(within(summary).getByText("odhad")).toBeInTheDocument();
    expect(plain(within(summary).getByText(/aktivních dnů/).textContent)).toMatch(/v cenách API · \d+ aktivních dnů/);

    const totals = screen.getByRole("region", { name: "Součty" });
    for (const label of ["Zpracované tokeny", "Z cache", "Bez cache", "Výstup", "Úspora díky cache"]) {
      expect(within(totals).getByText(label)).toBeInTheDocument();
    }

    const kinds = card("Cena podle typu");
    expect(within(kinds).getByText("Čtení z cache")).toBeInTheDocument();
    expect(within(kinds).getByText("Codex (bez rozpadu)")).toBeInTheDocument();

    const breakdown = card("Rozpad");
    expect(within(breakdown).getByText("claude-opus-5-5")).toBeInTheDocument();
    expect(within(breakdown).getByText("Codex (bez rozpadu na modely)")).toBeInTheDocument();
    expect(screen.getByText(/Cena Codexu je odhad/)).toBeInTheDocument();
  });

  it("asks the core again for another period and provider", async () => {
    const user = userEvent.setup();
    render(<HistoryView snapshot={fixtures.normal(nowSec())} />);
    await screen.findByText("Celkem za posledních 30 dní");

    await user.click(screen.getByRole("radio", { name: "24 h" }));
    expect(await screen.findByText("Celkem za posledních 24 hodin")).toBeInTheDocument();
    expect(api.getHistory).toHaveBeenLastCalledWith("24h", "all");
    expect(screen.getByRole("heading", { name: "V cenách API po hodinách" })).toBeInTheDocument();
    expect(screen.getByText("Codex hlásí jen denní součty, v pohledu 24 h proto chybí.")).toBeInTheDocument();
    expect(within(card("Celkem za posledních 24 hodin")).queryByText("Codex")).toBeNull();

    await user.click(screen.getByRole("radio", { name: "Codex" }));
    await waitFor(() => expect(api.getHistory).toHaveBeenLastCalledWith("24h", "codex"));
    await user.click(screen.getByRole("radio", { name: "90 dní" }));
    await waitFor(() => expect(api.getHistory).toHaveBeenLastCalledWith("90d", "codex"));
    const totals = await screen.findByRole("region", { name: "Součty" });
    await waitFor(() => expect(within(totals).getAllByText("–")).toHaveLength(5));
  });

  it("switches to tokens and to the breakdown by day and source", async () => {
    const user = userEvent.setup();
    render(<HistoryView snapshot={fixtures.normal(nowSec())} />);
    await screen.findByText("Celkem za posledních 30 dní");

    await user.click(screen.getByRole("radio", { name: "Tokeny" }));
    expect(screen.getByRole("heading", { name: "Tokeny po dnech" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Tokeny podle typu" })).toBeInTheDocument();
    expect(plain(within(card("Celkem za posledních 30 dní")).getByText(/zpracovaných tokenů/).textContent)).toMatch(/^zpracovaných tokenů · /);

    await user.click(within(screen.getByRole("radiogroup", { name: "Období" })).getByRole("radio", { name: "Vše" }));
    await screen.findByText("Celkem za celou historii");
    await user.click(screen.getByRole("radio", { name: "Den" }));
    const breakdown = card("Rozpad");
    const rows = within(breakdown).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(HISTORY_ROWS);
    expect(plain(rows[0]?.textContent ?? "")).toMatch(/^1\s?\S\S \d+\. \d+\. 2026/);
    await user.click(within(breakdown).getByRole("button", { name: /Zobrazit všechny/ }));
    expect(within(breakdown).getAllByRole("row").length).toBeGreaterThan(HISTORY_ROWS + 1);

    await user.click(screen.getByRole("radio", { name: "Zdroj" }));
    expect(within(card("Rozpad")).getByText("Obnoveno z T3 Code")).toBeInTheDocument();
    expect(screen.getByText(/obnovená z mezipaměti T3 Code/)).toBeInTheDocument();
  });

  it("says so when the core does not answer", async () => {
    vi.mocked(api.getHistory).mockRejectedValue(new Error("Jádro aplikace neodpovídá."));
    render(<HistoryView snapshot={null} />);
    expect(await screen.findByText("Historii se nepodařilo načíst: Jádro aplikace neodpovídá.")).toBeInTheDocument();
  });
});
