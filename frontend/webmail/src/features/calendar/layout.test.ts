import { describe, expect, it } from "vitest";

import {
  DAY_MINUTES,
  HOUR_HEIGHT,
  SNAP_MINUTES,
  addDays,
  addMonths,
  chunkIds,
  clipToDay,
  columnFromX,
  dayKey,
  dragCreateRange,
  dragMoveRange,
  dragResizeEnd,
  isSameDay,
  joinOccurrences,
  layoutDayEvents,
  monthGrid,
  shiftAnchor,
  slotMath,
  startOfDay,
  startOfWeek,
  toIsoUtc,
  visibleRange,
  weekSegments,
} from "./layout";
import type { CalendarEvent, Occurrence } from "./types";

const seconds = (year: number, month: number, day: number, hour = 0, minute = 0) =>
  Math.floor(new Date(year, month, day, hour, minute, 0, 0).getTime() / 1000);

describe("date primitives", () => {
  it("startOfDay strips the time", () => {
    const result = startOfDay(new Date(2026, 6, 31, 17, 42, 9, 500));
    expect(result.getFullYear()).toBe(2026);
    expect(result.getMonth()).toBe(6);
    expect(result.getDate()).toBe(31);
    expect(result.getHours()).toBe(0);
    expect(result.getMilliseconds()).toBe(0);
  });

  it("addDays crosses month and year boundaries", () => {
    const forward = addDays(new Date(2026, 11, 30), 3);
    expect(forward.getFullYear()).toBe(2027);
    expect(forward.getMonth()).toBe(0);
    expect(forward.getDate()).toBe(2);
    const back = addDays(new Date(2026, 2, 1), -1);
    expect(back.getMonth()).toBe(1);
    expect(back.getDate()).toBe(28);
  });

  it("addMonths clamps the day of month", () => {
    const result = addMonths(new Date(2026, 0, 31), 1);
    expect(result.getMonth()).toBe(1);
    expect(result.getDate()).toBe(28);
  });

  it("startOfWeek honours the week start", () => {
    const friday = new Date(2026, 6, 31);
    const monday = startOfWeek(friday, 1);
    expect(monday.getDate()).toBe(27);
    expect(monday.getDay()).toBe(1);
    const sunday = startOfWeek(friday, 0);
    expect(sunday.getDate()).toBe(26);
    expect(sunday.getDay()).toBe(0);
  });

  it("isSameDay compares calendar days only", () => {
    expect(isSameDay(new Date(2026, 6, 31, 1), new Date(2026, 6, 31, 23))).toBe(true);
    expect(isSameDay(new Date(2026, 6, 31), new Date(2026, 7, 1))).toBe(false);
  });

  it("dayKey formats a padded local key", () => {
    expect(dayKey(new Date(2026, 0, 5, 22))).toBe("2026-01-05");
  });

  it("toIsoUtc drops milliseconds", () => {
    expect(toIsoUtc(new Date(Date.UTC(2026, 1, 1, 0, 0, 0, 250)))).toBe("2026-02-01T00:00:00Z");
  });
});

describe("monthGrid", () => {
  it("returns six rows of seven days starting on the week start", () => {
    const grid = monthGrid(new Date(2026, 6, 15), 1);
    expect(grid).toHaveLength(6);
    for (const row of grid) expect(row).toHaveLength(7);
    expect(grid[0]![0]!.getDay()).toBe(1);
    expect(grid[0]![0]!.getDate()).toBe(29);
    expect(grid[0]![0]!.getMonth()).toBe(5);
  });

  it("covers the whole anchor month", () => {
    const grid = monthGrid(new Date(2026, 1, 10), 1);
    const flat = grid.flat();
    expect(flat).toHaveLength(42);
    expect(flat.some((day) => day.getMonth() === 1 && day.getDate() === 1)).toBe(true);
    expect(flat.some((day) => day.getMonth() === 1 && day.getDate() === 28)).toBe(true);
  });
});

describe("visibleRange", () => {
  it("pads the month to the full grid", () => {
    const [start, end] = visibleRange("month", new Date(2026, 6, 15), 1);
    const from = new Date(start);
    const to = new Date(end);
    expect(from.getDate()).toBe(29);
    expect(from.getMonth()).toBe(5);
    expect(Math.round((to.getTime() - from.getTime()) / 86_400_000)).toBe(42);
  });

  it("spans a week, a day and thirty agenda days", () => {
    const anchor = new Date(2026, 6, 31, 13);
    const span = (view: "week" | "day" | "agenda") => {
      const [start, end] = visibleRange(view, anchor, 1);
      return Math.round((new Date(end).getTime() - new Date(start).getTime()) / 86_400_000);
    };
    expect(span("week")).toBe(7);
    expect(span("day")).toBe(1);
    expect(span("agenda")).toBe(30);
    const [dayStart] = visibleRange("day", anchor, 1);
    expect(new Date(dayStart).getHours()).toBe(0);
  });

  it("emits utc instants without milliseconds", () => {
    const [start, end] = visibleRange("day", new Date(2026, 6, 31), 1);
    expect(start).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
    expect(end).toMatch(/Z$/);
  });
});

