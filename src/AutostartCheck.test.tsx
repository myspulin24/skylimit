import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AutostartStatus } from "./types";

vi.mock("./api", () => ({
  autostartStatus: vi.fn(),
  setAutostart: vi.fn(),
}));

import * as api from "./api";
import { AutostartCheck } from "./AutostartCheck";

const status = (patch: Partial<AutostartStatus>): AutostartStatus => ({ state: "off", problem: "", ...patch });
const LABEL = "Spouštět po přihlášení do počítače";

beforeEach(() => {
  vi.mocked(api.autostartStatus).mockReset();
  vi.mocked(api.setAutostart).mockReset();
});

describe("AutostartCheck", () => {
  it("shows the system's state and switches it at once", async () => {
    vi.mocked(api.autostartStatus).mockResolvedValue(status({}));
    vi.mocked(api.setAutostart).mockImplementation((on) => Promise.resolve(status({ state: on ? "on" : "off" })));
    const user = userEvent.setup();
    render(<AutostartCheck />);
    const box = screen.getByLabelText(LABEL);
    expect(box).toBeDisabled();
    expect(await screen.findByText(/Widget se objeví sám/)).toBeInTheDocument();
    expect(box).toBeEnabled();
    expect(box).not.toBeChecked();

    await user.click(box);
    expect(api.setAutostart).toHaveBeenCalledWith(true);
    expect(await screen.findByLabelText(LABEL)).toBeChecked();
    await user.click(box);
    expect(api.setAutostart).toHaveBeenLastCalledWith(false);
    expect(box).not.toBeChecked();
  });

  it("an entry for another copy is off here and turning on switches to this one", async () => {
    vi.mocked(api.autostartStatus).mockResolvedValue(status({ state: "moved" }));
    vi.mocked(api.setAutostart).mockResolvedValue(status({ state: "on" }));
    const user = userEvent.setup();
    render(<AutostartCheck />);
    expect(await screen.findByText(/spouští Skylimit z jiné složky/)).toBeInTheDocument();
    expect(screen.getByLabelText(LABEL)).not.toBeChecked();
    await user.click(screen.getByLabelText(LABEL));
    expect(api.setAutostart).toHaveBeenCalledWith(true);
    expect(screen.getByLabelText(LABEL)).toBeChecked();
  });

  it("explains why it cannot be turned on, and a failure keeps the old state", async () => {
    vi.mocked(api.autostartStatus).mockResolvedValueOnce(status({ problem: "Skylimit běží z dočasného místa." }));
    const { unmount } = render(<AutostartCheck />);
    expect(await screen.findByText("Skylimit běží z dočasného místa.")).toBeInTheDocument();
    expect(screen.getByLabelText(LABEL)).toBeDisabled();
    unmount();

    vi.mocked(api.autostartStatus).mockResolvedValueOnce(status({}));
    vi.mocked(api.setAutostart).mockRejectedValue("Spouštění po přihlášení se nepodařilo zapnout: Přístup byl odepřen.");
    const user = userEvent.setup();
    render(<AutostartCheck />);
    await screen.findByText(/Widget se objeví sám/);
    await user.click(screen.getByLabelText(LABEL));
    expect(await screen.findByRole("alert")).toHaveTextContent("Přístup byl odepřen");
    expect(screen.getByLabelText(LABEL)).not.toBeChecked();
  });
});
