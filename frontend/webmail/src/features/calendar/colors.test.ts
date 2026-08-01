import { describe, expect, it } from "vitest";

import { CALENDAR_PALETTE, calendarColor, normalizeHex } from "./colors";

describe("CALENDAR_PALETTE", () => {
  it("offers a set of distinct hex swatches", () => {
    expect(CALENDAR_PALETTE.length).toBeGreaterThanOrEqual(6);
    const hexes = new Set(CALENDAR_PALETTE.map((swatch) => swatch.hex));
    expect(hexes.size).toBe(CALENDAR_PALETTE.length);
    for (const swatch of CALENDAR_PALETTE) {
      expect(swatch.hex).toMatch(/^#[0-9a-f]{6}$/);
      expect(swatch.label.length).toBeGreaterThan(0);
    }
  });
});

describe("normalizeHex", () => {
  it("accepts and lowercases valid hex", () => {
    expect(normalizeHex("#AABBCC")).toBe("#aabbcc");
    expect(normalizeHex("#abc")).toBe("#aabbcc");
  });

  it("rejects anything else", () => {
    expect(normalizeHex(null)).toBeNull();
    expect(normalizeHex("rebeccapurple")).toBeNull();
    expect(normalizeHex("#12345")).toBeNull();
  });
});

describe("calendarColor", () => {
  it("uses the stored colour when it is valid", () => {
    expect(calendarColor({ id: "c1", color: "#123456" })).toBe("#123456");
  });

  it("falls back to a stable palette entry", () => {
    const first = calendarColor({ id: "c1", color: null });
    expect(calendarColor({ id: "c1", color: "not a colour" })).toBe(first);
    expect(CALENDAR_PALETTE.some((swatch) => swatch.hex === first)).toBe(true);
    expect(calendarColor({ id: "", color: null })).toBe(CALENDAR_PALETTE[0]!.hex);
  });
});
