import { describe, expect, it } from "vitest";
import {
  accountFromSearch,
  activateSession,
  activeSession,
  addSession,
  parseAuthState,
  removeSession,
  sessionForAccount,
  setSessionAccount,
  stripAccountParam,
  withAccountParam,
  type AuthSession,
} from "@irixmail/shared";

const alice: AuthSession = { token: "t-a", isAdmin: false, username: "alice@example.com", accountId: "1" };
const bob: AuthSession = { token: "t-b", isAdmin: true, username: "bob@example.com" };

describe("parseAuthState", () => {
  it("reads the multi-session shape and keeps the chosen active user", () => {
    const state = parseAuthState(JSON.stringify({ sessions: [alice, bob], active: "BOB@example.com" }));
    expect(state.sessions).toEqual([alice, bob]);
    expect(activeSession(state)).toEqual(bob);
  });

  it("wraps a single stored session", () => {
    const state = parseAuthState(JSON.stringify(alice));
    expect(state).toEqual({ sessions: [alice], active: alice.username });
  });

  it("falls back to the first session when the active user is unknown, and to empty on garbage", () => {
    expect(activeSession(parseAuthState(JSON.stringify({ sessions: [alice], active: "ghost" })))).toEqual(alice);
    expect(parseAuthState("{not json")).toEqual({ sessions: [], active: null });
    expect(parseAuthState(JSON.stringify({ sessions: [{ token: "" }] }))).toEqual({ sessions: [], active: null });
    expect(parseAuthState(null)).toEqual({ sessions: [], active: null });
  });
});

describe("session list changes", () => {
  it("adding replaces the same user and makes it active", () => {
    const state = addSession({ sessions: [alice], active: alice.username }, bob);
    expect(state.active).toBe(bob.username);
    const again = addSession(state, { ...alice, token: "t-a2" });
    expect(again.sessions.map((entry) => entry.token)).toEqual(["t-b", "t-a2"]);
    expect(again.active).toBe(alice.username);
  });

  it("removing the active user activates the next one, or nothing", () => {
    const both = { sessions: [alice, bob], active: alice.username };
    const withoutAlice = removeSession(both, "ALICE@example.com");
    expect(withoutAlice).toEqual({ sessions: [bob], active: bob.username });
    expect(removeSession(withoutAlice, bob.username)).toEqual({ sessions: [], active: null });
    expect(removeSession(both, bob.username).active).toBe(alice.username);
  });

  it("activating an unknown user is a no-op", () => {
    const state = { sessions: [alice], active: alice.username };
    expect(activateSession(state, "ghost@example.com")).toBe(state);
    expect(activateSession({ sessions: [alice, bob], active: alice.username }, bob.username).active).toBe(bob.username);
  });

  it("records the account id once", () => {
    const state = { sessions: [bob], active: bob.username };
    const next = setSessionAccount(state, bob.username, "2");
    expect(next.sessions[0]?.accountId).toBe("2");
    expect(setSessionAccount(next, bob.username, "2")).toBe(next);
    expect(sessionForAccount(next.sessions, "2")).toEqual({ ...bob, accountId: "2" });
    expect(sessionForAccount(next.sessions, "9")).toBeNull();
  });
});

describe("account url parameter", () => {
  it("round-trips through the query string", () => {
    expect(accountFromSearch("?account=7&x=1")).toBe("7");
    expect(accountFromSearch("?x=1")).toBeNull();
    expect(withAccountParam("https://mail.example.com/webmail/1/2", "7")).toBe(
      "https://mail.example.com/webmail/1/2?account=7",
    );
    expect(stripAccountParam("https://mail.example.com/webmail/?account=7&x=1")).toBe(
      "https://mail.example.com/webmail/?x=1",
    );
  });
});
