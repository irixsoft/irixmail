import * as React from "react";

const QUERY = "(max-width: 767px)";

export function useIsMobile(): boolean {
  return React.useSyncExternalStore(
    (onChange) => {
      const media = window.matchMedia(QUERY);
      media.addEventListener("change", onChange);
      return () => media.removeEventListener("change", onChange);
    },
    () => window.matchMedia(QUERY).matches,
    () => false,
  );
}
