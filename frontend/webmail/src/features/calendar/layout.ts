import type { CalendarEvent, CalendarView, EventInstance, Occurrence, TimeSpan } from "./types";

export const HOUR_HEIGHT = 48;
export const SNAP_MINUTES = 15;
export const DAY_MINUTES = 1440;
export const WEEK_STARTS_ON = 1;

const DAY_MS = 86_400_000;

export function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

export function addDays(date: Date, days: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days, date.getHours(), date.getMinutes());
}

export function addMonths(date: Date, months: number): Date {
  const target = new Date(date.getFullYear(), date.getMonth() + months, 1);
  const lastDay = new Date(target.getFullYear(), target.getMonth() + 1, 0).getDate();
  target.setDate(Math.min(date.getDate(), lastDay));
  return target;
}

export function startOfWeek(date: Date, weekStartsOn: number = WEEK_STARTS_ON): Date {
  const day = startOfDay(date);
  const offset = (day.getDay() - weekStartsOn + 7) % 7;
  return addDays(day, -offset);
}

export function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
  );
}

export function dayKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

export function toIsoUtc(date: Date): string {
  return date.toISOString().replace(/\.\d{3}Z$/, "Z");
}

export function monthGrid(anchor: Date, weekStartsOn: number = WEEK_STARTS_ON): Date[][] {
  const first = startOfWeek(new Date(anchor.getFullYear(), anchor.getMonth(), 1), weekStartsOn);
  return Array.from({ length: 6 }, (_, row) =>
    Array.from({ length: 7 }, (_, column) => addDays(first, row * 7 + column)),
  );
}

export function visibleRange(
  view: CalendarView,
  anchor: Date,
  weekStartsOn: number = WEEK_STARTS_ON,
): [string, string] {
  const day = startOfDay(anchor);
  if (view === "month") {
    const grid = monthGrid(anchor, weekStartsOn);
    const first = grid[0]![0]!;
    return [toIsoUtc(first), toIsoUtc(addDays(first, 42))];
  }
  if (view === "week") {
    const first = startOfWeek(anchor, weekStartsOn);
    return [toIsoUtc(first), toIsoUtc(addDays(first, 7))];
  }
  if (view === "agenda") return [toIsoUtc(day), toIsoUtc(addDays(day, 30))];
  return [toIsoUtc(day), toIsoUtc(addDays(day, 1))];
}

export function shiftAnchor(view: CalendarView, anchor: Date, direction: number): Date {
  if (view === "month") return addMonths(startOfDay(anchor), direction);
  if (view === "week") return addDays(startOfDay(anchor), direction * 7);
  if (view === "agenda") return addDays(startOfDay(anchor), direction * 30);
  return addDays(startOfDay(anchor), direction);
}

export function layoutDayEvents(events: TimeSpan[]): { column: number; columns: number }[] {
  const result: { column: number; columns: number }[] = events.map(() => ({ column: 0, columns: 1 }));
  const order = events
    .map((event, index) => ({ index, start: event.start, end: Math.max(event.end, event.start + 1) }))
    .sort((a, b) => a.start - b.start || b.end - a.end || a.index - b.index);

  let cluster: typeof order = [];
  let clusterEnd = Number.NEGATIVE_INFINITY;

  const flush = () => {
    if (cluster.length === 0) return;
    const columnEnds: number[] = [];
    const placed: { index: number; column: number }[] = [];
    for (const entry of cluster) {
      let column = columnEnds.findIndex((end) => end <= entry.start);
      if (column === -1) {
        column = columnEnds.length;
        columnEnds.push(entry.end);
      } else {
        columnEnds[column] = entry.end;
      }
      placed.push({ index: entry.index, column });
    }
    for (const entry of placed) result[entry.index] = { column: entry.column, columns: columnEnds.length };
    cluster = [];
    clusterEnd = Number.NEGATIVE_INFINITY;
  };

  for (const entry of order) {
    if (cluster.length > 0 && entry.start >= clusterEnd) flush();
    cluster.push(entry);
    clusterEnd = Math.max(clusterEnd, entry.end);
  }
  flush();

  return result;
}

export interface WeekSegment {
  index: number;
  startIndex: number;
  span: number;
  row: number;
}

