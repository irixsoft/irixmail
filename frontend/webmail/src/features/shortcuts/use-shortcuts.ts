import * as React from "react";

import { isEditablePath, shortcutToken } from "@/lib/keyboard";

export type ShortcutHandlers = Record<string, (event: KeyboardEvent) => void>;

export function useShortcuts(handlers: ShortcutHandlers, enabled = true) {
  const ref = React.useRef(handlers);
  ref.current = handlers;

  React.useEffect(() => {
    if (!enabled) return;
    const listener = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat) return;
      if (isEditablePath(event.composedPath())) return;
      const handler = ref.current[shortcutToken(event)];
      if (handler) {
        event.preventDefault();
        handler(event);
      }
    };
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, [enabled]);
}
