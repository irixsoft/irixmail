import { describe, expect, it } from "vitest";

import { PERSIST_MAX_AGE, shouldPersistQuery } from "./persisted-queries";

const query = (queryKey: unknown[], status = "success") => ({ queryKey, state: { status } });

describe("shouldPersistQuery", () => {
  it("persists mail data", () => {
    expect(shouldPersistQuery(query(["emails", "a", { inMailbox: "1" }]))).toBe(true);
    expect(shouldPersistQuery(query(["email", "a", "7"]))).toBe(true);
    expect(shouldPersistQuery(query(["thread", "a", "7"]))).toBe(true);
    expect(shouldPersistQuery(query(["mailboxes", "a"]))).toBe(true);
    expect(shouldPersistQuery(query(["identities", "a"]))).toBe(true);
  });

  it("persists calendar and contact data", () => {
    expect(shouldPersistQuery(query(["calendars", "a"]))).toBe(true);
    expect(shouldPersistQuery(query(["calendar-events", "a"]))).toBe(true);
    expect(shouldPersistQuery(query(["contacts", "a"]))).toBe(true);
    expect(shouldPersistQuery(query(["address-books", "a"]))).toBe(true);
  });

  it("never persists account or session data", () => {
    expect(shouldPersistQuery(query(["jmap-session"]))).toBe(false);
    expect(shouldPersistQuery(query(["me"]))).toBe(false);
    expect(shouldPersistQuery(query(["me", "app-passwords"]))).toBe(false);
    expect(shouldPersistQuery(query(["push-status", "a"]))).toBe(false);
    expect(shouldPersistQuery(query(["totp"]))).toBe(false);
    expect(shouldPersistQuery(query(["search", "a"]))).toBe(false);
  });

  it("never persists a query that did not succeed", () => {
    expect(shouldPersistQuery(query(["emails", "a"], "pending"))).toBe(false);
    expect(shouldPersistQuery(query(["emails", "a"], "error"))).toBe(false);
  });

  it("ignores a non-string key root", () => {
    expect(shouldPersistQuery(query([{ kind: "emails" }]))).toBe(false);
    expect(shouldPersistQuery(query([]))).toBe(false);
  });

  it("keeps cached mail for a week", () => {
    expect(PERSIST_MAX_AGE).toBe(7 * 24 * 60 * 60 * 1000);
  });
});
