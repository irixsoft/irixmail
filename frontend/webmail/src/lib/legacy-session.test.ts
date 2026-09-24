import { describe, expect, it } from "vitest";
import { LEGACY_STORAGE_KEY, dropLegacySession } from "@irixmail/shared";

describe("dropLegacySession", () => {
  it("removes the key both apps used to share", () => {
    const removed: string[] = [];
    dropLegacySession({ removeItem: (key) => removed.push(key) });
    expect(removed).toEqual([LEGACY_STORAGE_KEY]);
    expect(LEGACY_STORAGE_KEY).toBe("irixmail.auth");
  });

  it("tolerates unavailable storage", () => {
    expect(() =>
      dropLegacySession({
        removeItem: () => {
          throw new Error("blocked");
        },
      }),
    ).not.toThrow();
  });
});
