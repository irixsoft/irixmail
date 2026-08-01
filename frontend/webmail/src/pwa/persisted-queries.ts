export const PERSIST_MAX_AGE = 7 * 24 * 60 * 60 * 1000;
export const PERSIST_BUSTER = "1";

const PERSISTED_ROOTS = new Set([
  "emails",
  "email",
  "thread",
  "mailboxes",
  "identities",
  "calendars",
  "calendar-events",
  "contacts",
  "address-books",
]);

export interface PersistableQuery {
  queryKey: readonly unknown[];
  state: { status: string };
}

export function shouldPersistQuery(query: PersistableQuery): boolean {
  if (query.state.status !== "success") return false;
  const root = query.queryKey[0];
  return typeof root === "string" && PERSISTED_ROOTS.has(root);
}
