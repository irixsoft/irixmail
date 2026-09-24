import { describe, expect, it } from "vitest";
import { IDBFactory } from "fake-indexeddb";

import {
  listAccountLabels,
  listPending,
  putAccountLabel,
  putPending,
  removeAccountLabel,
} from "./pending-verifications";

describe("account labels", () => {
  it("stores, lists and removes labels next to pending verifications", async () => {
    const factory = new IDBFactory();
    await putAccountLabel({ accountId: "1", label: "alice@example.com" }, factory);
    await putAccountLabel({ accountId: "2", label: "bob@example.com" }, factory);
    await putAccountLabel({ accountId: "1", label: "alice@example.com" }, factory);
    await putPending({ subscriptionId: "5", code: "abc" }, factory);

    expect(await listAccountLabels(factory)).toEqual([
      { accountId: "1", label: "alice@example.com" },
      { accountId: "2", label: "bob@example.com" },
    ]);
    expect(await listPending(factory)).toEqual([{ subscriptionId: "5", code: "abc" }]);

    await removeAccountLabel("1", factory);
    expect(await listAccountLabels(factory)).toEqual([{ accountId: "2", label: "bob@example.com" }]);
  });
});
