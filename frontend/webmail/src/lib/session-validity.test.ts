import { describe, expect, it } from "vitest";
import { JmapError } from "@irixmail/shared";

import { sessionStillValid } from "./session-validity";

describe("sessionStillValid", () => {
  it("discards the session on 401", () => {
    expect(sessionStillValid(new JmapError("unauthorized", undefined, 401))).toBe(false);
  });

  it("discards the session on 403", () => {
    expect(sessionStillValid(new JmapError("forbidden", undefined, 403))).toBe(false);
  });

  it("keeps the session when the network is unreachable", () => {
    expect(sessionStillValid(new TypeError("Failed to fetch"))).toBe(true);
  });

  it("keeps the session on server errors", () => {
    expect(sessionStillValid(new JmapError("boom", undefined, 503))).toBe(true);
  });

  it("keeps the session when the error carries no status", () => {
    expect(sessionStillValid(new JmapError("offline"))).toBe(true);
    expect(sessionStillValid(new Error("something else"))).toBe(true);
  });
});
