import * as React from "react";
import { QueryClient, useQueryClient } from "@tanstack/react-query";
import { PersistQueryClientProvider } from "@tanstack/react-query-persist-client";
import { createAsyncStoragePersister } from "@tanstack/query-async-storage-persister";
import { MotionConfig } from "motion/react";
import { RouterProvider } from "react-router-dom";
import {
  AuthProvider,
  JmapClient,
  Toaster,
  useAuth,
  usePush,
  type AuthSession,
} from "@irixmail/shared";

import { JmapProvider } from "@/lib/jmap";
import { sessionStillValid } from "@/lib/session-validity";
import {
  PERSIST_BUSTER,
  PERSIST_MAX_AGE,
  shouldPersistQuery,
} from "@/pwa/persisted-queries";
import { createQueryStorage } from "@/pwa/query-storage";
import { PwaBridge } from "@/pwa/pwa-bridge";
import { teardownPush } from "@/pwa/web-push";
import { router } from "@/router";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, refetchOnWindowFocus: false, gcTime: PERSIST_MAX_AGE },
  },
});

const queryStorage = createQueryStorage();

const persistOptions = {
  persister: createAsyncStoragePersister({ storage: queryStorage, key: "irixmail.query-cache" }),
  maxAge: PERSIST_MAX_AGE,
  buster: PERSIST_BUSTER,
  dehydrateOptions: { shouldDehydrateQuery: shouldPersistQuery },
};

const PUSH_INVALIDATIONS: Record<string, string[][]> = {
  Email: [["emails"], ["email"], ["search"], ["mailboxes"]],
  Mailbox: [["mailboxes"]],
  Thread: [["emails"], ["email"]],
  Identity: [["identities"]],
  Calendar: [["calendars"], ["calendar-events"]],
  CalendarEvent: [["calendar-events"]],
  AddressBook: [["address-books"], ["contacts"]],
  ContactCard: [["contacts"]],
};

function LivePush() {
  const { token } = useAuth();
  const client = useQueryClient();
  usePush({
    enabled: Boolean(token),
    getToken: () => token,
    ping: 30,
    onEvent: (event) => {
      if (event.event !== "state") return;
      let changed: Record<string, Record<string, string>>;
      try {
        changed = JSON.parse(event.data)?.changed ?? {};
      } catch {
        return;
      }
      const types = new Set<string>();
      for (const account of Object.values(changed)) {
        for (const type of Object.keys(account)) types.add(type);
      }
      for (const type of types) {
        for (const queryKey of PUSH_INVALIDATIONS[type] ?? []) {
          void client.invalidateQueries({ queryKey });
        }
      }
    },
  });
  return null;
}

function CacheReset() {
  const { token } = useAuth();
  const client = useQueryClient();
  const previous = React.useRef(token);
  React.useEffect(() => {
    if (previous.current && !token) {
      client.clear();
      void queryStorage.clear();
      void teardownPush(null, null);
    }
    previous.current = token;
  }, [token, client]);
  return null;
}

async function validateSession(session: AuthSession): Promise<boolean> {
  const client = new JmapClient({ baseUrl: "", getToken: () => session.token });
  try {
    await client.session();
    return true;
  } catch (error) {
    return sessionStillValid(error);
  }
}

function ThemedToaster() {
  const [theme, setTheme] = React.useState<"light" | "dark">(() =>
    document.documentElement.classList.contains("dark") ? "dark" : "light",
  );
  React.useEffect(() => {
    const observer = new MutationObserver(() =>
      setTheme(document.documentElement.classList.contains("dark") ? "dark" : "light"),
    );
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);
  return <Toaster position="top-right" theme={theme} richColors closeButton />;
}

export function App() {
  return (
    <PersistQueryClientProvider client={queryClient} persistOptions={persistOptions}>
      <AuthProvider baseUrl="" validate={validateSession}>
        <JmapProvider>
          <MotionConfig reducedMotion="user">
            <LivePush />
            <CacheReset />
            <PwaBridge />
            <RouterProvider router={router} />
            <ThemedToaster />
          </MotionConfig>
        </JmapProvider>
      </AuthProvider>
    </PersistQueryClientProvider>
  );
}
