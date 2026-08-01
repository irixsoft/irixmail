import * as React from "react";

import { ApiClient } from "../api/client";

export interface AuthSession {
  token: string;
  isAdmin: boolean;
  username: string;
}

export type AuthStatus = "loading" | "authenticated" | "unauthenticated";

export type LoginOutcome = { status: "authenticated" } | { status: "totp_required" };

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
  login: (username: string, password: string) => Promise<LoginOutcome>;
  verifyTotp: (code: string) => Promise<LoginOutcome>;
  logout: () => void;
  client: ApiClient;
}

const AuthContext = React.createContext<AuthContextValue | null>(null);

function readStored(storageKey: string): AuthSession | null {
  try {
    const raw = localStorage.getItem(storageKey);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<AuthSession>;
    if (typeof parsed.token === "string" && parsed.token) {
      return {
        token: parsed.token,
        isAdmin: Boolean(parsed.isAdmin),
        username: String(parsed.username ?? ""),
      };
    }
  } catch {
    return null;
  }
  return null;
}

export interface AuthProviderProps {
  children: React.ReactNode;
  baseUrl?: string;
  storageKey?: string;
  validate?: (session: AuthSession) => Promise<boolean>;
}

function AuthProvider({
  children,
  baseUrl,
  storageKey = "irixmail.auth",
  validate,
}: AuthProviderProps) {
  const [session, setSession] = React.useState<AuthSession | null>(() => readStored(storageKey));
  const [status, setStatus] = React.useState<AuthStatus>(() =>
    readStored(storageKey) ? "loading" : "unauthenticated",
  );
  const pendingUsername = React.useRef<string | null>(null);
  const tokenRef = React.useRef<string | null>(session?.token ?? null);
  const logoutRef = React.useRef<() => void>(() => {});

  const persist = React.useCallback(
    (next: AuthSession | null) => {
      setSession(next);
      tokenRef.current = next?.token ?? null;
      try {
        if (next) localStorage.setItem(storageKey, JSON.stringify(next));
        else localStorage.removeItem(storageKey);
      } catch {
        /* storage unavailable */
      }
    },
    [storageKey],
  );

  const logout = React.useCallback(() => {
    const token = tokenRef.current;
    if (token) {
      void fetch(`${baseUrl ?? ""}/api/auth/logout`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => undefined);
    }
    pendingUsername.current = null;
    persist(null);
    setStatus("unauthenticated");
  }, [baseUrl, persist]);

  React.useEffect(() => {
    logoutRef.current = logout;
  }, [logout]);

  const client = React.useMemo(
    () =>
      new ApiClient({
        baseUrl,
        getToken: () => tokenRef.current,
        onUnauthorized: () => logoutRef.current(),
      }),
    [baseUrl],
  );

  React.useEffect(() => {
    const current = readStored(storageKey);
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

  const login = React.useCallback(
    async (username: string, password: string): Promise<LoginOutcome> => {
      const result = await client.post<LoginResponse>("/api/auth/login", { username, password });
      if (result?.totpRequired) {
        pendingUsername.current = username;
        return { status: "totp_required" };
      }
      if (result?.token) {
        pendingUsername.current = null;
        persist({ token: result.token, isAdmin: Boolean(result.isAdmin), username });
        setStatus("authenticated");
        return { status: "authenticated" };
      }
      throw new Error("unexpected login response");
    },
    [client, persist],
  );

  const verifyTotp = React.useCallback(
    async (code: string): Promise<LoginOutcome> => {
      const username = pendingUsername.current;
      if (!username) throw new Error("no pending login to verify");
      const result = await client.post<LoginResponse>("/api/auth/totp", { username, code });
      if (result?.token) {
        pendingUsername.current = null;
        persist({ token: result.token, isAdmin: Boolean(result.isAdmin), username });
        setStatus("authenticated");
        return { status: "authenticated" };
      }
      throw new Error("unexpected totp response");
    },
    [client, persist],
  );

  const value: AuthContextValue = {
    status,
    token: session?.token ?? null,
    isAdmin: session?.isAdmin ?? false,
    username: session?.username ?? null,
    login,
    verifyTotp,
    logout,
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
