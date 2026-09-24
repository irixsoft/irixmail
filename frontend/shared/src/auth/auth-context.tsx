import * as React from "react";

import { ApiClient } from "../api/client";
import {
  activateSession,
  activeSession,
  addSession,
  parseAuthState,
  removeSession,
  sessionForAccount,
  setSessionAccount,
  type AuthSession,
  type AuthState,
} from "./sessions";

export type AuthStatus = "loading" | "authenticated" | "unauthenticated";

export type LoginOutcome = { status: "authenticated" } | { status: "totp_required" };

export type SessionKind = "admin" | "webmail";

interface LoginResponse {
  token?: string;
  isAdmin?: boolean;
  totpRequired?: boolean;
}

export interface AuthContextValue {
  status: AuthStatus;
  token: string | null;
  isAdmin: boolean;
  username: string | null;
  accountId: string | null;
  sessions: AuthSession[];
  login: (username: string, password: string) => Promise<LoginOutcome>;
  verifyTotp: (code: string) => Promise<LoginOutcome>;
  logout: () => void;
  switchTo: (username: string) => void;
  setAccountId: (username: string, accountId: string) => void;
  client: ApiClient;
}

const AuthContext = React.createContext<AuthContextValue | null>(null);

export const LEGACY_STORAGE_KEY = "irixmail.auth";

export function dropLegacySession(storage: Pick<Storage, "removeItem"> = localStorage): void {
  try {
    storage.removeItem(LEGACY_STORAGE_KEY);
  } catch {
    /* storage unavailable */
  }
}

function readState(storageKey: string): AuthState {
  try {
    return parseAuthState(localStorage.getItem(storageKey));
  } catch {
    return parseAuthState(null);
  }
}

export interface AuthProviderProps {
  children: React.ReactNode;
  kind: SessionKind;
  storageKey: string;
  baseUrl?: string;
  validate?: (session: AuthSession) => Promise<boolean>;
  preferAccountId?: string | null;
}

function AuthProvider({ children, kind, storageKey, baseUrl, validate, preferAccountId }: AuthProviderProps) {
  const [auth, setAuth] = React.useState<AuthState>(() => {
    dropLegacySession();
    const stored = readState(storageKey);
    const preferred = preferAccountId ? sessionForAccount(stored.sessions, preferAccountId) : null;
    return preferred ? activateSession(stored, preferred.username) : stored;
  });
  const [status, setStatus] = React.useState<AuthStatus>(() =>
    activeSession(auth) ? "loading" : "unauthenticated",
  );
  const authRef = React.useRef(auth);
  const pendingUsername = React.useRef<string | null>(null);
  const logoutRef = React.useRef<() => void>(() => {});

  const persist = React.useCallback(
    (next: AuthState) => {
      authRef.current = next;
      setAuth(next);
      try {
        if (next.sessions.length > 0) localStorage.setItem(storageKey, JSON.stringify(next));
        else localStorage.removeItem(storageKey);
      } catch {
        /* storage unavailable */
      }
    },
    [storageKey],
  );

  const revoke = React.useCallback(
    (token: string) => {
      void fetch(`${baseUrl ?? ""}/api/auth/logout`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => undefined);
    },
    [baseUrl],
  );

  const logout = React.useCallback(() => {
    const current = activeSession(authRef.current);
    pendingUsername.current = null;
    if (!current) {
      setStatus("unauthenticated");
      return;
    }
    revoke(current.token);
    const next = removeSession(authRef.current, current.username);
    persist(next);
    setStatus(activeSession(next) ? "authenticated" : "unauthenticated");
  }, [persist, revoke]);

  React.useEffect(() => {
    logoutRef.current = logout;
  }, [logout]);

  const client = React.useMemo(
    () =>
      new ApiClient({
        baseUrl,
        getToken: () => activeSession(authRef.current)?.token ?? null,
        onUnauthorized: () => logoutRef.current(),
      }),
    [baseUrl],
  );

  React.useEffect(() => {
    const current = activeSession(authRef.current);
    if (!current) {
      setStatus("unauthenticated");
      return;
    }
    if (!validate) {
      setStatus("authenticated");
      return;
    }
    let cancelled = false;
    validate(current)
      .then((ok) => {
        if (cancelled) return;
        if (ok) setStatus("authenticated");
        else logout();
      })
      .catch(() => {
        if (!cancelled) logout();
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const accept = React.useCallback(
    (username: string, result: LoginResponse | undefined): LoginOutcome => {
      if (result?.totpRequired) {
        pendingUsername.current = username;
        return { status: "totp_required" };
      }
      if (result?.token) {
        pendingUsername.current = null;
        persist(addSession(authRef.current, { token: result.token, isAdmin: Boolean(result.isAdmin), username }));
        setStatus("authenticated");
        return { status: "authenticated" };
      }
      throw new Error("unexpected login response");
    },
    [persist],
  );

  const login = React.useCallback(
    async (username: string, password: string): Promise<LoginOutcome> => {
      const result = await client.post<LoginResponse>("/api/auth/login", { kind, username, password });
      return accept(username, result);
    },
    [accept, client, kind],
  );

  const verifyTotp = React.useCallback(
    async (code: string): Promise<LoginOutcome> => {
      const username = pendingUsername.current;
      if (!username) throw new Error("no pending login to verify");
      const result = await client.post<LoginResponse>("/api/auth/totp", { username, code });
      return accept(username, result);
    },
    [accept, client],
  );

  const switchTo = React.useCallback(
    (username: string) => {
      const next = activateSession(authRef.current, username);
      if (next === authRef.current) return;
      persist(next);
      setStatus("authenticated");
    },
    [persist],
  );

  const setAccountId = React.useCallback(
    (username: string, accountId: string) => {
      const next = setSessionAccount(authRef.current, username, accountId);
      if (next !== authRef.current) persist(next);
    },
    [persist],
  );

  const current = activeSession(auth);
  const value: AuthContextValue = {
    status,
    token: current?.token ?? null,
    isAdmin: current?.isAdmin ?? false,
    username: current?.username ?? null,
    accountId: current?.accountId ?? null,
    sessions: auth.sessions,
    login,
    verifyTotp,
    logout,
    switchTo,
    setAccountId,
    client,
  };

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

function useAuth(): AuthContextValue {
  const context = React.useContext(AuthContext);
  if (!context) throw new Error("useAuth must be used within an AuthProvider");
  return context;
}

export { AuthProvider, useAuth };
