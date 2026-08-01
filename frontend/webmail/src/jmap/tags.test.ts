import { beforeEach, describe, expect, it } from "vitest";
import {
  TAG_PALETTE,
  isTagKeyword,
  loadTagDefinitions,
  messageTagIds,
  saveTagDefinitions,
  tagIdFromKeyword,
  tagKeyword,
} from "./tags";

beforeEach(() => localStorage.clear());

describe("tag keywords", () => {
  it("round-trips an id through the keyword form", () => {
    expect(tagKeyword("work")).toBe("$label:work");
    expect(isTagKeyword("$label:work")).toBe(true);
    expect(isTagKeyword("$seen")).toBe(false);
    expect(tagIdFromKeyword("$label:work")).toBe("work");
  });

  it("extracts tag ids from a message keyword map", () => {
    expect(messageTagIds({ $seen: true, "$label:a": true, "$label:b": true })).toEqual(["a", "b"]);
  });
});

describe("tag definitions", () => {
  it("defaults to an empty list and round-trips through storage", () => {
    expect(loadTagDefinitions()).toEqual([]);
    saveTagDefinitions([{ id: "work", label: "Work", color: "amber" }]);
    expect(loadTagDefinitions()).toEqual([{ id: "work", label: "Work", color: "amber" }]);
  });

  it("drops malformed stored data", () => {
    localStorage.setItem("irixmail.webmail.tags", "{broken");
    expect(loadTagDefinitions()).toEqual([]);
  });
});

describe("palette", () => {
  it("names map to dot and background classes", () => {
    for (const entry of Object.values(TAG_PALETTE)) {
      expect(entry.dot).toMatch(/^bg-/);
    }
    expect(Object.keys(TAG_PALETTE).length).toBeGreaterThanOrEqual(8);
  });
});