describe("shiftAnchor", () => {
  it("steps by the visible period", () => {
    const anchor = new Date(2026, 6, 31);
    expect(shiftAnchor("month", anchor, 1).getMonth()).toBe(7);
    expect(shiftAnchor("month", anchor, -1).getMonth()).toBe(5);
    expect(shiftAnchor("week", anchor, 1).getDate()).toBe(7);
    expect(shiftAnchor("day", anchor, -1).getDate()).toBe(30);
    expect(shiftAnchor("agenda", anchor, 1).getMonth()).toBe(7);
  });
});

describe("layoutDayEvents", () => {
  it("gives a single column to events that never overlap", () => {
    const result = layoutDayEvents([
      { start: 0, end: 60 },
      { start: 60, end: 120 },
    ]);
    expect(result).toEqual([
      { column: 0, columns: 1 },
      { column: 0, columns: 1 },
    ]);
  });

  it("splits two overlapping events into two columns", () => {
    const result = layoutDayEvents([
      { start: 0, end: 90 },
      { start: 60, end: 120 },
    ]);
    expect(result).toEqual([
      { column: 0, columns: 2 },
      { column: 1, columns: 2 },
    ]);
  });

  it("packs a three-way overlap and reuses freed columns", () => {
    const result = layoutDayEvents([
      { start: 0, end: 180 },
      { start: 30, end: 90 },
      { start: 60, end: 120 },
      { start: 200, end: 260 },
    ]);
    expect(result[0]).toEqual({ column: 0, columns: 3 });
    expect(result[1]).toEqual({ column: 1, columns: 3 });
    expect(result[2]).toEqual({ column: 2, columns: 3 });
    expect(result[3]).toEqual({ column: 0, columns: 1 });
  });

  it("keeps results aligned with the input order", () => {
    const result = layoutDayEvents([
      { start: 600, end: 660 },
      { start: 60, end: 180 },
      { start: 120, end: 200 },
    ]);
    expect(result).toHaveLength(3);
    expect(result[0]).toEqual({ column: 0, columns: 1 });
    expect(result[1]!.columns).toBe(2);
    expect(result[2]!.columns).toBe(2);
    expect(result[1]!.column).not.toBe(result[2]!.column);
  });

  it("treats zero-length events as colliding", () => {
    const result = layoutDayEvents([
      { start: 60, end: 60 },
      { start: 60, end: 120 },
    ]);
    expect(result[0]!.columns).toBe(2);
    expect(result[1]!.columns).toBe(2);
  });

  it("returns an empty layout for no events", () => {
    expect(layoutDayEvents([])).toEqual([]);
  });
});

describe("weekSegments", () => {
  const week = Array.from({ length: 7 }, (_, index) => new Date(2026, 6, 27 + index));

  it("maps a single day event to a one cell span", () => {
    const segments = weekSegments([{ start: seconds(2026, 6, 29), end: seconds(2026, 6, 30) }], week);
    expect(segments).toEqual([{ index: 0, startIndex: 2, span: 1, row: 0 }]);
  });

  it("spans multiple days and clips to the week", () => {
    const segments = weekSegments(
      [
        { start: seconds(2026, 6, 25), end: seconds(2026, 6, 29) },
        { start: seconds(2026, 7, 1), end: seconds(2026, 7, 5) },
      ],
      week,
    );
    expect(segments[0]).toMatchObject({ index: 0, startIndex: 0, span: 2 });
    expect(segments[1]).toMatchObject({ index: 1, startIndex: 5, span: 2 });
  });

  it("packs overlapping bars onto separate rows", () => {
    const segments = weekSegments(
      [
        { start: seconds(2026, 6, 27), end: seconds(2026, 6, 31) },
        { start: seconds(2026, 6, 28), end: seconds(2026, 6, 30) },
        { start: seconds(2026, 7, 1), end: seconds(2026, 7, 2) },
      ],
      week,
    );
    const rows = new Map(segments.map((segment) => [segment.index, segment.row]));
    expect(rows.get(0)).toBe(0);
    expect(rows.get(1)).toBe(1);
    expect(rows.get(2)).toBe(0);
  });

  it("skips events outside the week", () => {
    const segments = weekSegments([{ start: seconds(2026, 5, 1), end: seconds(2026, 5, 2) }], week);
    expect(segments).toEqual([]);
  });
});

