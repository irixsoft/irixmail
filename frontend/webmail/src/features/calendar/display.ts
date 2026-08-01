import { addDays, startOfDay, startOfWeek } from "./layout";
import type { CalendarView, EventInstance } from "./types";

const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const hourOnly = new Intl.DateTimeFormat(undefined, { hour: "numeric" });
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const monthYear = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric" });
const dayMonth = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
const fullDay = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" });
const agendaDay = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" });

export function formatTime(date: Date): string {
  return time.format(date);
}

export function formatHourLabel(hour: number): string {
  return hourOnly.format(new Date(2026, 0, 1, hour));
}

export function formatWeekday(date: Date): string {
  return weekday.format(date);
}

export function formatAgendaDay(date: Date): string {
  return agendaDay.format(date);
}

export function instanceStart(instance: EventInstance): Date {
  return new Date(instance.occurrence.start * 1000);
}

export function instanceEnd(instance: EventInstance): Date {
  return new Date(instance.occurrence.end * 1000);
}

export function formatInstanceRange(instance: EventInstance): string {
  if (instance.event.showWithoutTime) return "All day";
  return `${formatTime(instanceStart(instance))} – ${formatTime(instanceEnd(instance))}`;
}

export function formatInstanceDetail(instance: EventInstance): string {
  const start = instanceStart(instance);
  const end = instanceEnd(instance);
  if (instance.event.showWithoutTime) {
    const last = addDays(startOfDay(end), -1);
    const span = startOfDay(start).getTime() === last.getTime() ? fullDay.format(start) : `${dayMonth.format(start)} – ${dayMonth.format(last)}`;
    return `${span} · all day`;
  }
  if (startOfDay(start).getTime() === startOfDay(end).getTime()) {
    return `${fullDay.format(start)} · ${formatTime(start)} – ${formatTime(end)}`;
  }
  return `${dayMonth.format(start)} ${formatTime(start)} – ${dayMonth.format(end)} ${formatTime(end)}`;
}

export function periodLabel(view: CalendarView, anchor: Date): string {
  if (view === "month") return monthYear.format(anchor);
  if (view === "day") return `${fullDay.format(anchor)} ${anchor.getFullYear()}`;
  const first = view === "week" ? startOfWeek(anchor) : startOfDay(anchor);
  const last = addDays(first, view === "week" ? 6 : 29);
  return `${dayMonth.format(first)} – ${dayMonth.format(last)} ${last.getFullYear()}`;
}
