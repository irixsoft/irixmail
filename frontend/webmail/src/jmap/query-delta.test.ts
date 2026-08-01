import { describe, expect, it } from "vitest";
import { applyQueryChanges, mergePage } from "./query-delta";

describe("applyQueryChanges", () => {
  it("removes then inserts added ids at their index", () => {
    const next = applyQueryChanges(["a", "b", "c"], {
      removed: ["b"],
      added: [{ id: "x", index: 0 }],
    });
    expect(next).toEqual(["x", "a", "c"]);
  });

  it("moves an id reported as both removed and added", () => {
    const next = applyQueryChanges(["a", "b", "c"], {
      removed: ["c"],
      added: [{ id: "c", index: 1 }],
    });
    expect(next).toEqual(["a", "c", "b"]);
  });

  it("clamps an out-of-range index to the tail", () => {
    const next = applyQueryChanges(["a"], {
      removed: [],
      added: [{ id: "z", index: 9 }],
    });
    expect(next).toEqual(["a", "z"]);
  });

  it("applies multiple additions in index order regardless of input order", () => {
    const next = applyQueryChanges(["a", "b"], {
      removed: [],
      added: [
        { id: "y", index: 2 },
        { id: "x", index: 0 },
      ],
    });
    expect(next).toEqual(["x", "a", "y", "b"]);
  });
});

describe("mergePage", () => {
  it("appends a fresh page at its position", () => {
    expect(mergePage(["a", "b"], ["c", "d"], 2)).toEqual(["a", "b", "c", "d"]);
  });

  it("overwrites overlap and drops duplicates from earlier pages", () => {
    expect(mergePage(["a", "b", "c"], ["b", "x"], 1)).toEqual(["a", "b", "x"]);
  });

  it("pads never — a gap position truncates to contiguous ids", () => {
    expect(mergePage(["a"], ["z"], 5)).toEqual(["a", "z"]);
  });
});
