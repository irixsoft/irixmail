import { describe, expect, it } from "vitest";
import { flaggedPatch, movePatch, seenPatch, tagPatch, updateMap } from "./mutations";

describe("patch builders", () => {
  it("builds seen and flagged keyword patches", () => {
    expect(seenPatch(true)).toEqual({ "keywords/$seen": true });
    expect(seenPatch(false)).toEqual({ "keywords/$seen": null });
    expect(flaggedPatch(true)).toEqual({ "keywords/$flagged": true });
  });

  it("builds a move patch that replaces all mailboxes", () => {
    expect(movePatch("target")).toEqual({ mailboxIds: { target: true } });
  });

  it("builds tag apply and remove patches", () => {
    expect(tagPatch("work", true)).toEqual({ "keywords/$label:work": true });
    expect(tagPatch("work", false)).toEqual({ "keywords/$label:work": null });
  });

  it("expands a patch over many ids", () => {
    expect(updateMap(["1", "2"], seenPatch(true))).toEqual({
      "1": { "keywords/$seen": true },
      "2": { "keywords/$seen": true },
    });
  });
});
