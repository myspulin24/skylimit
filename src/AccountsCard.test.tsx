import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Accounts } from "./types";

vi.mock("./api", () => ({
  getAccounts: vi.fn(),
}));

import * as api from "./api";
import { AccountsCard } from "./AccountsCard";

const now = Math.floor(Date.now() / 1000);

const both: Accounts = {
  claude: {
    email: "jana@example.com",
    name: "Jana Nováková",
    organization: "Studio",
    organizationType: "claude_team",
    role: "membership_admin",
    seatTier: "team_tier_1",
    rateLimitTier: "default_claude_max_5x",
    billing: "stripe_subscription",
    accountCreatedAt: 1_774_844_063,
    subscriptionCreatedAt: 1_774_727_612,
    extraUsage: true,
    extraUsageBlocked: "out_of_credits",
    profileFetchedAt: now - 3 * 3600,
    limitsFetchedAt: now - 240,
  },
  codex: {
    kind: "chatgpt",
    email: "jana@firma.cz",
    plan: "team",
    workspace: "Firma",
    cliVersion: "0.166.0",
    resetCredits: 1,
    credits: { hasCredits: false, unlimited: false, balance: null },
    lifetimeTokens: 1_284_000_000,
    readAt: now - 60,
  },
  codexNote: null,
};

const plain = (s: string | null) => (s ?? "").replace(/\s/g, " ");

beforeEach(() => {
  vi.mocked(api.getAccounts).mockReset();
});

describe("AccountsCard", () => {
  it("shows who is signed in to Claude and Codex, in plain Czech", async () => {
    vi.mocked(api.getAccounts).mockResolvedValue(both);
    render(<AccountsCard />);
    const claude = await screen.findByRole("article", { name: "Účet Claude" });
    expect(within(claude).getByText("jana@example.com")).toBeInTheDocument();
    expect(within(claude).getByText("Team")).toBeInTheDocument();
    expect(within(claude).getByText("Jana Nováková · Studio")).toBeInTheDocument();
    expect(within(claude).getByText("správce členů")).toBeInTheDocument();
    expect(within(claude).getByText("jako Max 5×")).toBeInTheDocument();
    expect(plain(within(claude).getByText(/^od /).textContent)).toBe("od 28. 3. 2026, přes web (Stripe)");
    expect(within(claude).getByText("zapnuté, kredit vyčerpaný")).toBeInTheDocument();
    expect(plain(within(claude).getByText(/^Z Claude Code/).textContent)).toBe("Z Claude Code · profil před 3 h · limity před 4 min");

    const codex = screen.getByRole("article", { name: "Účet Codex" });
    expect(within(codex).getByText("jana@firma.cz")).toBeInTheDocument();
    expect(within(codex).getByText("Team")).toBeInTheDocument();
    expect(within(codex).getByText("Firma")).toBeInTheDocument();
    expect(within(codex).getByText("účet ChatGPT")).toBeInTheDocument();
    expect(plain(within(codex).getByText(/k dispozici/).textContent)).toBe("1 reset k dispozici");
    expect(within(codex).getByText("žádné")).toBeInTheDocument();
    expect(plain(within(codex).getByText(/mld\./).textContent)).toBe("1,3 mld.");
    expect(plain(within(codex).getByText(/^Z Codex CLI/).textContent)).toBe("Z Codex CLI 0.166.0 · načteno před 1 min");
  });

  it("says what to do when nobody is signed in, and why Codex is missing", async () => {
    vi.mocked(api.getAccounts).mockResolvedValue({
      claude: null,
      codex: { ...both.codex!, kind: "none", email: null, plan: null, workspace: null },
      codexNote: null,
    });
    const { unmount } = render(<AccountsCard />);
    expect(await screen.findByText(/Claude Code tu není přihlášený/)).toBeInTheDocument();
    expect(screen.getByText(/codex login/)).toBeInTheDocument();
    unmount();

    vi.mocked(api.getAccounts).mockResolvedValue({ claude: null, codex: null, codexNote: "Codex CLI nenalezen" });
    render(<AccountsCard />);
    expect(await screen.findByText("Codex CLI nenalezen. Účet se ukáže, až Codex CLI odpoví.")).toBeInTheDocument();
  });

  it("shows a failure instead of empty panels", async () => {
    vi.mocked(api.getAccounts).mockRejectedValue("Aplikace se ukončuje.");
    render(<AccountsCard />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Aplikace se ukončuje.");
  });
});