describe("slotMath", () => {
  it("converts between minutes and pixels", () => {
    expect(slotMath.minutesToPx(60)).toBe(HOUR_HEIGHT);
    expect(slotMath.minutesToPx(30)).toBe(HOUR_HEIGHT / 2);
    expect(slotMath.pxToMinutes(HOUR_HEIGHT)).toBe(60);
  });

  it("snaps to the slot size and clamps to the day", () => {
    expect(slotMath.snapMinutes(67)).toBe(SNAP_MINUTES * 4);
    expect(slotMath.snapMinutes(7)).toBe(0);
    expect(slotMath.snapMinutes(8)).toBe(15);
    expect(slotMath.clampMinutes(-30)).toBe(0);
    expect(slotMath.clampMinutes(2000)).toBe(DAY_MINUTES);
  });
});

describe("drag geometry", () => {
  it("normalises an upward create drag", () => {
    expect(dragCreateRange(600, 480)).toEqual({ startMinutes: 480, endMinutes: 600 });
  });

  it("keeps a create drag at least one slot tall", () => {
    expect(dragCreateRange(600, 603)).toEqual({ startMinutes: 600, endMinutes: 615 });
  });

  it("keeps a create drag inside the day", () => {
    expect(dragCreateRange(1439, 1500)).toEqual({ startMinutes: 1425, endMinutes: DAY_MINUTES });
  });

  it("moves a range and clamps it to the day", () => {
    expect(dragMoveRange(600, 60, 30)).toEqual({ startMinutes: 630, endMinutes: 690 });
    expect(dragMoveRange(600, 60, -1000)).toEqual({ startMinutes: 0, endMinutes: 60 });
    expect(dragMoveRange(1380, 60, 120)).toEqual({ startMinutes: 1380, endMinutes: 1440 });
  });

  it("resizes the end below a one slot floor", () => {
    expect(dragResizeEnd(600, 500)).toBe(615);
    expect(dragResizeEnd(600, 703)).toBe(705);
    expect(dragResizeEnd(600, 3000)).toBe(DAY_MINUTES);
  });

  it("resolves a pointer x to a day column", () => {
    expect(columnFromX(10, 700, 7)).toBe(0);
    expect(columnFromX(350, 700, 7)).toBe(3);
    expect(columnFromX(-40, 700, 7)).toBe(0);
    expect(columnFromX(5000, 700, 7)).toBe(6);
    expect(columnFromX(10, 0, 7)).toBe(0);
  });
});

describe("clipToDay", () => {
  const day = new Date(2026, 6, 31);

  it("clips a span that overruns both ends", () => {
    const result = clipToDay({ start: seconds(2026, 6, 30, 22), end: seconds(2026, 7, 1, 3) }, day);
    expect(result).toEqual({ startMinutes: 0, endMinutes: DAY_MINUTES });
  });

  it("returns minute offsets inside the day", () => {
    const result = clipToDay({ start: seconds(2026, 6, 31, 9, 30), end: seconds(2026, 6, 31, 11) }, day);
    expect(result).toEqual({ startMinutes: 570, endMinutes: 660 });
  });

  it("returns null when the span misses the day", () => {
    expect(clipToDay({ start: seconds(2026, 7, 2), end: seconds(2026, 7, 3) }, day)).toBeNull();
  });
});

describe("chunkIds", () => {
  it("splits ids into fixed size chunks", () => {
    expect(chunkIds(["a", "b", "c", "d", "e"], 2)).toEqual([["a", "b"], ["c", "d"], ["e"]]);
    expect(chunkIds([], 2)).toEqual([]);
  });
});

describe("joinOccurrences", () => {
  const event = (id: string, title: string): CalendarEvent => ({
    id,
    calendarId: "cal1",
    uid: `${id}-uid`,
    title,
    description: null,
    location: null,
    start: "2026-07-31T09:00:00",
    timeZone: "Europe/Stockholm",
    duration: "PT1H",
    showWithoutTime: false,
    status: "confirmed",
    recurrenceRule: null,
    alerts: [],
  });

  it("joins occurrences with their event and sorts by start", () => {
    const occurrences: Occurrence[] = [
      { id: "e2", start: 200, end: 260 },
      { id: "e1", start: 100, end: 160 },
      { id: "e1", start: 300, end: 360 },
    ];
    const result = joinOccurrences(occurrences, [event("e1", "Standup"), event("e2", "Review")]);
    expect(result.map((instance) => instance.event.title)).toEqual(["Standup", "Review", "Standup"]);
    expect(result[0]!.key).toBe("e1:100");
  });

  it("drops occurrences without a matching event", () => {
    const result = joinOccurrences([{ id: "ghost", start: 1, end: 2 }], [event("e1", "Standup")]);
    expect(result).toEqual([]);
  });
});
