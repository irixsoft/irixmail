import type { EmailAddress } from "@/lib/mail-types";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const NAMED_RE = /^"?([^"<]*)"?\s*<([^>]+)>$/;

export function isValidEmail(email: string): boolean {
  return EMAIL_RE.test(email.trim());
}

export function parseRecipient(entry: string): EmailAddress | null {
  const trimmed = entry.trim().replace(/^[,;\s]+|[,;\s]+$/g, "");
  if (!trimmed) return null;
  const match = NAMED_RE.exec(trimmed);
  if (!match) return { email: trimmed };
  const name = match[1]?.trim();
  const email = (match[2] ?? trimmed).trim();
  return name ? { name, email } : { email };
}

export function parseRecipients(value: string): EmailAddress[] {
  return value
    .split(/[,;\n]+/)
    .map(parseRecipient)
    .filter((entry): entry is EmailAddress => entry !== null);
}

export function dedupeRecipients(list: EmailAddress[]): EmailAddress[] {
  const seen = new Set<string>();
  const result: EmailAddress[] = [];
  for (const entry of list) {
    const key = entry.email.trim().toLowerCase();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    result.push(entry);
  }
  return result;
}

export function recipientLabel(entry: EmailAddress): string {
  return entry.name?.trim() || entry.email;
}

export function recipientInitial(entry: EmailAddress): string {
  const source = entry.name?.trim() || entry.email;
  return (source[0] ?? "?").toUpperCase();
}

export function invalidRecipients(list: EmailAddress[]): EmailAddress[] {
  return list.filter((entry) => !isValidEmail(entry.email));
}

export function hasSeparator(value: string): boolean {
  return /[,;\n]/.test(value);
}