export function weekSegments(events: TimeSpan[], weekDays: Date[]): WeekSegment[] {
  const bounds = weekDays.map((day) => {
    const start = startOfDay(day).getTime() / 1000;
    return { start, end: start + DAY_MS / 1000 };
  });

  const raw: { index: number; startIndex: number; span: number }[] = [];
  events.forEach((event, index) => {
    const end = Math.max(event.end, event.start + 1);
    let first = -1;
    let last = -1;
    bounds.forEach((day, dayIndex) => {
      if (day.start < end && day.end > event.start) {
        if (first === -1) first = dayIndex;
        last = dayIndex;
      }
    });
    if (first === -1) return;
    raw.push({ index, startIndex: first, span: last - first + 1 });
  });

  const packed = [...raw].sort((a, b) => a.startIndex - b.startIndex || b.span - a.span || a.index - b.index);
  const rows: { from: number; to: number }[][] = [];
  const rowByIndex = new Map<number, number>();
  for (const segment of packed) {
    const from = segment.startIndex;
    const to = segment.startIndex + segment.span;
    let row = rows.findIndex((slots) => slots.every((slot) => slot.to <= from || slot.from >= to));
    if (row === -1) {
      row = rows.length;
      rows.push([]);
    }
    rows[row]!.push({ from, to });
    rowByIndex.set(segment.index, row);
  }

  return raw.map((segment) => ({ ...segment, row: rowByIndex.get(segment.index) ?? 0 }));
}

function minutesToPx(minutes: number, hourHeight: number = HOUR_HEIGHT): number {
  return (minutes / 60) * hourHeight;
}

function pxToMinutes(px: number, hourHeight: number = HOUR_HEIGHT): number {
  return (px / hourHeight) * 60;
}

function snapMinutes(minutes: number, step: number = SNAP_MINUTES): number {
  return Math.round(minutes / step) * step;
}

function clampMinutes(minutes: number): number {
  return Math.min(DAY_MINUTES, Math.max(0, minutes));
}

export const slotMath = { minutesToPx, pxToMinutes, snapMinutes, clampMinutes };

export function dragCreateRange(
  anchorMinutes: number,
  currentMinutes: number,
): { startMinutes: number; endMinutes: number } {
  const a = snapMinutes(clampMinutes(anchorMinutes));
  const b = snapMinutes(clampMinutes(currentMinutes));
  let start = Math.min(a, b);
  let end = Math.max(a, b);
  if (end - start < SNAP_MINUTES) end = start + SNAP_MINUTES;
  if (end > DAY_MINUTES) {
    end = DAY_MINUTES;
    start = Math.min(start, end - SNAP_MINUTES);
  }
  return { startMinutes: start, endMinutes: end };
}

export function dragMoveRange(
  startMinutes: number,
  durationMinutes: number,
  deltaMinutes: number,
): { startMinutes: number; endMinutes: number } {
  const raw = snapMinutes(startMinutes + deltaMinutes);
  const start = Math.max(0, Math.min(raw, DAY_MINUTES - durationMinutes));
  return { startMinutes: start, endMinutes: start + durationMinutes };
}

export function dragResizeEnd(startMinutes: number, pointerMinutes: number): number {
  const snapped = snapMinutes(clampMinutes(pointerMinutes));
  return Math.min(DAY_MINUTES, Math.max(snapped, startMinutes + SNAP_MINUTES));
}

export function columnFromX(x: number, width: number, columns: number): number {
  if (width <= 0 || columns <= 0) return 0;
  return Math.min(columns - 1, Math.max(0, Math.floor(x / (width / columns))));
}

export function clipToDay(span: TimeSpan, day: Date): { startMinutes: number; endMinutes: number } | null {
  const dayStart = startOfDay(day).getTime() / 1000;
  const dayEnd = dayStart + DAY_MS / 1000;
  const end = Math.max(span.end, span.start + 1);
  if (span.start >= dayEnd || end <= dayStart) return null;
  return {
    startMinutes: Math.max(0, Math.round((span.start - dayStart) / 60)),
    endMinutes: Math.min(DAY_MINUTES, Math.round((end - dayStart) / 60)),
  };
}

export function chunkIds(ids: string[], size: number): string[][] {
  const chunks: string[][] = [];
  for (let index = 0; index < ids.length; index += size) chunks.push(ids.slice(index, index + size));
  return chunks;
}

export function joinOccurrences(occurrences: Occurrence[], events: CalendarEvent[]): EventInstance[] {
  const byId = new Map(events.map((event) => [event.id, event]));
  const instances: EventInstance[] = [];
  for (const occurrence of occurrences) {
    const event = byId.get(occurrence.id);
    if (!event) continue;
    instances.push({ key: `${occurrence.id}:${occurrence.start}`, occurrence, event });
  }
  instances.sort(
    (a, b) =>
      a.occurrence.start - b.occurrence.start ||
      b.occurrence.end - a.occurrence.end ||
      a.event.title.localeCompare(b.event.title),
  );
  return instances;
}

export function instanceSpan(instance: EventInstance): TimeSpan {
  return { start: instance.occurrence.start, end: instance.occurrence.end };
}

export function isMultiDay(instance: EventInstance): boolean {
  const start = new Date(instance.occurrence.start * 1000);
  const end = new Date(Math.max(instance.occurrence.end - 1, instance.occurrence.start) * 1000);
  return !isSameDay(start, end);
}

export function isBarInstance(instance: EventInstance): boolean {
  return instance.event.showWithoutTime || isMultiDay(instance);
}
