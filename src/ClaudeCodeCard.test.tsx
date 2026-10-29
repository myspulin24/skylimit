import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClaudeCodeStatus } from "./types";

vi.mock("./api", () => ({
  claudeCodeStatus: vi.fn(),
  connectClaudeCode: vi.fn(),
  disconnectClaudeCode: vi.fn(),
}));

import * as api from "./api";
import { ClaudeCodeCard } from "./ClaudeCodeCard";

const status = (patch: Partial<ClaudeCodeStatus>): ClaudeCodeStatus => ({
  state: "off",
  settingsPath: "/Users/lumir/.claude/settings.json",
  ownStatusLine: false,
  lastReadingAt: null,
  problem: "",
  ...patch,
});

beforeEach(() => {
  vi.mocked(api.claudeCodeStatus).mockReset();
  vi.mocked(api.connectClaudeCode).mockReset();
  vi.mocked(api.disconnectClaudeCode).mockReset();
});

describe("ClaudeCodeCard", () => {
  it("connects and then shows when limits last came", async () => {
    vi.mocked(api.claudeCodeStatus).mockResolvedValue(status({}));
    vi.mocked(api.connectClaudeCode).mockResolvedValue(status({ state: "on", lastReadingAt: Date.now() / 1000 - 180 }));
    render(<ClaudeCodeCard />);
    expect(await screen.findByRole("status")).toHaveTextContent("Nepřipojeno.");
    expect(screen.getByText(/\/Users\/lumir\/\.claude\/settings\.json/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Připojit" }));
    expect(api.connectClaudeCode).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("status")).toHaveTextContent("Připojeno, poslední limity před 3 min.");
    expect(screen.getByRole("button", { name: "Odpojit" })).toBeInTheDocument();
  });

  it("tells that the user's own status line keeps running", async () => {
    vi.mocked(api.claudeCodeStatus).mockResolvedValue(status({ state: "on", ownStatusLine: true }));
    render(<ClaudeCodeCard />);
    expect(await screen.findByRole("status")).toHaveTextContent("Limity přijdou s další odpovědí Claude Code.");
    expect(screen.getByText(/Tvůj vlastní stavový řádek poběží dál/)).toBeInTheDocument();
  });

  it("shows a refusal from the shell and keeps the button usable", async () => {
    vi.mocked(api.claudeCodeStatus).mockResolvedValue(status({}));
    vi.mocked(api.connectClaudeCode).mockRejectedValue("settings.json není platný JSON, Skylimit ho proto neupraví.");
    render(<ClaudeCodeCard />);
    await userEvent.click(await screen.findByRole("button", { name: "Připojit" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("není platný JSON");
    expect(screen.getByRole("button", { name: "Připojit" })).toBeEnabled();
  });

  it("offers nothing to click without Claude Code or from a temporary location", async () => {
    vi.mocked(api.claudeCodeStatus).mockResolvedValueOnce(status({ state: "missing" }));
    const { unmount } = render(<ClaudeCodeCard />);
    expect(await screen.findByText(/Claude Code tu není/)).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
    unmount();

    vi.mocked(api.claudeCodeStatus).mockResolvedValueOnce(status({ problem: "Skylimit běží z dočasného umístění." }));
    render(<ClaudeCodeCard />);
    expect(await screen.findByText("Skylimit běží z dočasného umístění.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Připojit" })).toBeDisabled();
  });
});
