import { describe, expect, it } from "vitest";
import { buildEmailFilter, countActiveFilters, emptyFilters } from "./search-filter";

describe("buildEmailFilter", () => {
  it("returns the mailbox condition alone for a plain view", () => {
    expect(buildEmailFilter({ ...emptyFilters, mailboxId: "m1" })).toEqual({ inMailbox: "m1" });
  });

  it("returns an empty object when nothing is set", () => {
    expect(buildEmailFilter(emptyFilters)).toEqual({});
  });

  it("emits a single condition bare without an operator", () => {
    expect(buildEmailFilter({ ...emptyFilters, text: "invoice" })).toEqual({ text: "invoice" });
  });

  it("combines conditions under AND", () => {
    const filter = buildEmailFilter({
      ...emptyFilters,
      text: "report",
      from: "amelia",
      hasAttachment: true,
      unread: true,
      starred: false,
      mailboxId: "m1",
    });
    expect(filter).toEqual({
      operator: "AND",
      conditions: [
        { inMailbox: "m1" },
        { text: "report" },
        { from: "amelia" },
        { hasAttachment: true },
        { notKeyword: "$seen" },
        { notKeyword: "$flagged" },
      ],
    });
  });

  it("maps read tri-state and date bounds", () => {
    const filter = buildEmailFilter({
      ...emptyFilters,
      unread: false,
      after: "2026-01-01",
      before: "2026-02-01",
    });
    expect(filter).toEqual({
      operator: "AND",
      conditions: [
        { hasKeyword: "$seen" },
        { after: "2026-01-01T00:00:00Z" },
        { before: "2026-02-01T23:59:59Z" },
      ],
    });
  });

  it("filters by tag keyword", () => {
    expect(buildEmailFilter({ ...emptyFilters, tag: "$label:work" })).toEqual({
      hasKeyword: "$label:work",
    });
  });
});

describe("countActiveFilters", () => {
  it("ignores text and mailbox but counts the rest", () => {
    expect(countActiveFilters({ ...emptyFilters, text: "x", mailboxId: "m" })).toBe(0);
    expect(
      countActiveFilters({ ...emptyFilters, from: "a", unread: true, hasAttachment: false }),
    ).toBe(3);
  });
});
