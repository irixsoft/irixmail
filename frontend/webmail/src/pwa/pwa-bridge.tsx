import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { useJmap } from "@/lib/jmap";
import { router } from "@/router";
import { removePending } from "./pending-verifications";
import { drainPendingVerifications, verifySubscription } from "./web-push";

function routerPath(url: string): string | null {
  try {
    const pathname = new URL(url, window.location.origin).pathname;
    if (!pathname.startsWith("/webmail")) return null;
    return pathname.slice("/webmail".length) || "/";
  } catch {
    return null;
  }
}

export function PwaBridge() {
  const jmap = useJmap();
  const queryClient = useQueryClient();

  React.useEffect(() => {
    if (!("serviceWorker" in navigator)) return;
    if (import.meta.env.DEV) {
      void navigator.serviceWorker.getRegistrations().then((registrations) => {
        for (const registration of registrations) void registration.unregister();
      });
      return;
    }
    void navigator.serviceWorker.register("sw.js");
  }, []);

  React.useEffect(() => {
    if (!("serviceWorker" in navigator) || !("indexedDB" in window)) return;
    const drain = () => {
      void drainPendingVerifications(jmap)
        .then((verified) => {
          if (verified > 0) void queryClient.invalidateQueries({ queryKey: ["push-status"] });
        })
        .catch(() => undefined);
    };
    drain();
    let retries = 0;
    const timer = window.setInterval(() => {
      retries += 1;
      if (retries > 5) {
        window.clearInterval(timer);
        return;
      }
      drain();
    }, 60_000);
    const onVisible = () => {
      if (document.visibilityState === "visible") drain();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [jmap, queryClient]);

  React.useEffect(() => {
    if (!("serviceWorker" in navigator)) return;
    const onMessage = (event: MessageEvent) => {
      const data = event.data as { kind?: string } | null;
      if (!data) return;
      if (data.kind === "push-verification") {
        const { subscriptionId, code } = data as { subscriptionId: string; code: string };
        if (subscriptionId && code) {
          void verifySubscription(jmap, subscriptionId, code)
            .then(() => {
              void removePending(subscriptionId).catch(() => undefined);
              void queryClient.invalidateQueries({ queryKey: ["push-status"] });
            })
            .catch((error: unknown) => {
              console.warn("push verification failed", error);
            });
        }
      }
      if (data.kind === "state-change") {
        for (const key of [["emails"], ["email"], ["thread"], ["mailboxes"]]) {
          void queryClient.invalidateQueries({ queryKey: key });
        }
      }
      if (data.kind === "open-url") {
        const path = routerPath((data as { url?: string }).url ?? "");
        if (path) void router.navigate(path);
      }
    };
    navigator.serviceWorker.addEventListener("message", onMessage);
    return () => navigator.serviceWorker.removeEventListener("message", onMessage);
  }, [jmap, queryClient]);

  return null;
}
