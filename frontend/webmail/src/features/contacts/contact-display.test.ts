import { describe, expect, it } from "vitest";

import {
  contactInitials,
  displayName,
  filterContacts,
  formatBirthday,
  groupBySection,
  matchesContact,
  primaryEmail,
  sectionLetter,
  sortContacts,
} from "./contact-display";
import type { ContactCard } from "./types";

function card(partial: Partial<ContactCard>): ContactCard {
  return { id: "c1", addressBookId: "b1", ...partial };
}

describe("displayName", () => {
  it("prefers a non-empty fullName", () => {
    expect(displayName(card({ fullName: "Ada Lovelace", name: { given: "Ada" } }))).toBe("Ada Lovelace");
  });

  it("builds the name from its parts when fullName is blank", () => {
    expect(
      displayName(
        card({
          fullName: "  ",
          name: { prefix: "Dr", given: "Ada", additional: "Byron", surname: "Lovelace", suffix: "PhD" },
        }),
      ),
    ).toBe("Dr Ada Byron Lovelace PhD");
  });

  it("skips missing name parts", () => {
    expect(displayName(card({ name: { given: "Ada", surname: "Lovelace" } }))).toBe("Ada Lovelace");
  });

  it("falls back to the first email", () => {
    expect(displayName(card({ emails: [{ value: "ada@example.com", label: null }] }))).toBe(
      "ada@example.com",
    );
  });

  it("falls back to the organization before returning a placeholder", () => {
    expect(displayName(card({ organization: "Analytical Engines" }))).toBe("Analytical Engines");
    expect(displayName(card({}))).toBe("No name");
  });
});

describe("contactInitials", () => {
  it("takes the first letter of the first two words", () => {
    expect(contactInitials(card({ fullName: "Ada Lovelace" }))).toBe("AL");
  });

  it("takes a single letter for a one word name", () => {
    expect(contactInitials(card({ fullName: "Ada" }))).toBe("A");
  });

  it("uses the email initial when there is no name", () => {
    expect(contactInitials(card({ emails: [{ value: "grace@example.com", label: null }] }))).toBe("G");
  });

  it("returns a question mark for an unnamed card", () => {
    expect(contactInitials(card({ fullName: "  " }))).toBe("?");
  });

  it("ignores punctuation only words", () => {
    expect(contactInitials(card({ fullName: "Ada - Lovelace" }))).toBe("AL");
  });
});

describe("primaryEmail", () => {
  it("returns the first non-empty address", () => {
    expect(
      primaryEmail(
        card({
          emails: [
            { value: "  ", label: null },
            { value: "ada@example.com", label: "work" },
          ],
        }),
      ),
    ).toBe("ada@example.com");
  });

  it("returns an empty string when there is none", () => {
    expect(primaryEmail(card({ emails: null }))).toBe("");
  });
});

describe("sectionLetter", () => {
  it("uppercases the first letter of the display name", () => {
    expect(sectionLetter(card({ fullName: "ada lovelace" }))).toBe("A");
  });

  it("groups non-letters under a hash", () => {
    expect(sectionLetter(card({ fullName: "3M" }))).toBe("#");
    expect(sectionLetter(card({ fullName: "" }))).toBe("#");
  });

  it("folds accents onto the base letter", () => {
    expect(sectionLetter(card({ fullName: "Ángela" }))).toBe("A");
  });
});

describe("matchesContact", () => {
  const ada = card({
    fullName: "Ada Lovelace",
    organization: "Analytical Engines",
    emails: [{ value: "ada@example.com", label: "work" }],
  });

  it("matches an empty query", () => {
    expect(matchesContact(ada, "  ")).toBe(true);
  });

  it("matches the name case insensitively", () => {
    expect(matchesContact(ada, "LOVE")).toBe(true);
  });

  it("matches the organization and the email", () => {
    expect(matchesContact(ada, "engines")).toBe(true);
    expect(matchesContact(ada, "ada@ex")).toBe(true);
  });

  it("rejects a query that appears nowhere", () => {
    expect(matchesContact(ada, "babbage")).toBe(false);
  });
});

describe("sortContacts", () => {
  it("sorts by display name case insensitively", () => {
    const list = [card({ id: "b", fullName: "zoe" }), card({ id: "a", fullName: "Ada" })];
    expect(sortContacts(list).map((entry) => entry.id)).toEqual(["a", "b"]);
  });

  it("does not mutate the input", () => {
    const list = [card({ id: "b", fullName: "zoe" }), card({ id: "a", fullName: "Ada" })];
    sortContacts(list);
    expect(list[0]!.id).toBe("b");
  });
});

describe("filterContacts", () => {
  it("returns every card sorted when the query is blank", () => {
    const list = [card({ id: "b", fullName: "Zoe" }), card({ id: "a", fullName: "Ada" })];
    expect(filterContacts(list, "").map((entry) => entry.id)).toEqual(["a", "b"]);
  });

  it("keeps only the matching cards", () => {
    const list = [card({ id: "b", fullName: "Zoe" }), card({ id: "a", fullName: "Ada" })];
    expect(filterContacts(list, "zo").map((entry) => entry.id)).toEqual(["b"]);
  });
});

describe("groupBySection", () => {
  it("builds ordered sections keyed by letter", () => {
    const list = [
      card({ id: "a", fullName: "Ada" }),
      card({ id: "z", fullName: "Zoe" }),
      card({ id: "n", fullName: "9Lives" }),
      card({ id: "a2", fullName: "alan" }),
    ];
    expect(groupBySection(list).map((section) => [section.letter, section.contacts.length])).toEqual([
      ["A", 2],
      ["Z", 1],
      ["#", 1],
    ]);
  });

  it("returns nothing for an empty list", () => {
    expect(groupBySection([])).toEqual([]);
  });
});

describe("formatBirthday", () => {
  it("returns an em dash for a missing value", () => {
    expect(formatBirthday(null)).toBe("—");
    expect(formatBirthday("")).toBe("—");
  });

  it("keeps an unparseable value as written", () => {
    expect(formatBirthday("not-a-date")).toBe("not-a-date");
  });

  it("renders a full date", () => {
    expect(formatBirthday("1985-12-03")).toContain("1985");
  });
});
