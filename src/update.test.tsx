import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UpdateStatus } from "./types";

vi.mock("./api", () => ({
  isTauri: () => false,
  updateStatus: vi.fn(),
  checkForUpdate: vi.fn(),
  installUpdate: vi.fn(() => Promise.resolve()),
  onUpdate: vi.fn(() => () => {}),
}));

import * as api from "./api";
import { ReleaseNotes, UpdateBanner, UpdateLine, hasOffer, useUpdate } from "./update";

const base: UpdateStatus = {
  phase: "upToDate",
  current: "0.6.0",
  version: null,
  notes: null,
  date: null,
  progress: null,
  error: null,
  checkedAt: null,
  auto: true,
};
const offer: UpdateStatus = {
  ...base,
  phase: "available",
  version: "0.7.0",
  notes: "### Přidáno\n\n- **Něco nového.** Popis, který\n  pokračuje na dalším řádku.\n- Druhá věc.\n\n---\n\nStáhni si instalátor níž.",
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.onUpdate).mockImplementation(() => () => {});
});

describe("ReleaseNotes", () => {
  it("renders the CHANGELOG headings, bullets and bold, without the download hint", () => {
    render(<ReleaseNotes notes={offer.notes!} />);
    expect(screen.getByRole("heading", { name: "Přidáno" })).toBeInTheDocument();
    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Něco nového. Popis, který pokračuje na dalším řádku.");
    expect(within(items[0]!).getByText("Něco nového.").tagName).toBe("B");
    expect(screen.queryByText(/Stáhni si instalátor/)).not.toBeInTheDocument();
  });
});

describe("UpdateBanner", () => {
  it("stays hidden without an offer", () => {
    const { container } = render(<UpdateBanner status={base} />);
    expect(container).toBeEmptyDOMElement();
    expect(hasOffer(base)).toBe(false);
    expect(hasOffer(null)).toBe(false);
  });

  it("offers the new version, shows what is new and installs on click", () => {
    render(<UpdateBanner status={offer} />);
    const banner = screen.getByRole("status", { name: "Aktualizace" });
    expect(banner).toHaveTextContent("Je k dispozici Skylimit 0.7.0. Teď máš 0.6.0.");
    fireEvent.click(within(banner).getByRole("button", { name: "Co je nového" }));
    expect(within(banner).getByRole("heading", { name: "Přidáno" })).toBeInTheDocument();
    fireEvent.click(within(banner).getByRole("button", { name: "Aktualizovat a restartovat" }));
    expect(api.installUpdate).toHaveBeenCalledTimes(1);
  });

  it("shows progress while downloading and the reason when it failed", () => {
    const { rerender } = render(<UpdateBanner status={{ ...offer, phase: "downloading", progress: 0.42 }} />);
    expect(screen.getByRole("status", { name: "Aktualizace" })).toHaveTextContent("Stahuji Skylimit 0.7.0… 42 %");
    expect(screen.getByRole("button", { name: "Aktualizovat a restartovat" })).toBeDisabled();
    rerender(<UpdateBanner status={{ ...offer, phase: "error", error: "Aktualizaci se nepodařilo stáhnout. Zkus to prosím znovu." }} />);
    expect(screen.getByText("Aktualizaci se nepodařilo stáhnout. Zkus to prosím znovu.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Aktualizovat a restartovat" })).toBeEnabled();
  });
});

describe("UpdateLine", () => {
  it("says the version is current and checks on demand", async () => {
    vi.mocked(api.checkForUpdate).mockResolvedValue(base);
    render(<UpdateLine status={{ ...base, checkedAt: Math.floor(Date.now() / 1000) - 600 }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Máš nejnovější verzi (kontrola před 10 min).");
    fireEvent.click(screen.getByRole("button", { name: "Zkontrolovat aktualizace" }));
    expect(api.checkForUpdate).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByRole("button", { name: "Zkontrolovat aktualizace" })).toBeEnabled());
  });

  it("tells when automatic checks are off and when a check failed", () => {
    const { rerender } = render(<UpdateLine status={{ ...base, phase: "idle", auto: false }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Automatická kontrola je vypnutá.");
    rerender(<UpdateLine status={{ ...base, phase: "error", error: "Aktualizaci se nepodařilo zkontrolovat. Jsi připojený k internetu?" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Jsi připojený k internetu?");
  });
});

describe("useUpdate", () => {
  it("starts from the shell's status and follows the update event", async () => {
    let push: (s: UpdateStatus) => void = () => {};
    vi.mocked(api.updateStatus).mockResolvedValue(base);
    vi.mocked(api.onUpdate).mockImplementation((cb) => {
      push = cb;
      return () => {};
    });
    function Probe() {
      const s = useUpdate();
      return <p>{s ? `${s.phase} ${s.version ?? ""}` : "nic"}</p>;
    }
    render(<Probe />);
    expect(await screen.findByText("upToDate")).toBeInTheDocument();
    act(() => push(offer));
    expect(screen.getByText("available 0.7.0")).toBeInTheDocument();
  });
});
