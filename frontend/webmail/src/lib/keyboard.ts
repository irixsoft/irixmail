const NAMED_KEYS = new Set(["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight", "Enter", "Escape", "Delete", "Backspace"]);

const SHIFTED_DIGITS: Record<string, string> = {
  Digit1: "!",
  Digit3: "#",
};

export function physicalShortcutKey(event: KeyboardEvent): string {
  if (NAMED_KEYS.has(event.key)) return event.key;
  if (event.code.startsWith("Key")) return event.code.slice(3).toLowerCase();
  if (event.code === "Slash") return event.shiftKey ? "?" : "/";
  if (event.shiftKey && event.code in SHIFTED_DIGITS) return SHIFTED_DIGITS[event.code]!;
  if (event.code.startsWith("Digit")) return event.code.slice(5);
  return event.key;
}

export function shortcutToken(event: KeyboardEvent): string {
  const key = physicalShortcutKey(event);
  if (event.metaKey || event.ctrlKey) return `mod+${key}`;
  if (event.shiftKey && /^[a-z]$/.test(key)) return `shift+${key}`;
  return key;
}

export function isEditablePath(path: EventTarget[]): boolean {
  for (const target of path) {
    if (!(target instanceof HTMLElement)) continue;
    const tag = target.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
    if (target.isContentEditable || target.getAttribute("contenteditable") === "true") return true;
  }
  return false;
}
