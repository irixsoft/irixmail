export interface CalendarSwatch {
  id: string;
  hex: string;
  label: string;
}

export const CALENDAR_PALETTE: CalendarSwatch[] = [
  { id: "bronze", hex: "#b4763c", label: "Bronze" },
  { id: "clay", hex: "#c05f3f", label: "Clay" },
  { id: "gold", hex: "#c2a02c", label: "Gold" },
  { id: "olive", hex: "#7d8b47", label: "Olive" },
  { id: "teal", hex: "#3f8b85", label: "Teal" },
  { id: "indigo", hex: "#5a6ea8", label: "Indigo" },
  { id: "plum", hex: "#8b5a8c", label: "Plum" },
  { id: "stone", hex: "#77706a", label: "Stone" },
];

export function normalizeHex(value: string | null | undefined): string | null {
  if (!value) return null;
  const trimmed = value.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(trimmed)) return trimmed;
  if (/^#[0-9a-f]{3}$/.test(trimmed)) {
    const [, r, g, b] = trimmed.split("") as [string, string, string, string];
    return `#${r}${r}${g}${g}${b}${b}`;
  }
  return null;
}

export function calendarColor(calendar: { id: string; color: string | null }): string {
  const explicit = normalizeHex(calendar.color);
  if (explicit) return explicit;
  let hash = 0;
  for (let index = 0; index < calendar.id.length; index += 1) {
    hash = (hash * 31 + calendar.id.charCodeAt(index)) % 1_000_003;
  }
  return CALENDAR_PALETTE[hash % CALENDAR_PALETTE.length]!.hex;
}
