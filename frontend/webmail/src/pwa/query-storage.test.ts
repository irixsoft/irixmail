import { describe, expect, it } from "vitest";
import { IDBFactory } from "fake-indexeddb";

import { createQueryStorage } from "./query-storage";

describe("query storage", () => {
  it("round trips a value", async () => {
    const storage = createQueryStorage(new IDBFactory());
    await storage.setItem("cache", '{"a":1}');
    expect(await storage.getItem("cache")).toBe('{"a":1}');
  });

  it("overwrites an existing value", async () => {
    const storage = createQueryStorage(new IDBFactory());
    await storage.setItem("cache", "one");
    await storage.setItem("cache", "two");
    expect(await storage.getItem("cache")).toBe("two");
  });

  it("returns null for a missing key", async () => {
    const storage = createQueryStorage(new IDBFactory());
    expect(await storage.getItem("nope")).toBeNull();
  });

  it("removes a value", async () => {
    const storage = createQueryStorage(new IDBFactory());
    await storage.setItem("cache", "one");
    await storage.removeItem("cache");
    expect(await storage.getItem("cache")).toBeNull();
  });

  it("clears every value", async () => {
    const storage = createQueryStorage(new IDBFactory());
    await storage.setItem("a", "1");
    await storage.setItem("b", "2");
    await storage.clear();
    expect(await storage.getItem("a")).toBeNull();
    expect(await storage.getItem("b")).toBeNull();
  });

  it("resolves to null instead of throwing when storage is unavailable", async () => {
    const broken = {
      open: () => {
        throw new Error("blocked");
      },
    } as unknown as IDBFactory;
    const storage = createQueryStorage(broken);
    expect(await storage.getItem("a")).toBeNull();
    await expect(storage.setItem("a", "1")).resolves.toBeUndefined();
    await expect(storage.clear()).resolves.toBeUndefined();
  });
});
