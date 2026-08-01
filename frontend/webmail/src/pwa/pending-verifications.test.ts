import { describe, expect, it } from "vitest";
import { IDBFactory } from "fake-indexeddb";

import { listPending, putPending, removePending } from "./pending-verifications";

describe("pending verifications", () => {
  it("stores and lists entries by subscription id", async () => {
    const factory = new IDBFactory();
    await putPending({ subscriptionId: "5", code: "abc" }, factory);
    await putPending({ subscriptionId: "6", code: "def" }, factory);
    await putPending({ subscriptionId: "5", code: "xyz" }, factory);
    const entries = await listPending(factory);
    expect(entries).toHaveLength(2);
    expect(entries.find((entry) => entry.subscriptionId === "5")?.code).toBe("xyz");
    expect(entries.find((entry) => entry.subscriptionId === "6")?.code).toBe("def");
  });

  it("removes an entry", async () => {
    const factory = new IDBFactory();
    await putPending({ subscriptionId: "5", code: "abc" }, factory);
    await removePending("5", factory);
    expect(await listPending(factory)).toEqual([]);
  });

  it("lists nothing from an empty database", async () => {
    expect(await listPending(new IDBFactory())).toEqual([]);
  });
});
