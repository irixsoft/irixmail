import { beforeEach, describe, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";

import type { JmapClient } from "@irixmail/shared";
import { listPending, putPending } from "./pending-verifications";
import { drainPendingVerifications, teardownPush } from "./web-push";

function fakeJmap(call = vi.fn().mockResolvedValue({})) {
  return { client: { call } as unknown as JmapClient, call };
}

describe("drainPendingVerifications", () => {
  it("verifies and clears every pending entry", async () => {
    const factory = new IDBFactory();
    await putPending({ subscriptionId: "5", code: "abc" }, factory);
    await putPending({ subscriptionId: "6", code: "def" }, factory);
    const { client, call } = fakeJmap();

    const verified = await drainPendingVerifications(client, factory);

    expect(verified).toBe(2);
    expect(call).toHaveBeenCalledWith("PushSubscription/set", { update: { "5": { verificationCode: "abc" } } });
    expect(call).toHaveBeenCalledWith("PushSubscription/set", { update: { "6": { verificationCode: "def" } } });
    expect(await listPending(factory)).toEqual([]);
  });

  it("drops the entry when the server rejects the code", async () => {
    const factory = new IDBFactory();
    await putPending({ subscriptionId: "5", code: "abc" }, factory);
    const { client } = fakeJmap(
      vi.fn().mockResolvedValue({
        notUpdated: { "5": { type: "invalidProperties", description: "verification code does not match" } },
      }),
    );

    const verified = await drainPendingVerifications(client, factory);

    expect(verified).toBe(0);
    expect(await listPending(factory)).toEqual([]);
  });

  it("keeps an entry when verification fails", async () => {
    const factory = new IDBFactory();
    await putPending({ subscriptionId: "5", code: "abc" }, factory);
    const { client } = fakeJmap(vi.fn().mockRejectedValue(new Error("offline")));

    const verified = await drainPendingVerifications(client, factory);

    expect(verified).toBe(0);
    expect(await listPending(factory)).toHaveLength(1);
  });

  it("does nothing when the store is empty", async () => {
    const { client, call } = fakeJmap();
    expect(await drainPendingVerifications(client, new IDBFactory())).toBe(0);
    expect(call).not.toHaveBeenCalled();
  });
});

describe("teardownPush", () => {
  const SUB_KEY = "irixmail.webmail.push.sub.acct";

  beforeEach(() => {
    localStorage.clear();
  });

  it("destroys the server subscription and clears local push state", async () => {
    const factory = new IDBFactory();
    await putPending({ subscriptionId: "5", code: "abc" }, factory);
    localStorage.setItem(SUB_KEY, "9001");
    const { client, call } = fakeJmap();

    await teardownPush(client, "acct", factory);

    expect(call).toHaveBeenCalledWith("PushSubscription/set", { destroy: ["9001"] });
    expect(localStorage.getItem(SUB_KEY)).toBeNull();
    expect(await listPending(factory)).toEqual([]);
  });

  it("clears stale keys from other accounts too", async () => {
    localStorage.setItem(SUB_KEY, "9001");
    localStorage.setItem("irixmail.webmail.push.sub.other", "1");
    const { client } = fakeJmap();

    await teardownPush(client, "acct", new IDBFactory());

    expect(localStorage.getItem("irixmail.webmail.push.sub.other")).toBeNull();
  });

  it("cleans local state without a client and never throws", async () => {
    const factory = new IDBFactory();
    await putPending({ subscriptionId: "5", code: "abc" }, factory);
    localStorage.setItem(SUB_KEY, "9001");

    await expect(teardownPush(null, null, factory)).resolves.toBeUndefined();

    expect(localStorage.getItem(SUB_KEY)).toBeNull();
    expect(await listPending(factory)).toEqual([]);
  });

  it("survives a failing server call", async () => {
    const { client } = fakeJmap(vi.fn().mockRejectedValue(new Error("offline")));
    localStorage.setItem(SUB_KEY, "9001");

    await expect(teardownPush(client, "acct", new IDBFactory())).resolves.toBeUndefined();

    expect(localStorage.getItem(SUB_KEY)).toBeNull();
  });
});
