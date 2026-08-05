export interface FilterRule {
  id: string;
  name: string;
  field: "from" | "to" | "subject";
  operator: "contains" | "is";
  value: string;
  action: "fileinto" | "forward" | "discard" | "markRead";
  target: string;
}

export interface SieveScript {
  id: string;
  name: string;
  rules?: FilterRule[] | null;
  source?: string;
  isActive?: boolean;
}

export const FIELD_LABEL: Record<FilterRule["field"], string> = {
  from: "From",
  to: "To",
  subject: "Subject",
};

export const ACTION_LABEL: Record<FilterRule["action"], string> = {
  fileinto: "Move to folder",
  forward: "Forward to",
  discard: "Discard",
  markRead: "Mark as read",
};

export function emptyRule(id: string = crypto.randomUUID()): FilterRule {
  return {
    id,
    name: "",
    field: "from",
    operator: "contains",
    value: "",
    action: "fileinto",
    target: "",
  };
}

export function scriptRules(script: SieveScript | undefined): FilterRule[] | null {
  if (!script) return [];
  return script.rules ?? null;
}

export function isExternallyEdited(script: SieveScript | undefined): boolean {
  return Boolean(script) && scriptRules(script) === null;
}

export function ruleSummary(rule: FilterRule): string {
  const condition = `${FIELD_LABEL[rule.field]} ${rule.operator} “${rule.value}”`;
  const action = ACTION_LABEL[rule.action] + (rule.target ? ` ${rule.target}` : "");
  return `${condition} → ${action}`;
}

export function upsertRule(rules: FilterRule[], rule: FilterRule): FilterRule[] {
  const exists = rules.some((entry) => entry.id === rule.id);
  return exists ? rules.map((entry) => (entry.id === rule.id ? rule : entry)) : [...rules, rule];
}

export function removeRule(rules: FilterRule[], id: string): FilterRule[] {
  return rules.filter((entry) => entry.id !== id);
}

export function savePayload(
  accountId: string,
  scriptId: string | undefined,
  rules: FilterRule[],
): { accountId: string; update?: object; create?: object } {
  const body = { name: "filters", rules };
  if (scriptId) {
    return { accountId, update: { [scriptId]: body } };
  }
  return { accountId, create: { filters: body } };
}
