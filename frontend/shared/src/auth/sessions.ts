export interface AuthSession {
  token: string;
  isAdmin: boolean;
  username: string;
  accountId?: string;
}

export interface AuthState {
  sessions: AuthSession[];
  active: string | null;
}

export const EMPTY_AUTH: AuthState = { sessions: [], active: null };

export function sameUser(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function asSession(value: unknown): AuthSession | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Partial<AuthSession>;
  if (typeof raw.token !== "string" || !raw.token) return null;
  const session: AuthSession = {
    token: raw.token,
    isAdmin: Boolean(raw.isAdmin),
    username: String(raw.username ?? ""),
  };
  if (typeof raw.accountId === "string" && raw.accountId) session.accountId = raw.accountId;
  return session;
}

export function parseAuthState(raw: string | null): AuthState {
  if (!raw) return EMPTY_AUTH;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return EMPTY_AUTH;
  }
  if (!parsed || typeof parsed !== "object") return EMPTY_AUTH;
  const shape = parsed as { sessions?: unknown; active?: unknown };
  if (Array.isArray(shape.sessions)) {
    const sessions = shape.sessions.map(asSession).filter((entry): entry is AuthSession => entry !== null);
    const wanted = typeof shape.active === "string" ? shape.active : null;
    const active = sessions.find((entry) => wanted !== null && sameUser(entry.username, wanted)) ?? sessions[0];
    return { sessions, active: active?.username ?? null };
  }
  const single = asSession(parsed);
  return single ? { sessions: [single], active: single.username } : EMPTY_AUTH;
}

export function activeSession(state: AuthState): AuthSession | null {
  if (state.active === null) return null;
  return state.sessions.find((entry) => sameUser(entry.username, state.active as string)) ?? null;
}

export function addSession(state: AuthState, session: AuthSession): AuthState {
  const others = state.sessions.filter((entry) => !sameUser(entry.username, session.username));
  return { sessions: [...others, session], active: session.username };
}

export function removeSession(state: AuthState, username: string): AuthState {
  const sessions = state.sessions.filter((entry) => !sameUser(entry.username, username));
  const stillActive = state.active !== null && !sameUser(state.active, username);
  return { sessions, active: stillActive ? state.active : (sessions[0]?.username ?? null) };
}

export function activateSession(state: AuthState, username: string): AuthState {
  const match = state.sessions.find((entry) => sameUser(entry.username, username));
  return match ? { ...state, active: match.username } : state;
}

export function setSessionAccount(state: AuthState, username: string, accountId: string): AuthState {
  let changed = false;
  const sessions = state.sessions.map((entry) => {
    if (!sameUser(entry.username, username) || entry.accountId === accountId) return entry;
    changed = true;
    return { ...entry, accountId };
  });
  return changed ? { ...state, sessions } : state;
}

export function sessionForAccount(sessions: AuthSession[], accountId: string): AuthSession | null {
  return sessions.find((entry) => entry.accountId === accountId) ?? null;
}

export const ACCOUNT_PARAM = "account";

export function accountFromSearch(search: string): string | null {
  const value = new URLSearchParams(search).get(ACCOUNT_PARAM);
  return value && value.trim() ? value.trim() : null;
}

export function stripAccountParam(href: string): string {
  const url = new URL(href);
  url.searchParams.delete(ACCOUNT_PARAM);
  return url.toString();
}

export function withAccountParam(href: string, accountId: string | null): string {
  if (!accountId) return href;
  const url = new URL(href);
  url.searchParams.set(ACCOUNT_PARAM, accountId);
  return url.toString();
}
