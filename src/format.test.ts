import { describe, expect, it } from "vitest";
import {
  barTone,
  formatAgo,
  formatDays,
  formatDuration,
  formatMoney,
  formatMultiple,
  formatTokens,
  monthLabel,
} from "./format";
import { parseAmount } from "./SettingsForm";

// Intl and our helpers use (narrow) non-breaking spaces; compare as plain text.
const plain = (s: string) => s.replace(/\s/g, " ");

describe("format", () => {
  it("money in cs-CZ", () => {
    expect(plain(formatMoney(930, "EUR"))).toBe("930 €");
    expect(plain(formatMoney(23436, "CZK"))).toBe("23 436 Kč");
    expect(plain(formatMoney(89, "EUR"))).toBe("89 €");
    expect(plain(formatMoney(8.4, "EUR"))).toBe("8,40 €");
    expect(plain(formatMoney(0, "EUR"))).toBe("0 €");
  });

  it("durations", () => {
    expect(plain(formatDuration(100 * 60))).toBe("1 h 40 min");
    expect(plain(formatDuration(40 * 60))).toBe("40 min");
    expect(plain(formatDuration(3 * 3600))).toBe("3 h");
    expect(plain(formatDuration(4 * 86400 + 6 * 3600))).toBe("4 d 6 h");
  });

  it("ages", () => {
    expect(formatAgo(10)).toBe("právě teď");
    expect(plain(formatAgo(120))).toBe("před 2 min");
    expect(plain(formatAgo(3 * 3600))).toBe("před 3 h");
  });

  it("bar tone thresholds match the tray: <75 green, ≥75 amber, ≥95 red", () => {
    expect(barTone(74.9)).toBe("ok");
    expect(barTone(75)).toBe("warn");
    expect(barTone(94.9)).toBe("warn");
    expect(barTone(95)).toBe("crit");
  });

  it("multiples, tokens, days, month", () => {
    expect(formatMultiple(4.594)).toBe("4,6×");
    expect(plain(formatTokens(1_412_300_000))).toBe("1,4 mld.");
    expect(plain(formatTokens(61_250_000))).toBe("61,3 mil.");
    expect(plain(formatDays(1))).toBe("1 den");
    expect(plain(formatDays(3))).toBe("3 dny");
    expect(plain(formatDays(12))).toBe("12 dní");
    expect(monthLabel("2026-10")).toBe("říjen 2026");
  });

  it("parses Czech amounts", () => {
    expect(parseAmount("250,5")).toBe(250.5);
    expect(parseAmount("1 234")).toBe(1234);
    expect(parseAmount("0")).toBeNull();
    expect(parseAmount("abc")).toBeNull();
    expect(parseAmount("")).toBeNull();
  });
});
