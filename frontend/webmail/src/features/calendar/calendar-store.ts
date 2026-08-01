import { dayKey } from "./layout";
import type { CalendarView } from "./types";

const VIEW_KEY = "irixmail.webmail.calendar.view";
const HIDDEN_KEY = "irixmail.webmail.calendar.hidden";

const VIEWS: CalendarView[] = ["month", "week", "day", "agenda"];

function isView(value: unknown): value is CalendarView {
  return typeof value === "string" && (VIEWS as string[]).includes(value);
}

export function viewFromParam(value: string | null, fallback: CalendarView): CalendarView {
  return isView(value) ? value : fallback;
}

export function loadCalendarView(fallback: CalendarView): CalendarView {
  return viewFromParam(localStorage.getItem(VIEW_KEY), fallback);
}

export function saveCalendarView(view: CalendarView) {
  localStorage.setItem(VIEW_KEY, view);
}

export function anchorFromParam(value: string | null, fallback: Date): Date {
  if (!value || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return fallback;
  const [year, month, day] = value.split("-").map(Number) as [number, number, number];
  const parsed = new Date(year, month - 1, day);
  if (Number.isNaN(parsed.getTime()) || parsed.getMonth() !== month - 1 || parsed.getDate() !== day) {
    return fallback;
  }
  return parsed;
}

export function anchorToParam(date: Date): string {
  return dayKey(date);
}

export function loadHiddenCalendars(): string[] {
  const raw = localStorage.getItem(HIDDEN_KEY);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((entry): entry is string => typeof entry === "string");
  } catch {
    return [];
  }
}

export function saveHiddenCalendars(ids: string[]) {
  localStorage.setItem(HIDDEN_KEY, JSON.stringify(ids));
}

export function toggleHidden(ids: string[], id: string): string[] {
  return ids.includes(id) ? ids.filter((entry) => entry !== id) : [...ids, id];
}
