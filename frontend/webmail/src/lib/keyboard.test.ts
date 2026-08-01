import { describe, expect, it } from "vitest";
import { isEditablePath, physicalShortcutKey, shortcutToken } from "./keyboard";

function event(code: string, key: string, shiftKey = false, meta = false, ctrl = false) {
  return { code, key, shiftKey, metaKey: meta, ctrlKey: ctrl } as KeyboardEvent;
}

describe("physicalShortcutKey", () => {
  it("maps letter codes independent of layout", () => {
    expect(physicalShortcutKey(event("KeyJ", "о"))).toBe("j");
    expect(physicalShortcutKey(event("KeyA", "a"))).toBe("a");
  });

  it("maps slash and question mark", () => {
    expect(physicalShortcutKey(event("Slash", "/"))).toBe("/");
    expect(physicalShortcutKey(event("Slash", "?", true))).toBe("?");
  });

  it("maps shifted digits used by shortcuts", () => {
    expect(physicalShortcutKey(event("Digit3", "#", true))).toBe("#");
    expect(physicalShortcutKey(event("Digit1", "!", true))).toBe("!");
  });

  it("keeps named keys", () => {
    expect(physicalShortcutKey(event("ArrowDown", "ArrowDown"))).toBe("ArrowDown");
    expect(physicalShortcutKey(event("Enter", "Enter"))).toBe("Enter");
    expect(physicalShortcutKey(event("Escape", "Escape"))).toBe("Escape");
  });
});

describe("shortcutToken", () => {
  it("prefixes mod for ctrl or meta", () => {
    expect(shortcutToken(event("KeyK", "k", false, true))).toBe("mod+k");
    expect(shortcutToken(event("KeyA", "a", false, false, true))).toBe("mod+a");
  });

  it("prefixes shift for letters but not for punctuation results", () => {
    expect(shortcutToken(event("KeyR", "R", true))).toBe("shift+r");
    expect(shortcutToken(event("Slash", "?", true))).toBe("?");
  });

  it("plain keys pass through", () => {
    expect(shortcutToken(event("KeyJ", "j"))).toBe("j");
  });
});

describe("isEditablePath", () => {
  it("detects inputs textareas and contenteditable", () => {
    const input = document.createElement("input");
    const textarea = document.createElement("textarea");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    const plain = document.createElement("div");
    expect(isEditablePath([input])).toBe(true);
    expect(isEditablePath([textarea])).toBe(true);
    expect(isEditablePath([editable, plain])).toBe(true);
    expect(isEditablePath([plain])).toBe(false);
  });
});
