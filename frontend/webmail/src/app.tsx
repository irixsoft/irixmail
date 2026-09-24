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
  accountFromSearch,
  sameUser,
  stripAccountParam,
  useAuth,
  usePush,
  type AuthSession,
} from "@irixmail/shared";

import { JmapProvider } from "@/lib/jmap";
import { sessionStillValid } from "@/lib/session-validity";
import { removeAccountLabel } from "@/pwa/pending-verifications";
import {
  PERSIST_BUSTER,
  PERSIST_MAX_AGE,
  shouldPersistQuery,
} from "@/pwa/persisted-queries";
import { createQueryStorage } from "@/pwa/query-storage";
import { PwaBridge } from "@/pwa/pwa-bridge";
import { forgetPush, teardownPush } from "@/pwa/web-push";
import { router } from "@/router";

const queryStorage = createQueryStorage();

const clients = new Map<string, QueryClient>();

function scopeOf(username: string): string {
  return username.trim().toLowerCase();
}

function cacheKey(scope: string): string {
  return `irixmail.query-cache.${scope}`;
}

function clientFor(scope: string): QueryClient {
  const existing = clients.get(scope);
  if (existing) return existing;
  const created = new QueryClient({
    defaultOptions: {
      queries: { retry: 1, refetchOnWindowFocus: false, gcTime: PERSIST_MAX_AGE },
    },
  });
  clients.set(scope, created);
  return created;
}

function persistOptionsFor(scope: string) {
  return {
    persister: createAsyncStoragePersister({ storage: queryStorage, key: cacheKey(scope) }),
    maxAge: PERSIST_MAX_AGE,
    buster: PERSIST_BUSTER,
    dehydrateOptions: { shouldDehydrateQuery: shouldPersistQuery },
  };
}

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

function forgetSession(session: AuthSession) {
  const scope = scopeOf(session.username);
  clients.get(scope)?.clear();
  clients.delete(scope);
  void queryStorage.removeItem(cacheKey(scope));
  if (session.accountId) {
    forgetPush(session.accountId);
    void removeAccountLabel(session.accountId).catch(() => undefined);
  }
}

function SessionCleanup() {
  const { sessions } = useAuth();
  const previous = React.useRef(sessions);
  React.useEffect(() => {
    const gone = previous.current.filter(
      (old) => !sessions.some((current) => sameUser(current.username, old.username)),
    );
    for (const session of gone) forgetSession(session);
    if (gone.length > 0 && sessions.length === 0) void teardownPush(null, null);
    previous.current = sessions;
  }, [sessions]);
  return null;
}

function AccountScope({ children }: { children: React.ReactNode }) {
  const { status, username } = useAuth();
  const scope = status === "authenticated" && username ? scopeOf(username) : "anonymous";
  return (
    <PersistQueryClientProvider key={scope} client={clientFor(scope)} persistOptions={persistOptionsFor(scope)}>
      {children}
    </PersistQueryClientProvider>
  );
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

function bootAccount(): string | null {
  const wanted = accountFromSearch(window.location.search);
  if (wanted) window.history.replaceState(null, "", stripAccountParam(window.location.href));
  return wanted;
}

export function App() {
  const [preferAccountId] = React.useState(bootAccount);
  return (
    <AuthProvider
      kind="webmail"
      storageKey="irixmail.webmail"
      baseUrl=""
      validate={validateSession}
      preferAccountId={preferAccountId}
    >
      <AccountScope>
        <JmapProvider>
          <MotionConfig reducedMotion="user">
            <LivePush />
            <SessionCleanup />
            <PwaBridge />
            <RouterProvider router={router} />
            <ThemedToaster />
          </MotionConfig>
        </JmapProvider>
      </AccountScope>
    </AuthProvider>
  );
}
