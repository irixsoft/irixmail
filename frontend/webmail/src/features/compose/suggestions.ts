import type { ContactCard } from "@/features/contacts/types";
import type { EmailAddress } from "@/lib/mail-types";

export interface Suggestion {
  id: string;
  kind: "individual" | "group";
  name: string;
  email: string;
  memberCount: number;
  addresses: EmailAddress[];
}

const NAME_PARTS = ["prefix", "given", "additional", "surname", "suffix"] as const;

const DEFAULT_LIMIT = 8;

function emailsOf(card: ContactCard): string[] {
  return (card.emails ?? []).map((entry) => entry.value?.trim() ?? "").filter(Boolean);
}

function displayNameOf(card: ContactCard): string {
  const full = card.fullName?.trim();
  if (full) return full;
  const joined = NAME_PARTS.map((part) => card.name?.[part]?.trim() ?? "")
    .filter(Boolean)
    .join(" ");
  if (joined) return joined;
  return emailsOf(card)[0] ?? "";
}

function toAddress(name: string, email: string): EmailAddress {
  return name ? { name, email } : { email };
}

function rankOf(name: string, organization: string, email: string, query: string): number | null {
  const lowerName = name.toLowerCase();
  const lowerOrg = organization.toLowerCase();
  const lowerEmail = email.toLowerCase();
  if (lowerName.startsWith(query)) return 0;
  if (lowerEmail && (lowerEmail.startsWith(query) || lowerEmail.split("@")[0]!.startsWith(query))) {
    return 1;
  }
  if (lowerName.split(/\s+/).some((word) => word && word.startsWith(query))) return 2;
  if (lowerOrg && lowerOrg.startsWith(query)) return 2;
  if (lowerName.includes(query) || lowerOrg.includes(query) || lowerEmail.includes(query)) return 3;
  return null;
}

interface Ranked {
  rank: number;
  suggestion: Suggestion;
}

function groupSuggestion(
  card: ContactCard,
  byId: Map<string, ContactCard>,
  excluded: Set<string>,
): Suggestion | null {
  const name = displayNameOf(card);
  const seen = new Set<string>();
  const addresses: EmailAddress[] = [];
  let memberCount = 0;
  for (const memberId of card.members ?? []) {
    const member = byId.get(memberId);
    if (!member) continue;
    const email = emailsOf(member)[0];
    if (!email) continue;
    memberCount += 1;
    const key = email.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    if (excluded.has(key)) continue;
    addresses.push(toAddress(displayNameOf(member), email));
  }
  if (addresses.length === 0) return null;
  return { id: `group:${card.id}`, kind: "group", name, email: "", memberCount, addresses };
}

export function rankSuggestions(
  cards: ContactCard[],
  query: string,
  exclude: string[] = [],
  limit: number = DEFAULT_LIMIT,
): Suggestion[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];

  const excluded = new Set(exclude.map((entry) => entry.trim().toLowerCase()).filter(Boolean));
  const byId = new Map(cards.map((card) => [card.id, card]));
  const ranked: Ranked[] = [];

  for (const card of cards) {
    const name = displayNameOf(card);
    if (card.kind === "group") {
      const rank = rankOf(name, "", "", needle);
      if (rank === null) continue;
      const suggestion = groupSuggestion(card, byId, excluded);
      if (suggestion) ranked.push({ rank, suggestion });
      continue;
    }
    const organization = card.organization?.trim() ?? "";
    for (const email of emailsOf(card)) {
      if (excluded.has(email.toLowerCase())) continue;
      const rank = rankOf(name, organization, email, needle);
      if (rank === null) continue;
      ranked.push({
        rank,
        suggestion: {
          id: `${card.id}:${email.toLowerCase()}`,
          kind: "individual",
          name,
          email,
          memberCount: 0,
          addresses: [toAddress(name, email)],
        },
      });
    }
  }

  ranked.sort((left, right) => {
    if (left.rank !== right.rank) return left.rank - right.rank;
    const byName = left.suggestion.name.localeCompare(right.suggestion.name, undefined, {
      sensitivity: "base",
    });
    if (byName !== 0) return byName;
    const leftEmail = left.suggestion.email.toLowerCase();
    const rightEmail = right.suggestion.email.toLowerCase();
    return leftEmail < rightEmail ? -1 : leftEmail > rightEmail ? 1 : 0;
  });

  return ranked.slice(0, Math.max(0, limit)).map((entry) => entry.suggestion);
}

export function nextHighlight(current: number, delta: number, count: number): number {
  if (count <= 0) return -1;
  if (current < 0 || current >= count) return delta > 0 ? 0 : count - 1;
  return (current + delta + count) % count;
}
