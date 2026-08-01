import { describe, expect, it } from "vitest";

import {
  dedupeRecipients,
  invalidRecipients,
  isValidEmail,
  parseRecipient,
  parseRecipients,
  recipientInitial,
  recipientLabel,
} from "./recipients";

describe("parseRecipient", () => {
  it("returns null for blank entries", () => {
    expect(parseRecipient("   ")).toBeNull();
    expect(parseRecipient(",")).toBeNull();
  });

  it("reads a bare address", () => {
    expect(parseRecipient("  ada@example.com ")).toEqual({ email: "ada@example.com" });
  });

  it("reads a named address", () => {
    expect(parseRecipient('"Ada Lovelace" <ada@example.com>')).toEqual({
      name: "Ada Lovelace",
      email: "ada@example.com",
    });
    expect(parseRecipient("Ada <ada@example.com>")).toEqual({
      name: "Ada",
      email: "ada@example.com",
    });
  });

  it("drops an empty display name", () => {
    expect(parseRecipient("<ada@example.com>")).toEqual({ email: "ada@example.com" });
  });
});

describe("parseRecipients", () => {
  it("splits on commas, semicolons and newlines", () => {
    expect(parseRecipients("a@x.com, b@x.com;\nc@x.com")).toEqual([
      { email: "a@x.com" },
      { email: "b@x.com" },
      { email: "c@x.com" },
    ]);
  });

  it("keeps entries that are not valid addresses", () => {
    expect(parseRecipients("nope")).toEqual([{ email: "nope" }]);
  });
});

describe("isValidEmail", () => {
  it("accepts an address with a dotted domain", () => {
    expect(isValidEmail("ada@example.com")).toBe(true);
  });

  it("rejects incomplete addresses", () => {
    expect(isValidEmail("ada@example")).toBe(false);
    expect(isValidEmail("ada")).toBe(false);
    expect(isValidEmail("a b@example.com")).toBe(false);
  });
});

describe("dedupeRecipients", () => {
  it("keeps the first entry per address, case insensitively", () => {
    expect(
      dedupeRecipients([
        { name: "Ada", email: "ada@example.com" },
        { email: "ADA@example.com" },
        { email: "grace@example.com" },
      ]),
    ).toEqual([{ name: "Ada", email: "ada@example.com" }, { email: "grace@example.com" }]);
  });
});

describe("invalidRecipients", () => {
  it("returns only the malformed entries", () => {
    expect(invalidRecipients([{ email: "ada@example.com" }, { email: "oops" }])).toEqual([
      { email: "oops" },
    ]);
  });
});

describe("recipientLabel", () => {
  it("prefers the display name and falls back to the address", () => {
    expect(recipientLabel({ name: "Ada", email: "ada@example.com" })).toBe("Ada");
    expect(recipientLabel({ name: "  ", email: "ada@example.com" })).toBe("ada@example.com");
    expect(recipientInitial({ email: "ada@example.com" })).toBe("A");
  });
});
