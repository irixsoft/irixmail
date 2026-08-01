import { tagKeyword } from "./tags";

export type EmailPatch = Record<string, unknown>;

export function seenPatch(seen: boolean): EmailPatch {
  return { "keywords/$seen": seen ? true : null };
}

export function flaggedPatch(flagged: boolean): EmailPatch {
  return { "keywords/$flagged": flagged ? true : null };
}

export function tagPatch(tagId: string, applied: boolean): EmailPatch {
  return { [`keywords/${tagKeyword(tagId)}`]: applied ? true : null };
}

export function movePatch(mailboxId: string): EmailPatch {
  return { mailboxIds: { [mailboxId]: true } };
}

export function updateMap(ids: string[], patch: EmailPatch): Record<string, EmailPatch> {
  return Object.fromEntries(ids.map((id) => [id, patch]));
}
