import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
  feedbackStatus: vi.fn(),
  sendFeedback: vi.fn(),
  onOpenFeedback: vi.fn(() => () => {}),
}));

import * as api from "./api";
import { AppWindow } from "./AppWindow";
import { readAttachment } from "./FeedbackDialog";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getSnapshot).mockResolvedValue(fixtures.normal(nowSec()));
  vi.mocked(api.getSettings).mockResolvedValue(defaultSettings());
  vi.mocked(api.onSnapshot).mockImplementation(() => () => {});
  vi.mocked(api.onOpenFeedback).mockImplementation(() => () => {});
  vi.mocked(api.feedbackStatus).mockResolvedValue({
    available: true,
    host: "skylimit-feedback.michal-jasek.workers.dev",
    version: "0.4.0",
    os: "windows (x86_64)",
    error: "",
  });
});

async function openDialog() {
  const user = userEvent.setup();
  render(<AppWindow />);
  await user.click(await screen.findByRole("button", { name: "Zpětná vazba" }));
  const dialog = await screen.findByRole("dialog", { name: "Zpětná vazba" });
  await within(dialog).findByText(/skylimit-feedback\.michal-jasek\.workers\.dev/);
  return { user, dialog };
}

describe("Zpětná vazba", () => {
  it("shows where the message goes, then sends it with the tab as location", async () => {
    vi.mocked(api.sendFeedback).mockResolvedValue({ id: "abcd1234" });
    const { user, dialog } = await openDialog();
    expect(within(dialog).getByText(/verze 0\.4\.0 a systém windows \(x86_64\)/)).toBeInTheDocument();

    await user.click(within(dialog).getByLabelText("Vylepšení"));
    await user.type(within(dialog).getByLabelText("Zpráva"), "Tmavší widget, prosím.");
    await user.type(within(dialog).getByLabelText(/Jméno/), "Lumír");
    await user.click(within(dialog).getByRole("button", { name: "Odeslat" }));

    expect(api.sendFeedback).toHaveBeenCalledWith({
      kind: "improvement",
      message: "Tmavší widget, prosím.",
      location: "Okno aplikace › Měsíc",
      author: { name: "Lumír", email: "" },
      attachment: null,
    });
    expect(await within(dialog).findByText("Díky! Zpětná vazba dorazila.")).toBeInTheDocument();
    expect(within(dialog).getByText("abcd1234")).toBeInTheDocument();
  });

  it("refuses an empty message and sends anonymously without name and e-mail", async () => {
    vi.mocked(api.sendFeedback).mockResolvedValue({ id: "x1" });
    const { user, dialog } = await openDialog();
    await user.click(within(dialog).getByRole("button", { name: "Odeslat" }));
    expect(api.sendFeedback).not.toHaveBeenCalled();
    expect(within(dialog).getByRole("alert")).toHaveTextContent("Zpráva je prázdná.");

    await user.type(within(dialog).getByLabelText("Zpráva"), "Nejde obnovit.");
    await user.click(within(dialog).getByRole("button", { name: "Odeslat" }));
    expect(vi.mocked(api.sendFeedback).mock.calls[0]?.[0]).toMatchObject({ kind: "bug", author: null });
  });

  it("keeps the text when the Worker says no", async () => {
    vi.mocked(api.sendFeedback).mockRejectedValue("Zpráv přišlo moc najednou. Zkus to za minutu.");
    const { user, dialog } = await openDialog();
    await user.type(within(dialog).getByLabelText("Zpráva"), "Druhý pokus");
    await user.click(within(dialog).getByRole("button", { name: "Odeslat" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("moc najednou");
    expect(within(dialog).getByLabelText("Zpráva")).toHaveValue("Druhý pokus");
  });

  it("closes with Escape and opens straight from ?feedback=1", async () => {
    const { dialog } = await openDialog();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(dialog).not.toBeInTheDocument());

    window.history.replaceState(null, "", "/?window=app&feedback=1");
    try {
      render(<AppWindow />);
      expect(await screen.findByRole("dialog", { name: "Zpětná vazba" })).toBeInTheDocument();
    } finally {
      window.history.replaceState(null, "", "/");
    }
  });

  it("cannot send when the shell says it is unavailable", async () => {
    vi.mocked(api.feedbackStatus).mockResolvedValue({ available: false, host: "", version: "0.4.0", os: "linux", error: "adresa není https" });
    const user = userEvent.setup();
    render(<AppWindow />);
    await user.click(await screen.findByRole("button", { name: "Zpětná vazba" }));
    const dialog = await screen.findByRole("dialog", { name: "Zpětná vazba" });
    expect(await within(dialog).findByText("Odeslat teď nejde: adresa není https")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Odeslat" })).toBeDisabled();
  });
});

describe("readAttachment", () => {
  it("encodes an allowed file and takes the type from the extension", async () => {
    const result = await readAttachment(new File(["ahoj"], "log.txt"));
    expect(result).toEqual({ name: "log.txt", type: "text/plain", data: btoa("ahoj") });
  });

  it("refuses unknown types and files over 2 MB", async () => {
    expect(await readAttachment(new File(["x"], "setup.exe"))).toMatch(/nejde přiložit/);
    const big = new File([new Uint8Array(2 * 1024 * 1024 + 1)], "snimek.png", { type: "image/png" });
    expect(await readAttachment(big)).toBe("Příloha je větší než 2 MB.");
  });
});
