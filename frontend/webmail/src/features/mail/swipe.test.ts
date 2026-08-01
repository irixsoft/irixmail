import { describe, expect, it } from "vitest";
import { SWIPE_THRESHOLD, resolveSwipe } from "./swipe";

describe("resolveSwipe", () => {
  it("archives on a long right swipe", () => {
    expect(resolveSwipe(SWIPE_THRESHOLD + 1)).toBe("archive");
  });

  it("opens the action menu on a long left swipe", () => {
    expect(resolveSwipe(-SWIPE_THRESHOLD - 1)).toBe("menu");
  });

  it("does nothing within the threshold", () => {
    expect(resolveSwipe(40)).toBeNull();
    expect(resolveSwipe(-40)).toBeNull();
  });
});
