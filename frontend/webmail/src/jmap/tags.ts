export interface TagDefinition {
  id: string;
  label: string;
  color: string;
}

export interface TagColor {
  dot: string;
  bg: string;
  text: string;
}

const TAG_PREFIX = "$label:";
const STORAGE_KEY = "irixmail.webmail.tags";

export const TAG_PALETTE: Record<string, TagColor> = {
  amber: { dot: "bg-amber-500", bg: "bg-amber-500/15", text: "text-amber-700 dark:text-amber-300" },
  red: { dot: "bg-red-500", bg: "bg-red-500/15", text: "text-red-700 dark:text-red-300" },
  orange: { dot: "bg-orange-500", bg: "bg-orange-500/15", text: "text-orange-700 dark:text-orange-300" },
  green: { dot: "bg-green-600", bg: "bg-green-600/15", text: "text-green-700 dark:text-green-300" },
  teal: { dot: "bg-teal-600", bg: "bg-teal-600/15", text: "text-teal-700 dark:text-teal-300" },
  blue: { dot: "bg-blue-500", bg: "bg-blue-500/15", text: "text-blue-700 dark:text-blue-300" },
  violet: { dot: "bg-violet-500", bg: "bg-violet-500/15", text: "text-violet-700 dark:text-violet-300" },
  pink: { dot: "bg-pink-500", bg: "bg-pink-500/15", text: "text-pink-700 dark:text-pink-300" },
  slate: { dot: "bg-slate-500", bg: "bg-slate-500/15", text: "text-slate-700 dark:text-slate-300" },
};

export function tagKeyword(id: string): string {
  return `${TAG_PREFIX}${id}`;
}

export function isTagKeyword(keyword: string): boolean {
  return keyword.startsWith(TAG_PREFIX);
}

export function tagIdFromKeyword(keyword: string): string {
  return keyword.slice(TAG_PREFIX.length);
}

export function messageTagIds(keywords: Record<string, boolean>): string[] {
  return Object.keys(keywords).filter(isTagKeyword).map(tagIdFromKeyword).sort();
}

export function loadTagDefinitions(): TagDefinition[] {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (entry): entry is TagDefinition =>
        typeof entry === "object" &&
        entry !== null &&
        typeof (entry as TagDefinition).id === "string" &&
        typeof (entry as TagDefinition).label === "string" &&
        typeof (entry as TagDefinition).color === "string",
    );
  } catch {
    return [];
  }
}

export function saveTagDefinitions(definitions: TagDefinition[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(definitions));
}
