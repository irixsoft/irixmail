import { describe, expect, it } from "vitest";
import { applySelectionClick, emptySelection } from "./selection";

const order = ["a", "b", "c", "d", "e"];

describe("applySelectionClick", () => {
  it("toggle-selects with ctrl and keeps the anchor on the last toggle", () => {
    let state = applySelectionClick(emptySelection, order, "b", { toggle: true, range: false });
    expect([...state.selected]).toEqual(["b"]);
    state = applySelectionClick(state, order, "d", { toggle: true, range: false });
    expect([...state.selected].sort()).toEqual(["b", "d"]);
    expect(state.anchor).toBe("d");
    state = applySelectionClick(state, order, "b", { toggle: true, range: false });
    expect([...state.selected]).toEqual(["d"]);
  });

  it("selects a contiguous range from the anchor with shift", () => {
    let state = applySelectionClick(emptySelection, order, "b", { toggle: true, range: false });
    state = applySelectionClick(state, order, "d", { toggle: false, range: true });
    expect([...state.selected].sort()).toEqual(["b", "c", "d"]);
    expect(state.anchor).toBe("b");
  });

  it("range works upward too", () => {
    let state = applySelectionClick(emptySelection, order, "d", { toggle: true, range: false });
    state = applySelectionClick(state, order, "a", { toggle: false, range: true });
    expect([...state.selected].sort()).toEqual(["a", "b", "c", "d"]);
  });

  it("a plain click clears the selection", () => {
    let state = applySelectionClick(emptySelection, order, "b", { toggle: true, range: false });
    state = applySelectionClick(state, order, "c", { toggle: false, range: false });
    expect(state.selected.size).toBe(0);
  });
});
