import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defaultSettings, fixtures, nowSec } from "./fixtures";

vi.mock("./api", () => ({
  isTauri: () => false,
  getSnapshot: vi.fn(),
  getSettings: vi.fn(),
  setSettings: vi.fn(() => Promise.resolve()),
  setClickThrough: vi.fn(() => Promise.resolve()),
  setWidgetTransparency: vi.fn(() => Promise.resolve()),
  openApp: vi.fn(() => Promise.resolve()),
  clearHistory: vi.fn(() => Promise.resolve()),
  onSnapshot: vi.fn(() => () => {}),
  feedbackStatus: vi.fn(() => new Promise(() => {})),
  sendFeedback: vi.fn(),
  onOpenFeedback: vi.fn(() => () => {}),
}));

import * as api from "./api";
import { AppWindow } from "./AppWindow";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getSnapshot).mockResolvedValue(fixtures.normal(nowSec()));
  vi.mocked(api.getSettings).mockResolvedValue(defaultSettings());
  vi.mocked(api.onSnapshot).mockImplementation(() => () => {});
});

async function openSettings() {
  const user = userEvent.setup();
  render(<AppWindow />);
  await user.click(screen.getByRole("tab", { name: "Nastavení" }));
  await screen.findByRole("form", { name: "Nastavení" });
  return user;
}

describe("AppWindow · Měsíc", () => {
  it("shows score, multiples, sources, models, projects and streaks", async () => {
    render(<AppWindow />);
    expect(await screen.findByText("Skóre: říjen")).toBeInTheDocument();
    expect(screen.getByText(/vytěženo/)).toHaveTextContent("vytěženo 4,6× paušálu");
    expect(screen.getByText("T3 Code a další SDK")).toBeInTheDocument();
    expect(screen.getByText("Terminál")).toBeInTheDocument();
    expect(screen.getByText("Claude Desktop")).toBeInTheDocument();
    expect(screen.getByText("claude-opus-5-5")).toBeInTheDocument();
    const projects = screen.getByRole("heading", { name: "Projekty" }).closest("section") as HTMLElement;
    expect(within(projects).getByText("e-shop-api")).toBeInTheDocument();
    expect(screen.getByText("12 dní")).toBeInTheDocument();
    expect(screen.getByText("4 dny")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /Denně v cenách API/ })).toBeInTheDocument();
  });

  it("long breakdowns show the top 8 and expand on demand", async () => {
    const s = fixtures.normal(nowSec());
    s.byProject = Array.from({ length: 12 }, (_, i) => ({
      key: `projekt-${String(i + 1).padStart(2, "0")} s hodně dlouhým názvem složky`,
      usd: 100 - i,
      display: 100 - i,
      tokens: 1_000_000,
    }));
    vi.mocked(api.getSnapshot).mockResolvedValue(s);
    const user = userEvent.setup();
    render(<AppWindow />);
    const projects = (await screen.findByRole("heading", { name: "Projekty" })).closest("section") as HTMLElement;
    expect(within(projects).getAllByRole("row")).toHaveLength(1 + 8);
    expect(within(projects).getByText(/projekt-01/)).toHaveAttribute("title", "projekt-01 s hodně dlouhým názvem složky");
    expect(within(projects).queryByText(/projekt-09/)).not.toBeInTheDocument();

    await user.click(within(projects).getByRole("button", { name: "Zobrazit všechny (12) · ještě 4" }));
    expect(within(projects).getAllByRole("row")).toHaveLength(1 + 12);
    await user.click(within(projects).getByRole("button", { name: "Zobrazit méně" }));
    expect(within(projects).getAllByRole("row")).toHaveLength(1 + 8);
  });
});

