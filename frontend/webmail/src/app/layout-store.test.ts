import { beforeEach, describe, expect, it } from "vitest";
import {
  PANE_LIMITS,
  clampPane,
  getLayout,
  loadLayout,
  saveLayout,
  setLayout,
  subscribeLayout,
} from "./layout-store";

beforeEach(() => {
  localStorage.clear();
  setLayout(loadLayout());
});

describe("clampPane", () => {
  it("clamps folder and list widths to their limits", () => {
    expect(clampPane("folders", 10)).toBe(PANE_LIMITS.folders.min);
    expect(clampPane("folders", 9999)).toBe(PANE_LIMITS.folders.max);
    expect(clampPane("list", 384)).toBe(384);
  });
});

describe("layout persistence", () => {
  it("defaults to the standard pane widths", () => {
    const layout = loadLayout();
    expect(layout.folders).toBe(PANE_LIMITS.folders.initial);
    expect(layout.list).toBe(PANE_LIMITS.list.initial);
    expect(layout.foldersCollapsed).toBe(false);
    expect(layout.readingPane).toBe("right");
    expect(layout.listHeight).toBe(PANE_LIMITS.listHeight.initial);
  });

  it("round-trips and clamps stored values", () => {
    saveLayout({
      folders: 300,
      list: 9999,
      foldersCollapsed: true,
      readingPane: "bottom",
      listHeight: 40,
    });
    const layout = loadLayout();
    expect(layout.folders).toBe(300);
    expect(layout.list).toBe(PANE_LIMITS.list.max);
    expect(layout.foldersCollapsed).toBe(true);
    expect(layout.readingPane).toBe("bottom");
    expect(layout.listHeight).toBe(PANE_LIMITS.listHeight.min);
  });

  it("falls back to the right reading pane for unknown values", () => {
    localStorage.setItem(
      "irixmail.webmail.layout",
      JSON.stringify({ readingPane: "sideways", listHeight: "tall" }),
    );
    const layout = loadLayout();
    expect(layout.readingPane).toBe("right");
    expect(layout.listHeight).toBe(PANE_LIMITS.listHeight.initial);
  });

  it("survives malformed storage", () => {
    localStorage.setItem("irixmail.webmail.layout", "not json");
    expect(loadLayout().folders).toBe(PANE_LIMITS.folders.initial);
  });
});

describe("layout subscription", () => {
  it("notifies subscribers on save and on transient set", () => {
    let calls = 0;
    const unsubscribe = subscribeLayout(() => {
      calls += 1;
    });
    saveLayout({ ...getLayout(), readingPane: "off" });
    expect(calls).toBe(1);
    expect(getLayout().readingPane).toBe("off");
    expect(loadLayout().readingPane).toBe("off");

    setLayout({ ...getLayout(), list: 300 });
    expect(calls).toBe(2);
    expect(getLayout().list).toBe(300);
    expect(loadLayout().list).not.toBe(300);

    unsubscribe();
    saveLayout({ ...getLayout(), foldersCollapsed: true });
    expect(calls).toBe(2);
  });

  it("keeps the snapshot identity stable between changes", () => {
    expect(getLayout()).toBe(getLayout());
  });
});
