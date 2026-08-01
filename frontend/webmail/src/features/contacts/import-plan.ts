import type { ContactCard, LabeledValue } from "./types";

export interface ImportCandidate {
  uid: string | null;
  emails: LabeledValue[];
}

export interface ImportPlan<T extends ImportCandidate> {
  fresh: T[];
  duplicates: T[];
}

function emailKeys(emails: LabeledValue[] | null | undefined): string[] {
  return (emails ?? [])
    .map((entry) => entry.value.trim().toLocaleLowerCase())
    .filter((value) => value.length > 0);
}

export function planImport<T extends ImportCandidate>(
  candidates: T[],
  existing: ContactCard[],
): ImportPlan<T> {
  const uids = new Set<string>();
  const emails = new Set<string>();
  for (const card of existing) {
    const uid = card.uid?.trim().toLocaleLowerCase();
    if (uid) uids.add(uid);
    for (const key of emailKeys(card.emails)) emails.add(key);
  }

  const fresh: T[] = [];
  const duplicates: T[] = [];
  for (const candidate of candidates) {
    const uid = candidate.uid?.trim().toLocaleLowerCase() ?? "";
    const keys = emailKeys(candidate.emails);
    const seen = (uid && uids.has(uid)) || keys.some((key) => emails.has(key));
    if (seen) {
      duplicates.push(candidate);
      continue;
    }
    fresh.push(candidate);
    if (uid) uids.add(uid);
    for (const key of keys) emails.add(key);
  }
  return { fresh, duplicates };
}
