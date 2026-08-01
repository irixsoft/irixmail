import * as React from "react";

import { PushClient, type PushEvent } from "./client";

export interface UsePushOptions {
  enabled?: boolean;
  baseUrl?: string;
  getToken?: () => string | null;
  types?: string[];
  ping?: number;
  onEvent?: (event: PushEvent) => void;
}

export function usePush({
  enabled = true,
  baseUrl,
  getToken,
  types,
  ping,
  onEvent,
}: UsePushOptions): void {
  const onEventRef = React.useRef(onEvent);
  const getTokenRef = React.useRef(getToken);

  React.useEffect(() => {
    onEventRef.current = onEvent;
    getTokenRef.current = getToken;
  });

  const typesKey = types && types.length > 0 ? types.join(",") : "";

  React.useEffect(() => {
    if (!enabled) return;
    const client = new PushClient({
      baseUrl,
      getToken: () => getTokenRef.current?.() ?? null,
      types: typesKey ? typesKey.split(",") : undefined,
      ping,
    });
    const off = client.on((event) => onEventRef.current?.(event));
    client.start();
    return () => {
      off();
      client.close();
    };
  }, [enabled, baseUrl, typesKey, ping]);
}
