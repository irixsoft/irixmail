import { describe, expect, it } from "vitest";
import { LIST_PROPS, emailListCalls, threadCalls } from "./requests";

describe("emailListCalls", () => {
  it("pairs a query with a back-referenced get", () => {
    const calls = emailListCalls("1", { filter: { inMailbox: "m" }, position: 30, limit: 30 });
    expect(calls).toHaveLength(2);
    const [query, get] = calls;
    expect(query![0]).toBe("Email/query");
    expect(query![1]).toMatchObject({
      accountId: "1",
      filter: { inMailbox: "m" },
      position: 30,
      limit: 30,
      calculateTotal: true,
      sort: [{ property: "receivedAt", isAscending: false }],
    });
    expect(get![0]).toBe("Email/get");
    expect(get![1]).toMatchObject({
      accountId: "1",
      properties: LIST_PROPS,
      "#ids": { resultOf: query![2], name: "Email/query", path: "/ids" },
    });
  });
});

describe("threadCalls", () => {
  it("resolves thread email ids into a full get", () => {
    const calls = threadCalls("1", "t9");
    const [thread, get] = calls;
    expect(thread![0]).toBe("Thread/get");
    expect(thread![1]).toMatchObject({ accountId: "1", ids: ["t9"] });
    expect(get![1]).toMatchObject({
      "#ids": { resultOf: thread![2], name: "Thread/get", path: "/list/*/emailIds" },
    });
  });
});
