import { describe, expect, it } from "vitest";

import { generateIcs, parseIcs, planIcsImport } from "./ics";
import type { CalendarEvent } from "./types";

const SIMPLE = [
  "BEGIN:VCALENDAR",
  "VERSION:2.0",
  "PRODID:-//Test//EN",
  "BEGIN:VEVENT",
  "UID:one@example.com",
  "DTSTAMP:20260101T000000Z",
  "DTSTART:20260210T100000Z",
  "DTEND:20260210T113000Z",
  "SUMMARY:Team sync\\, weekly",
  "LOCATION:Room 4",
  "DESCRIPTION:Line one\\nLine two",
  "STATUS:TENTATIVE",
  "BEGIN:VALARM",
  "TRIGGER:-PT15M",
  "ACTION:DISPLAY",
  "END:VALARM",
  "END:VEVENT",
  "END:VCALENDAR",
].join("\r\n");

const FOLDED_TZID = [
  "BEGIN:VCALENDAR",
  "BEGIN:VEVENT",
  "UID:two@example.com",
  "DTSTART;TZID=Europe/Stockholm:20260315T090000",
  "DURATION:PT45M",
  "SUMMARY:A very long summary that has been",
  " folded across two lines",
  "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=8",
  "END:VEVENT",
  "END:VCALENDAR",
].join("\r\n");

const ALL_DAY = [
  "BEGIN:VCALENDAR",
  "BEGIN:VEVENT",
  "UID:three@example.com",
  "DTSTART;VALUE=DATE:20260401",
  "DTEND;VALUE=DATE:20260403",
  "SUMMARY:Offsite",
  "END:VEVENT",
  "END:VCALENDAR",
].join("\r\n");

describe("parseIcs", () => {
  it("parses a timed utc event with alarm and escaped text", () => {
    const events = parseIcs(SIMPLE);
    expect(events).toHaveLength(1);
    const parsed = events[0]!;
    expect(parsed.uid).toBe("one@example.com");
    expect(parsed.payload.title).toBe("Team sync, weekly");
    expect(parsed.payload.start).toBe("2026-02-10T10:00:00");
    expect(parsed.payload.timeZone).toBe("UTC");
    expect(parsed.payload.duration).toBe("PT1H30M");
    expect(parsed.payload.showWithoutTime).toBe(false);
    expect(parsed.payload.location).toBe("Room 4");
    expect(parsed.payload.description).toBe("Line one\nLine two");
    expect(parsed.payload.status).toBe("tentative");
    expect(parsed.payload.alerts).toEqual([{ minutesBefore: 15 }]);
  });

  it("unfolds lines and reads tzid duration and recurrence", () => {
    const parsed = parseIcs(FOLDED_TZID)[0]!;
    expect(parsed.payload.title).toBe("A very long summary that has beenfolded across two lines");
    expect(parsed.payload.start).toBe("2026-03-15T09:00:00");
    expect(parsed.payload.timeZone).toBe("Europe/Stockholm");
    expect(parsed.payload.duration).toBe("PT45M");
    expect(parsed.payload.recurrenceRule).toEqual({
      frequency: "weekly",
      interval: 2,
      count: 8,
      until: null,
      byDay: ["mo", "we"],
    });
  });

  it("parses an all day event with a day span duration", () => {
    const parsed = parseIcs(ALL_DAY)[0]!;
    expect(parsed.payload.showWithoutTime).toBe(true);
    expect(parsed.payload.start).toBe("2026-04-01T00:00:00");
    expect(parsed.payload.timeZone).toBe("UTC");
    expect(parsed.payload.duration).toBe("P2D");
  });

  it("skips vevents without dtstart and recurrence overrides", () => {
    const text = [
      "BEGIN:VCALENDAR",
      "BEGIN:VEVENT",
      "UID:bad@example.com",
      "SUMMARY:No start",
      "END:VEVENT",
      "BEGIN:VEVENT",
      "UID:one@example.com",
      "RECURRENCE-ID:20260210T100000Z",
      "DTSTART:20260210T100000Z",
      "SUMMARY:Override",
      "END:VEVENT",
      "END:VCALENDAR",
    ].join("\r\n");
    expect(parseIcs(text)).toEqual([]);
  });

  it("returns nothing for junk input", () => {
    expect(parseIcs("hello")).toEqual([]);
  });
});

describe("planIcsImport", () => {
  it("splits fresh events from duplicates by uid", () => {
    const parsed = parseIcs(SIMPLE).concat(parseIcs(ALL_DAY));
    const plan = planIcsImport(parsed, ["one@example.com"]);
    expect(plan.duplicates).toBe(1);
    expect(plan.fresh).toHaveLength(1);
    expect(plan.fresh[0]!.uid).toBe("three@example.com");
  });
});

describe("generateIcs", () => {
  const event: CalendarEvent = {
    id: "9",
    calendarId: "1",
    uid: "one@example.com",
    title: "Team sync, weekly",
    description: "Line one\nLine two",
    location: "Room 4",
    start: "2026-02-10T10:00:00",
    timeZone: "UTC",
    duration: "PT1H30M",
    showWithoutTime: false,
    status: "confirmed",
    recurrenceRule: { frequency: "weekly", interval: 2, count: 8, until: null, byDay: ["mo", "we"] },
    alerts: [{ minutesBefore: 15 }],
  };

  it("round trips through parseIcs", () => {
    const text = generateIcs([event], "Work", "20260731T120000Z");
    const parsed = parseIcs(text)[0]!;
    expect(parsed.uid).toBe("one@example.com");
    expect(parsed.payload.title).toBe(event.title);
    expect(parsed.payload.start).toBe(event.start);
    expect(parsed.payload.timeZone).toBe("UTC");
    expect(parsed.payload.duration).toBe("PT1H30M");
    expect(parsed.payload.description).toBe(event.description);
    expect(parsed.payload.recurrenceRule).toEqual(event.recurrenceRule);
    expect(parsed.payload.alerts).toEqual([{ minutesBefore: 15 }]);
  });

  it("writes crlf lines a calendar name and folds long lines", () => {
    const long = generateIcs(
      [{ ...event, title: "x".repeat(200), recurrenceRule: null, alerts: [] }],
      "Work",
      "20260731T120000Z",
    );
    expect(long).toContain("X-WR-CALNAME:Work\r\n");
    expect(long).toContain("BEGIN:VCALENDAR\r\n");
    expect(long.split("\r\n").every((line) => line.length <= 75)).toBe(true);
    const parsed = parseIcs(long)[0]!;
    expect(parsed.payload.title).toBe("x".repeat(200));
  });

  it("writes all day events as date values", () => {
    const allDay: CalendarEvent = {
      ...event,
      start: "2026-04-01T00:00:00",
      timeZone: null,
      duration: "P2D",
      showWithoutTime: true,
      recurrenceRule: null,
      alerts: [],
    };
    const text = generateIcs([allDay], "Home", "20260731T120000Z");
    expect(text).toContain("DTSTART;VALUE=DATE:20260401");
    expect(text).toContain("DTEND;VALUE=DATE:20260403");
    const parsed = parseIcs(text)[0]!;
    expect(parsed.payload.showWithoutTime).toBe(true);
    expect(parsed.payload.duration).toBe("P2D");
  });
});
