import { describe, expect, it } from "vitest";

import type { ContactCard } from "@/features/contacts/types";
import { nextHighlight, rankSuggestions } from "./suggestions";

function card(partial: Partial<ContactCard> & { id: string }): ContactCard {
  return { addressBookId: "ab1", kind: "individual", ...partial };
}

const ada = card({
  id: "1",
  fullName: "Ada Lovelace",
  organization: "Analytical Engines",
  emails: [
    { value: "ada@example.com", label: "work" },
    { value: "lovelace@other.test", label: "home" },
  ],
});

const grace = card({
  id: "2",
  name: { given: "Grace", surname: "Hopper" },
  organization: "Navy",
  emails: [{ value: "grace@navy.test", label: null }],
});

const alan = card({
  id: "3",
  fullName: "Alan Turing",
  emails: [{ value: "turing@bletchley.test", label: null }],
});

describe("rankSuggestions", () => {
  it("returns nothing for an empty or whitespace query", () => {
    expect(rankSuggestions([ada, grace], "")).toEqual([]);
    expect(rankSuggestions([ada, grace], "   ")).toEqual([]);
  });

  it("matches the display name case-insensitively", () => {
    const result = rankSuggestions([ada, grace], "ADA");
    expect(result.map((entry) => entry.email)).toEqual([
      "ada@example.com",
      "lovelace@other.test",
    ]);
  });

  it("emits one entry per matching email of an individual", () => {
    const result = rankSuggestions([ada], "Ada Lovelace");
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({
      id: expect.any(String),
      kind: "individual",
      name: "Ada Lovelace",
      email: "ada@example.com",
      memberCount: 0,
      addresses: [{ name: "Ada Lovelace", email: "ada@example.com" }],
    });
  });

  it("builds the display name from name parts when fullName is absent", () => {
    const result = rankSuggestions([grace], "hopper");
    expect(result[0]?.name).toBe("Grace Hopper");
    expect(result[0]?.addresses).toEqual([{ name: "Grace Hopper", email: "grace@navy.test" }]);
  });

  it("falls back to the first email when there is no name", () => {
    const nameless = card({ id: "9", emails: [{ value: "ghost@example.com", label: null }] });
    const result = rankSuggestions([nameless], "ghost");
    expect(result[0]?.name).toBe("ghost@example.com");
  });

  it("ignores blank name parts when joining", () => {
    const spaced = card({
      id: "9",
      name: { prefix: "  ", given: "Ada", additional: null, surname: "Byron", suffix: "" },
      emails: [{ value: "byron@example.com", label: null }],
    });
    expect(rankSuggestions([spaced], "byron")[0]?.name).toBe("Ada Byron");
  });

  it("skips cards without any email", () => {
    const empty = card({ id: "9", fullName: "Ada Nobody", emails: [] });
    expect(rankSuggestions([empty], "ada")).toEqual([]);
  });

  it("matches on the organization", () => {
    const result = rankSuggestions([ada, grace], "navy");
    expect(result.map((entry) => entry.name)).toEqual(["Grace Hopper"]);
  });

  it("matches on an email value", () => {
    const result = rankSuggestions([ada, alan], "bletchley");
    expect(result.map((entry) => entry.email)).toEqual(["turing@bletchley.test"]);
  });

  it("ranks a name prefix above an email prefix", () => {
    const nameMatch = card({
      id: "a",
      fullName: "Turing Institute",
      emails: [{ value: "hello@institute.test", label: null }],
    });
    const result = rankSuggestions([alan, nameMatch], "turing");
    expect(result.map((entry) => entry.name)).toEqual(["Turing Institute", "Alan Turing"]);
  });

  it("ranks an email prefix above an inner word prefix", () => {
    const emailPrefix = card({
      id: "a",
      fullName: "Zed Zebra",
      emails: [{ value: "hop@z.test", label: null }],
    });
    const result = rankSuggestions([emailPrefix, grace], "hop");
    expect(result.map((entry) => entry.name)).toEqual(["Zed Zebra", "Grace Hopper"]);
  });

  it("ranks an inner word prefix above a bare contains match", () => {
    const contains = card({
      id: "a",
      fullName: "Machopper Ltd",
      emails: [{ value: "info@macho.test", label: null }],
    });
    const result = rankSuggestions([contains, grace], "hopp");
    expect(result.map((entry) => entry.name)).toEqual(["Grace Hopper", "Machopper Ltd"]);
  });

  it("breaks ties by display name then email", () => {
    const b = card({ id: "b", fullName: "Team Beta", emails: [{ value: "b@t.test", label: null }] });
    const a = card({
      id: "a",
      fullName: "Team Alpha",
      emails: [
        { value: "z@t.test", label: null },
        { value: "a@t.test", label: null },
      ],
    });
    const result = rankSuggestions([b, a], "team");
    expect(result.map((entry) => entry.email)).toEqual(["a@t.test", "z@t.test", "b@t.test"]);
  });

  it("drops individual suggestions whose email is excluded", () => {
    const result = rankSuggestions([ada], "ada", ["  ADA@EXAMPLE.COM "]);
    expect(result.map((entry) => entry.email)).toEqual(["lovelace@other.test"]);
  });

  it("caps the result at the default limit of eight", () => {
    const many = Array.from({ length: 12 }, (_, index) =>
      card({
        id: `c${index}`,
        fullName: `Person ${index}`,
        emails: [{ value: `p${index}@x.test`, label: null }],
      }),
    );
    expect(rankSuggestions(many, "person")).toHaveLength(8);
    expect(rankSuggestions(many, "person", [], 3)).toHaveLength(3);
  });

  it("collapses a group into a single suggestion with member addresses", () => {
    const group = card({ id: "g", kind: "group", fullName: "Crew", members: ["1", "2"] });
    const result = rankSuggestions([ada, grace, group], "crew");
    expect(result).toHaveLength(1);
    expect(result[0]).toEqual({
      id: expect.any(String),
      kind: "group",
      name: "Crew",
      email: "",
      memberCount: 2,
      addresses: [
        { name: "Ada Lovelace", email: "ada@example.com" },
        { name: "Grace Hopper", email: "grace@navy.test" },
      ],
    });
  });

  it("ignores group members that are missing or have no email", () => {
    const noEmail = card({ id: "4", fullName: "Empty Person", emails: [] });
    const group = card({
      id: "g",
      kind: "group",
      fullName: "Crew",
      members: ["1", "4", "missing"],
    });
    const result = rankSuggestions([ada, noEmail, group], "crew");
    expect(result[0]?.memberCount).toBe(1);
    expect(result[0]?.addresses).toEqual([{ name: "Ada Lovelace", email: "ada@example.com" }]);
  });

  it("dedupes group addresses case-insensitively keeping the first", () => {
    const dupe = card({
      id: "5",
      fullName: "Ada Clone",
      emails: [{ value: "ADA@example.com", label: null }],
    });
    const group = card({ id: "g", kind: "group", fullName: "Crew", members: ["1", "5"] });
    const result = rankSuggestions([ada, dupe, group], "crew");
    expect(result[0]?.addresses).toEqual([{ name: "Ada Lovelace", email: "ada@example.com" }]);
    expect(result[0]?.memberCount).toBe(2);
  });

  it("does not match a group on its member emails", () => {
    const group = card({ id: "g", kind: "group", fullName: "Crew", members: ["1"] });
    expect(rankSuggestions([ada, group], "example.com").every((e) => e.kind === "individual")).toBe(
      true,
    );
  });

  it("drops a group with no resolvable addresses", () => {
    const group = card({ id: "g", kind: "group", fullName: "Crew", members: ["nope"] });
    expect(rankSuggestions([group], "crew")).toEqual([]);
  });

  it("filters excluded addresses out of a group", () => {
    const group = card({ id: "g", kind: "group", fullName: "Crew", members: ["1", "2"] });
    const result = rankSuggestions([ada, grace, group], "crew", ["grace@NAVY.test"]);
    expect(result[0]?.addresses).toEqual([{ name: "Ada Lovelace", email: "ada@example.com" }]);
  });

  it("drops a group whose every address is excluded", () => {
    const group = card({ id: "g", kind: "group", fullName: "Crew", members: ["1"] });
    expect(rankSuggestions([ada, group], "crew", ["ada@example.com"])).toEqual([]);
  });
});

describe("nextHighlight", () => {
  it("stays at -1 when there is nothing to highlight", () => {
    expect(nextHighlight(-1, 1, 0)).toBe(-1);
    expect(nextHighlight(0, -1, 0)).toBe(-1);
  });

  it("moves from -1 to the first item going down", () => {
    expect(nextHighlight(-1, 1, 3)).toBe(0);
  });

  it("moves from -1 to the last item going up", () => {
    expect(nextHighlight(-1, -1, 3)).toBe(2);
  });

  it("wraps past the last item to the first", () => {
    expect(nextHighlight(2, 1, 3)).toBe(0);
  });

  it("wraps before the first item to the last", () => {
    expect(nextHighlight(0, -1, 3)).toBe(2);
  });

  it("normalises an out-of-range current index", () => {
    expect(nextHighlight(9, 1, 3)).toBe(0);
  });
});
