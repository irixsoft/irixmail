export interface SearchFilters {
  text: string;
  from: string;
  to: string;
  subject: string;
  body: string;
  mailboxId: string | null;
  tag: string | null;
  hasAttachment: boolean | null;
  unread: boolean | null;
  starred: boolean | null;
  after: string | null;
  before: string | null;
}

export const emptyFilters: SearchFilters = {
  text: "",
  from: "",
  to: "",
  subject: "",
  body: "",
  mailboxId: null,
  tag: null,
  hasAttachment: null,
  unread: null,
  starred: null,
  after: null,
  before: null,
};

type Condition = Record<string, unknown>;

export function buildEmailFilter(filters: SearchFilters): Condition {
  const conditions: Condition[] = [];
  if (filters.mailboxId) conditions.push({ inMailbox: filters.mailboxId });
  if (filters.text) conditions.push({ text: filters.text });
  if (filters.from) conditions.push({ from: filters.from });
  if (filters.to) conditions.push({ to: filters.to });
  if (filters.subject) conditions.push({ subject: filters.subject });
  if (filters.body) conditions.push({ body: filters.body });
  if (filters.hasAttachment != null) conditions.push({ hasAttachment: filters.hasAttachment });
  if (filters.unread != null) {
    conditions.push(filters.unread ? { notKeyword: "$seen" } : { hasKeyword: "$seen" });
  }
  if (filters.starred != null) {
    conditions.push(filters.starred ? { hasKeyword: "$flagged" } : { notKeyword: "$flagged" });
  }
  if (filters.tag) conditions.push({ hasKeyword: filters.tag });
  if (filters.after) conditions.push({ after: `${filters.after}T00:00:00Z` });
  if (filters.before) conditions.push({ before: `${filters.before}T23:59:59Z` });
  if (conditions.length === 0) return {};
  if (conditions.length === 1) return conditions[0]!;
  return { operator: "AND", conditions };
}

export function countActiveFilters(filters: SearchFilters): number {
  let count = 0;
  if (filters.from) count += 1;
  if (filters.to) count += 1;
  if (filters.subject) count += 1;
  if (filters.body) count += 1;
  if (filters.tag) count += 1;
  if (filters.hasAttachment != null) count += 1;
  if (filters.unread != null) count += 1;
  if (filters.starred != null) count += 1;
  if (filters.after) count += 1;
  if (filters.before) count += 1;
  return count;
}
