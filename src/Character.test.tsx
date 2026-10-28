import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Character } from "./Character";
import { installAnimationPause } from "./hooks";
import type { Mood } from "./types";

const MOODS: [Mood, string][] = [
  ["calm", "klid"],
  ["focus", "soustředění"],
  ["sweat", "pot"],
  ["panic", "panika"],
  ["celebrate", "oslava"],
  ["dozing", "dřímání"],
  ["sleep", "spánek"],
  ["confused", "zmatek"],
];

describe("Character", () => {
  it.each(MOODS)("Sunnie %s gets mood class and Czech label (%s)", (mood, label) => {
    render(<Character provider="claude" mood={mood} />);
    const svg = screen.getByRole("img", { name: `Sunnie: ${label}` });
    expect(svg).toHaveClass("mascot", "mascot-claude", `mood-${mood}`);
  });

  it.each(MOODS)("Dropie %s gets mood class and Czech label (%s)", (mood, label) => {
    render(<Character provider="codex" mood={mood} />);
    const svg = screen.getByRole("img", { name: `Dropie: ${label}` });
    expect(svg).toHaveClass("mascot", "mascot-codex", `mood-${mood}`);
  });

  it("uses no inline style attributes (CSP forbids them)", () => {
    const { container } = render(<Character provider="codex" mood="celebrate" />);
    expect(container.querySelector("[style]")).toBeNull();
  });

  it("gives two mascots of the same kind distinct gradient ids", () => {
    const { container } = render(
      <>
        <Character provider="claude" mood="calm" />
        <Character provider="claude" mood="sleep" />
      </>,
    );
    const ids = [...container.querySelectorAll("radialGradient")].map((g) => g.id);
    expect(new Set(ids).size).toBe(2);
  });
});

describe("animation pause", () => {
  let hidden = false;
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });

  afterEach(() => {
    hidden = false;
    document.documentElement.classList.remove("anim-paused");
  });

  it("adds anim-paused to <html> while the document is hidden", () => {
    const off = installAnimationPause();
    expect(document.documentElement).not.toHaveClass("anim-paused");
    hidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(document.documentElement).toHaveClass("anim-paused");
    hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(document.documentElement).not.toHaveClass("anim-paused");
    off();
  });
});
