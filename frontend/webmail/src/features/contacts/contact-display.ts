import type { ContactCard } from "./types";

const NAME_ORDER = ["prefix", "given", "additional", "surname", "suffix"] as const;

function nameFromParts(card: ContactCard): string {
  const name = card.name;
  if (!name) return "";
  return NAME_ORDER.map((part) => name[part]?.trim() ?? "")
    .filter(Boolean)
    .join(" ");
}

export function primaryEmail(card: ContactCard): string {
  for (const entry of card.emails ?? []) {
    const value = entry?.value?.trim();
    if (value) return value;
  }
  return "";
}

function rawName(card: ContactCard): string {
  return (
    card.fullName?.trim() ||
    nameFromParts(card) ||
    primaryEmail(card) ||
    card.organization?.trim() ||
    ""
  );
}

export function displayName(card: ContactCard): string {
  return rawName(card) || "No name";
}

export function contactInitials(card: ContactCard): string {
  const named = card.fullName?.trim() || nameFromParts(card);
  const source = named || primaryEmail(card);
  const letters = source
    .split(/[\s.@_-]+/)
    .map((word) => word.match(/\p{L}|\p{N}/u)?.[0] ?? "")
    .filter(Boolean);
  if (letters.length === 0) return "?";
  return letters.slice(0, named ? 2 : 1).join("").toUpperCase();
}

export function sortKey(card: ContactCard): string {
  return displayName(card).trim().toLocaleLowerCase();
}

export function sectionLetter(card: ContactCard): string {
  const first = rawName(card).trim().normalize("NFD").replace(/\p{M}/gu, "")[0] ?? "";
  return /\p{L}/u.test(first) ? first.toLocaleUpperCase() : "#";
}

export function matchesContact(card: ContactCard, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  if (displayName(card).toLocaleLowerCase().includes(needle)) return true;
  if ((card.organization ?? "").toLocaleLowerCase().includes(needle)) return true;
  if ((card.nickname ?? "").toLocaleLowerCase().includes(needle)) return true;
  return (card.emails ?? []).some((entry) => entry.value.toLocaleLowerCase().includes(needle));
}

export function sortContacts(cards: ContactCard[]): ContactCard[] {
  return [...cards].sort((a, b) =>
    sortKey(a).localeCompare(sortKey(b), undefined, { sensitivity: "base" }),
  );
}

export function filterContacts(cards: ContactCard[], query: string): ContactCard[] {
  return sortContacts(cards.filter((card) => matchesContact(card, query)));
}

export interface ContactSection {
  letter: string;
  contacts: ContactCard[];
}

export function groupBySection(cards: ContactCard[]): ContactSection[] {
  const sections: ContactSection[] = [];
  const letters = new Map<string, ContactSection>();
  for (const card of sortContacts(cards)) {
    const letter = sectionLetter(card);
    let section = letters.get(letter);
    if (!section) {
      section = { letter, contacts: [] };
      letters.set(letter, section);
      sections.push(section);
    }
    section.contacts.push(card);
  }
  return sections.sort((a, b) => {
    if (a.letter === "#") return 1;
    if (b.letter === "#") return -1;
    return a.letter.localeCompare(b.letter);
  });
}

export function formatBirthday(value?: string | null): string {
  if (!value) return "—";
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return value;
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, { year: "numeric", month: "long", day: "numeric" }).format(date);
}
