import { describe, expect, it } from "vitest";

import { base64Size, fitDimensions, nextQuality, photoSrc, splitDataUrl } from "./photo";

describe("fitDimensions", () => {
  it("scales a landscape image by its width", () => {
    expect(fitDimensions(800, 400, 192)).toEqual({ width: 192, height: 96 });
  });

  it("scales a portrait image by its height", () => {
    expect(fitDimensions(400, 800, 192)).toEqual({ width: 96, height: 192 });
  });

  it("scales a square image to the max edge", () => {
    expect(fitDimensions(500, 500, 192)).toEqual({ width: 192, height: 192 });
  });

  it("never upscales a smaller image", () => {
    expect(fitDimensions(64, 48, 192)).toEqual({ width: 64, height: 48 });
  });

  it("rounds to whole pixels with a minimum of one", () => {
    expect(fitDimensions(1000, 3, 192)).toEqual({ width: 192, height: 1 });
  });

  it("falls back to a single pixel for zero input", () => {
    expect(fitDimensions(0, 100, 192)).toEqual({ width: 1, height: 1 });
  });

  it("falls back to a single pixel for NaN input", () => {
    expect(fitDimensions(Number.NaN, 100, 192)).toEqual({ width: 1, height: 1 });
  });
});

describe("base64Size", () => {
  it("counts unpadded base64", () => {
    expect(base64Size("AAAA")).toBe(3);
  });

  it("counts base64 with one pad character", () => {
    expect(base64Size("AAAAAAA=")).toBe(5);
  });

  it("counts base64 with two pad characters", () => {
    expect(base64Size("AAAAAA==")).toBe(4);
  });

  it("ignores whitespace", () => {
    expect(base64Size("AA AA\nAAAA")).toBe(6);
  });

  it("counts an empty string as nothing", () => {
    expect(base64Size("")).toBe(0);
  });
});

describe("splitDataUrl", () => {
  it("splits a base64 data url", () => {
    expect(splitDataUrl("data:image/jpeg;base64,AAAA")).toEqual({
      mediaType: "image/jpeg",
      data: "AAAA",
    });
  });

  it("returns null for a plain url", () => {
    expect(splitDataUrl("https://example.com/a.png")).toBeNull();
  });

  it("returns null for a data url that is not base64", () => {
    expect(splitDataUrl("data:image/svg+xml,<svg/>")).toBeNull();
  });
});

describe("photoSrc", () => {
  it("builds a data url", () => {
    expect(photoSrc({ mediaType: "image/png", data: "BBBB" })).toBe("data:image/png;base64,BBBB");
  });
});

describe("nextQuality", () => {
  it("steps down from the first quality", () => {
    expect(nextQuality(0.8)).toBe(0.65);
  });

  it("steps down to the third quality", () => {
    expect(nextQuality(0.65)).toBe(0.5);
  });

  it("steps down to the last quality", () => {
    expect(nextQuality(0.5)).toBe(0.35);
  });

  it("stops at the last quality", () => {
    expect(nextQuality(0.35)).toBeNull();
  });
});
