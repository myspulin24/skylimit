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
  updateStatus: vi.fn(() => Promise.resolve({ phase: "upToDate", current: "0.6.0", version: null, notes: null, date: null, progress: null, error: null, checkedAt: null, auto: true })),
  checkForUpdate: vi.fn(() => Promise.resolve({ phase: "upToDate", current: "0.6.0", version: null, notes: null, date: null, progress: null, error: null, checkedAt: null, auto: true })),
  installUpdate: vi.fn(() => Promise.resolve()),
  onUpdate: vi.fn(() => () => {}),
  feedbackStatus: vi.fn(),
  sendFeedback: vi.fn(),
  onOpenFeedback: vi.fn(() => () => {}),
  claudeCodeStatus: vi.fn(() => Promise.resolve({ state: "off", settingsPath: "~/.claude/settings.json", ownStatusLine: false, lastReadingAt: null, problem: "" })),
  connectClaudeCode: vi.fn(),
  disconnectClaudeCode: vi.fn(),
}));

import * as api from "./api";
import { AppWindow } from "./AppWindow";
import { fitImage, pastedImageFile, readAttachment } from "./FeedbackDialog";

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
  // Sending is enabled once the shell said where it goes (not shown any more).
  await waitFor(() => expect(within(dialog).getByRole("button", { name: "Odeslat" })).toBeEnabled());
  return { user, dialog };
}

describe("Zpětná vazba", () => {
  it("talks to people, not about servers, and sends with the tab as location", async () => {
    vi.mocked(api.sendFeedback).mockResolvedValue({ id: "abcd1234" });
    const { user, dialog } = await openDialog();
    expect(within(dialog).getByText("Napiš, co nefunguje nebo co by šlo udělat líp. Každou zprávu si přečtu.")).toBeInTheDocument();
    expect(dialog).not.toHaveTextContent(/workers\.dev|repozitář|x86_64/);

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

describe("Příloha ze schránky", () => {
  const paste = (target: HTMLElement, blob: Blob | null, text?: string) =>
    fireEvent.paste(target, {
      clipboardData: {
        items: blob ? [{ kind: "file", type: blob.type, getAsFile: () => blob }] : [],
        getData: () => text ?? "",
      },
    });

  it("a pasted screenshot becomes the attachment and is sent", async () => {
    vi.mocked(api.sendFeedback).mockResolvedValue({ id: "pic1" });
    const { user, dialog } = await openDialog();
    const message = within(dialog).getByRole("textbox", { name: "Zpráva" });
    await user.type(message, "Codex se nepropisuje");
    paste(message, new Blob(["png-bytes"], { type: "image/png" }));
    expect(await within(dialog).findByText(/^snimek-\d{4}-\d{2}-\d{2}-\d{6}\.png$/)).toBeInTheDocument();
    expect(within(dialog).getByRole("img", { name: "Náhled přílohy" })).toBeInTheDocument();
    expect(message).toHaveValue("Codex se nepropisuje");

    await user.click(within(dialog).getByRole("button", { name: "Odeslat" }));
    await waitFor(() => expect(api.sendFeedback).toHaveBeenCalledTimes(1));
    const sent = vi.mocked(api.sendFeedback).mock.calls[0]?.[0];
    expect(sent?.attachment).toMatchObject({ type: "image/png", data: btoa("png-bytes") });
    expect(sent?.attachment?.name).toMatch(/^snimek-.*\.png$/);
  });

  it("text pastes as usual and the attachment can be removed", async () => {
    const { dialog } = await openDialog();
    const message = within(dialog).getByRole("textbox", { name: "Zpráva" });
    expect(paste(message, null, "jen text")).toBe(true);
    paste(message, new Blob(["x"], { type: "image/jpeg" }));
    fireEvent.click(await within(dialog).findByRole("button", { name: "Odebrat" }));
    expect(within(dialog).queryByRole("img", { name: "Náhled přílohy" })).toBeNull();
  });

  it("names clipboard images and leaves small or undrawable ones as they are", async () => {
    const at = new Date(2026, 9, 29, 13, 5, 9);
    expect(pastedImageFile(new Blob(["a"], { type: "image/png" }), at)?.name).toBe("snimek-2026-10-29-130509.png");
    expect(pastedImageFile(new Blob(["a"], { type: "image/jpeg" }), at)?.name).toBe("snimek-2026-10-29-130509.jpg");
    expect(pastedImageFile(new Blob(["a"], { type: "image/tiff" }), at)).toBeNull();
    const small = new File(["a"], "a.png", { type: "image/png" });
    expect(await fitImage(small)).toBe(small);
    // jsdom cannot draw; the size check in readAttachment then explains it.
    const big = new File([new Uint8Array(2 * 1024 * 1024 + 1)], "big.png", { type: "image/png" });
    expect(await fitImage(big)).toBe(big);
  });
});

describe("O aplikaci a copyright", () => {
  it("settings show version and copyright and open the feedback dialog", async () => {
    const user = userEvent.setup();
    render(<AppWindow />);
    expect(screen.getByRole("contentinfo")).toHaveTextContent(`Skylimit ${__APP_VERSION__} · © 2026 Michal Jašek · licence MIT`);
    await user.click(await screen.findByRole("tab", { name: "Nastavení" }));
    const about = (await screen.findByRole("heading", { name: "O aplikaci" })).closest("section") as HTMLElement;
    expect(within(about).getByText(__APP_VERSION__)).toBeInTheDocument();
    expect(within(about).getByText(/© 2026 Michal Jašek/)).toBeInTheDocument();
    await user.click(within(about).getByRole("button", { name: "Poslat zpětnou vazbu" }));
    expect(await screen.findByRole("dialog", { name: "Zpětná vazba" })).toBeInTheDocument();
  });

  it("the version comes from package.json", async () => {
    const { version } = (await import("../package.json")).default as { version: string };
    expect(__APP_VERSION__).toBe(version);
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
