import { describe, expect, it } from "vitest";

import { planImport } from "./import-plan";
import type { ContactCard } from "./types";

const existing: ContactCard[] = [
  {
    id: "1",
    addressBookId: "b1",
    uid: "urn-ada",
    fullName: "Ada Lovelace",
    emails: [{ value: "Ada@Example.com", label: "work" }],
  },
  { id: "2", addressBookId: "b1", uid: null, fullName: "Grace", emails: null },
];

describe("planImport", () => {
  it("treats an unknown card as fresh", () => {
    const plan = planImport([{ uid: "new", emails: [{ value: "new@example.com", label: null }] }], existing);
    expect(plan.fresh).toHaveLength(1);
    expect(plan.duplicates).toHaveLength(0);
  });

  it("matches an existing uid", () => {
    const plan = planImport([{ uid: "urn-ada", emails: [] }], existing);
    expect(plan.duplicates).toHaveLength(1);
    expect(plan.fresh).toHaveLength(0);
  });

  it("matches any email case insensitively when the uid is absent", () => {
    const plan = planImport([{ uid: null, emails: [{ value: "ADA@example.com", label: null }] }], existing);
    expect(plan.duplicates).toHaveLength(1);
  });

  it("ignores blank emails when matching", () => {
    const plan = planImport([{ uid: null, emails: [{ value: "   ", label: null }] }], existing);
    expect(plan.fresh).toHaveLength(1);
  });

  it("marks a repeat inside the same file as a duplicate", () => {
    const plan = planImport(
      [
        { uid: null, emails: [{ value: "zoe@example.com", label: null }] },
        { uid: null, emails: [{ value: "zoe@example.com", label: null }] },
      ],
      existing,
    );
    expect(plan.fresh).toHaveLength(1);
    expect(plan.duplicates).toHaveLength(1);
  });

  it("keeps the input order within each bucket", () => {
    const plan = planImport(
      [
        { uid: "urn-ada", emails: [] },
        { uid: "a", emails: [] },
        { uid: "b", emails: [] },
      ],
      existing,
    );
    expect(plan.fresh.map((card) => card.uid)).toEqual(["a", "b"]);
  });

  it("handles an empty file", () => {
    expect(planImport([], existing)).toEqual({ fresh: [], duplicates: [] });
  });
});