describe("AppWindow · Stavy", () => {
  it("lists every mood in priority order and marks the current ones", async () => {
    const user = userEvent.setup();
    render(<AppWindow />);
    await user.click(await screen.findByRole("tab", { name: "Stavy" }));
    const moods = screen.getByRole("heading", { name: "Nálady postaviček" }).closest("section") as HTMLElement;
    const items = within(moods).getAllByRole("listitem");
    expect(items.map((li) => li.querySelector("h3")?.firstChild?.textContent)).toEqual([
      "Zmatek",
      "Dřímání",
      "Panika",
      "Pot",
      "Oslava",
      "Soustředění",
      "Spánek",
      "Klid",
    ]);
    // normal fixture: Claude focuses, Codex is calm.
    const focus = items[5] as HTMLElement;
    expect(focus).toHaveClass("is-now");
    expect(within(focus).getByText("teď Sunnie")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: "Skylimit" })).toBeInTheDocument();
    expect(within(focus).getByText("Sunnie počítá · Dropie píše kód")).toBeInTheDocument();
    expect(within(items[7] as HTMLElement).getByText("teď Dropie")).toBeInTheDocument();
    expect(within(items[6] as HTMLElement).getByText("30 minut bez práce.")).toBeInTheDocument();

    expect(screen.getByRole("heading", { name: "Řádky limitů" })).toBeInTheDocument();
    expect(screen.getByText(/T3 Code zapisuje do svých logů/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Ikona v oznamovací oblasti" })).toBeInTheDocument();
  });

  it("hovering a mood thumbnail shows a large animated preview", async () => {
    const user = userEvent.setup();
    render(<AppWindow />);
    await user.click(await screen.findByRole("tab", { name: "Stavy" }));
    const thumb = screen.getByRole("button", { name: "Velký náhled: panika" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    fireEvent.mouseEnter(thumb);
    const preview = screen.getByRole("tooltip");
    expect(preview.parentElement).toBe(document.body);
    expect(within(preview).getByText("Panika")).toBeInTheDocument();
    expect(within(preview).getByRole("img", { name: "Sunnie: panika" })).toHaveClass("mood-panic");
    expect(within(preview).getByRole("img", { name: "Dropie: panika" })).toHaveClass("mood-panic");
    expect(thumb).toHaveAttribute("aria-describedby", "mood-preview");

    fireEvent.mouseLeave(thumb);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    // Keyboard: focus opens it, Escape closes it.
    fireEvent.focus(screen.getByRole("button", { name: "Velký náhled: spánek" }));
    expect(within(screen.getByRole("tooltip")).getByRole("img", { name: "Sunnie: spánek" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("button", { name: "Velký náhled: spánek" }), { key: "Escape" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("opens straight on the tab from ?tab=states", async () => {
    window.history.replaceState(null, "", "/?window=app&tab=states");
    render(<AppWindow />);
    expect(await screen.findByRole("heading", { name: "Nálady postaviček" })).toBeInTheDocument();
    window.history.replaceState(null, "", "/");
  });
});

describe("AppWindow · Nastavení", () => {
  it("saves the edited settings as one payload", async () => {
    const user = await openSettings();

    await user.click(screen.getByLabelText("CZK (Kč)"));
    const amount = screen.getByLabelText("Částka paušálu Claude");
    await user.clear(amount);
    await user.type(amount, "250,5");
    await user.selectOptions(screen.getByLabelText("Měna paušálu Claude"), "EUR");
    await user.selectOptions(screen.getByLabelText("Období paušálu Claude"), "year");
    await user.click(screen.getByLabelText("Paušál Codex nezadáno"));
    await user.click(screen.getByLabelText("Započítat odhad Codexu do skóre"));
    await user.click(screen.getByLabelText(/Terminál/));
    await user.click(screen.getByRole("button", { name: "Uložit" }));

    expect(api.setSettings).toHaveBeenCalledTimes(1);
    expect(api.setSettings).toHaveBeenCalledWith({
      currency: "CZK",
      plans: {
        claude: { amount: 250.5, currency: "EUR", period: "year" },
        codex: null,
      },
      codexEstimateInScore: false,
      excludedEntrypoints: ["cli"],
      clickThrough: false,
      widgetTransparency: 0,
    });
    expect(await screen.findByText("Uloženo.")).toBeInTheDocument();
  });

  it("refuses to save a plan without a valid amount", async () => {
    const user = await openSettings();
    const amount = screen.getByLabelText("Částka paušálu Codex");
    await user.clear(amount);
    await user.type(amount, "abc");
    await user.click(screen.getByRole("button", { name: "Uložit" }));
    expect(api.setSettings).not.toHaveBeenCalled();
    expect(screen.getByText(/Paušál Codex: zadej kladnou částku/)).toBeInTheDocument();
  });

  it("transparency slider applies immediately and shows the value", async () => {
    await openSettings();
    const slider = screen.getByLabelText("Průhlednost widgetu");
    fireEvent.change(slider, { target: { value: "40" } });
    expect(api.setWidgetTransparency).toHaveBeenCalledWith(40);
    expect(screen.getByText("40 %")).toBeInTheDocument();
    expect(slider).toHaveAttribute("max", "90");
  });

  it("click-through toggle applies immediately", async () => {
    const user = await openSettings();
    await user.click(screen.getByLabelText("Průchod kliknutí (widget nereaguje na myš)"));
    expect(api.setClickThrough).toHaveBeenCalledWith(true);
  });

  it("clearing history needs an explicit confirmation", async () => {
    const user = await openSettings();
    await user.click(screen.getByRole("button", { name: "Smazat historii" }));
    expect(api.clearHistory).not.toHaveBeenCalled();

    const dialog = screen.getByRole("alertdialog", { name: "Potvrzení smazání historie" });
    await user.click(within(dialog).getByRole("button", { name: "Zrušit" }));
    expect(api.clearHistory).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Smazat historii" }));
    await user.click(screen.getByRole("button", { name: "Ano, smazat natrvalo" }));
    expect(api.clearHistory).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("Historie smazána.")).toBeInTheDocument();
  });
});
