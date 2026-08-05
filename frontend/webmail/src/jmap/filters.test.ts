import { describe, expect, it } from "vitest";

import {
  emptyRule,
  isExternallyEdited,
  removeRule,
  ruleSummary,
  savePayload,
  scriptRules,
  upsertRule,
  type FilterRule,
  type SieveScript,
} from "./filters";

const rule = (patch: Partial<FilterRule> = {}): FilterRule => ({
  ...emptyRule("r1"),
  name: "Receipts",
  field: "subject",
  operator: "contains",
  value: "receipt",
  action: "fileinto",
  target: "Receipts",
  ...patch,
});

describe("scriptRules", () => {
  it("treats a missing script as an empty rule list", () => {
    expect(scriptRules(undefined)).toEqual([]);
  });

  it("returns the rules of a builder-managed script", () => {
    const script: SieveScript = { id: "s1", name: "filters", rules: [rule()] };
    expect(scriptRules(script)).toHaveLength(1);
  });

  it("returns null for an externally edited script", () => {
    const script: SieveScript = { id: "s1", name: "custom", rules: null, source: "keep;" };
    expect(scriptRules(script)).toBeNull();
    expect(isExternallyEdited(script)).toBe(true);
    expect(isExternallyEdited(undefined)).toBe(false);
  });
});

describe("upsertRule", () => {
  it("appends a new rule and preserves order", () => {
    const first = rule();
    const second = rule({ id: "r2", name: "Later" });
    expect(upsertRule([first], second)).toEqual([first, second]);
  });

  it("replaces an existing rule in place", () => {
    const first = rule();
    const second = rule({ id: "r2" });
    const edited = rule({ value: "invoice" });
    expect(upsertRule([first, second], edited)).toEqual([edited, second]);
  });
});

describe("removeRule", () => {
  it("removes only the matching rule", () => {
    const first = rule();
    const second = rule({ id: "r2" });
    expect(removeRule([first, second], "r1")).toEqual([second]);
    expect(removeRule([first], "missing")).toEqual([first]);
  });
});

describe("ruleSummary", () => {
  it("describes the condition and the action with its target", () => {
    expect(ruleSummary(rule())).toBe("Subject contains “receipt” → Move to folder Receipts");
  });

  it("omits an empty target", () => {
    expect(ruleSummary(rule({ action: "discard", target: "" }))).toBe(
      "Subject contains “receipt” → Discard",
    );
  });
});

describe("savePayload", () => {
  it("updates the existing script by id", () => {
    const payload = savePayload("a1", "s1", [rule()]);
    expect(payload.update).toEqual({ s1: { name: "filters", rules: [rule()] } });
    expect(payload.create).toBeUndefined();
  });

  it("creates the script when none exists", () => {
    const payload = savePayload("a1", undefined, []);
    expect(payload.create).toEqual({ filters: { name: "filters", rules: [] } });
    expect(payload.update).toBeUndefined();
  });
});

describe("emptyRule", () => {
  it("starts as a fileinto rule on the from field", () => {
    const fresh = emptyRule("fixed");
    expect(fresh).toEqual({
      id: "fixed",
      name: "",
      field: "from",
      operator: "contains",
      value: "",
      action: "fileinto",
      target: "",
    });
  });
});
