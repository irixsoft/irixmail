import { describe, expect, it } from "vitest";

import {
  addNaiveMinutes,
  dateFromNaive,
  defaultEventForm,
  eventToForm,
  formToPayload,
  formatDuration,
  joinNaive,
  naiveDiffDays,
  naiveAtMinutes,
  naiveDiffMinutes,
  naiveFromDate,
  parseDuration,
  presetFromRule,
  recurrenceSummary,
  ruleForPreset,
  splitNaive,
  validateEventForm,
  weekdayIdFromDate,
} from "./event-form";
import type { EventFormValues } from "./event-form";
import type { CalendarEvent } from "./types";

const base: CalendarEvent = {
  id: "e1",
  calendarId: "cal1",
  uid: "uid-1",
  title: "Design review",
  description: "Bring the mocks",
  location: "Studio",
  start: "2026-07-31T09:30:00",
  timeZone: "Europe/Stockholm",
  duration: "PT1H30M",
  showWithoutTime: false,
  status: "confirmed",
  recurrenceRule: null,
  alerts: [{ minutesBefore: 10 }],
};

describe("duration codecs", () => {
  it("parses iso durations into minutes", () => {
    expect(parseDuration("PT1H30M")).toBe(90);
    expect(parseDuration("PT45M")).toBe(45);
    expect(parseDuration("PT0S")).toBe(0);
    expect(parseDuration("P1D")).toBe(1440);
    expect(parseDuration("P2DT3H15M")).toBe(3075);
    expect(parseDuration("PT90S")).toBe(1);
    expect(parseDuration("nonsense")).toBe(0);
  });

  it("formats minutes back into iso durations", () => {
    expect(formatDuration(90)).toBe("PT1H30M");
    expect(formatDuration(60)).toBe("PT1H");
    expect(formatDuration(45)).toBe("PT45M");
    expect(formatDuration(0)).toBe("PT0S");
    expect(formatDuration(-10)).toBe("PT0S");
    expect(formatDuration(1500)).toBe("PT25H");
  });
});

describe("naive datetime helpers", () => {
  it("splits and rejoins naive stamps", () => {
    expect(splitNaive("2026-07-31T09:30:00")).toEqual({ date: "2026-07-31", time: "09:30" });
    expect(joinNaive("2026-07-31", "09:30")).toBe("2026-07-31T09:30:00");
  });

  it("falls back on a malformed stamp", () => {
    expect(splitNaive("garbage")).toEqual({ date: "", time: "00:00" });
  });

  it("adds minutes across midnight without timezone drift", () => {
    expect(addNaiveMinutes("2026-07-31T23:30:00", 60)).toBe("2026-08-01T00:30:00");
    expect(addNaiveMinutes("2026-03-29T01:00:00", 120)).toBe("2026-03-29T03:00:00");
    expect(addNaiveMinutes("2026-01-01T00:15:00", -30)).toBe("2025-12-31T23:45:00");
  });

  it("measures naive differences", () => {
    expect(naiveDiffMinutes("2026-07-31T09:00:00", "2026-07-31T10:30:00")).toBe(90);
    expect(naiveDiffDays("2026-07-31", "2026-08-02")).toBe(2);
    expect(naiveDiffDays("2026-07-31", "2026-07-31")).toBe(0);
  });

  it("builds a naive stamp from a day and a minute offset", () => {
    expect(naiveAtMinutes(new Date(2026, 6, 31), 570)).toBe("2026-07-31T09:30:00");
    expect(naiveAtMinutes(new Date(2026, 6, 31), -5)).toBe("2026-07-31T00:00:00");
    expect(naiveAtMinutes(new Date(2026, 6, 31), 5000)).toBe("2026-07-31T23:59:00");
  });

  it("round-trips local dates", () => {
    const date = new Date(2026, 6, 31, 14, 5);
    expect(naiveFromDate(date)).toBe("2026-07-31T14:05:00");
    const parsed = dateFromNaive("2026-07-31T14:05:00");
    expect(parsed.getMonth()).toBe(6);
    expect(parsed.getDate()).toBe(31);
    expect(parsed.getHours()).toBe(14);
  });
});

describe("eventToForm", () => {
  it("derives an end from the duration", () => {
    const form = eventToForm(base);
    expect(form).toMatchObject({
      title: "Design review",
      calendarId: "cal1",
      allDay: false,
      startDate: "2026-07-31",
      startTime: "09:30",
      endDate: "2026-07-31",
      endTime: "11:00",
      location: "Studio",
      description: "Bring the mocks",
      status: "confirmed",
      alertMinutes: 10,
    });
    expect(form.recurrence).toBeNull();
  });

  it("treats an all-day duration as an inclusive end date", () => {
    const form = eventToForm({
      ...base,
      showWithoutTime: true,
      start: "2026-07-31T00:00:00",
      duration: "P3D",
      alerts: [],
      status: null,
    });
    expect(form.allDay).toBe(true);
    expect(form.startDate).toBe("2026-07-31");
    expect(form.endDate).toBe("2026-08-02");
    expect(form.alertMinutes).toBeNull();
    expect(form.status).toBe("confirmed");
  });
});

describe("formToPayload", () => {
  const values: EventFormValues = {
    title: "  Design review  ",
    calendarId: "cal1",
    allDay: false,
    startDate: "2026-07-31",
    startTime: "09:30",
    endDate: "2026-07-31",
    endTime: "11:00",
    location: "Studio",
    description: "",
    status: "confirmed",
    recurrence: null,
    alertMinutes: 10,
  };

  it("maps a timed event", () => {
    expect(formToPayload(values, "Europe/Stockholm")).toEqual({
      calendarId: "cal1",
      title: "Design review",
      start: "2026-07-31T09:30:00",
      timeZone: "Europe/Stockholm",
      duration: "PT1H30M",
      showWithoutTime: false,
      location: "Studio",
      description: null,
      status: "confirmed",
      recurrenceRule: null,
      alerts: [{ minutesBefore: 10 }],
    });
  });

  it("maps an all-day event to whole days", () => {
    const payload = formToPayload(
      { ...values, allDay: true, endDate: "2026-08-02", alertMinutes: null },
      "Europe/Stockholm",
    );
    expect(payload.start).toBe("2026-07-31T00:00:00");
    expect(payload.duration).toBe("P3D");
    expect(payload.showWithoutTime).toBe(true);
    expect(payload.alerts).toEqual([]);
  });

  it("uses a single day for a same-day all-day event", () => {
    const payload = formToPayload({ ...values, allDay: true }, "UTC");
    expect(payload.duration).toBe("P1D");
  });
});

describe("validateEventForm", () => {
  const values: EventFormValues = {
    title: "Standup",
    calendarId: "cal1",
    allDay: false,
    startDate: "2026-07-31",
    startTime: "09:30",
    endDate: "2026-07-31",
    endTime: "10:00",
    location: "",
    description: "",
    status: "confirmed",
    recurrence: null,
    alertMinutes: null,
  };

  it("accepts a well formed event", () => {
    expect(validateEventForm(values)).toBeNull();
  });

  it("requires a title", () => {
    expect(validateEventForm({ ...values, title: "   " })).toBe("Title is required");
  });

  it("requires a calendar", () => {
    expect(validateEventForm({ ...values, calendarId: "" })).toBe("Pick a calendar");
  });

  it("requires valid dates", () => {
    expect(validateEventForm({ ...values, startDate: "" })).toBe("Enter a valid date");
  });

  it("requires the end to follow the start", () => {
    expect(validateEventForm({ ...values, endTime: "09:30" })).toBe("End must be after the start");
    expect(validateEventForm({ ...values, allDay: true, endDate: "2026-07-30" })).toBe(
      "End date must not be before the start date",
    );
  });
});

describe("recurrence helpers", () => {
  it("summarises rules without locale dependence", () => {
    expect(recurrenceSummary(null)).toBe("Does not repeat");
    expect(recurrenceSummary({ frequency: "daily", interval: 1, count: null, until: null, byDay: [] })).toBe("Daily");
    expect(recurrenceSummary({ frequency: "daily", interval: 3, count: null, until: null, byDay: [] })).toBe(
      "Every 3 days",
    );
    expect(
      recurrenceSummary({ frequency: "weekly", interval: 2, count: null, until: null, byDay: ["mo", "we"] }),
    ).toBe("Every 2 weeks on Mon, Wed");
    expect(recurrenceSummary({ frequency: "monthly", interval: 1, count: 10, until: null, byDay: [] })).toBe(
      "Monthly · 10 times",
    );
    expect(
      recurrenceSummary({ frequency: "yearly", interval: 1, count: null, until: "2026-12-31T00:00:00", byDay: [] }),
    ).toBe("Yearly · until 2026-12-31");
  });

  it("detects the preset behind a rule", () => {
    expect(presetFromRule(null)).toBe("none");
    expect(presetFromRule({ frequency: "weekly", interval: 1, count: null, until: null, byDay: ["mo"] })).toBe(
      "weekly",
    );
    expect(presetFromRule({ frequency: "weekly", interval: 2, count: null, until: null, byDay: ["mo"] })).toBe(
      "custom",
    );
    expect(presetFromRule({ frequency: "weekly", interval: 1, count: 4, until: null, byDay: ["mo"] })).toBe("custom");
    expect(presetFromRule({ frequency: "weekly", interval: 1, count: null, until: null, byDay: ["mo", "tu"] })).toBe(
      "custom",
    );
  });

  it("builds a rule for a preset seeded from the start date", () => {
    expect(ruleForPreset("none", "2026-07-31")).toBeNull();
    expect(ruleForPreset("weekly", "2026-07-31")).toEqual({
      frequency: "weekly",
      interval: 1,
      count: null,
      until: null,
      byDay: ["fr"],
    });
    expect(ruleForPreset("monthly", "2026-07-31")).toEqual({
      frequency: "monthly",
      interval: 1,
      count: null,
      until: null,
      byDay: [],
    });
  });

  it("maps a date to a weekday id", () => {
    expect(weekdayIdFromDate(new Date(2026, 6, 31))).toBe("fr");
    expect(weekdayIdFromDate(new Date(2026, 7, 2))).toBe("su");
  });
});

describe("defaultEventForm", () => {
  it("seeds a timed slot", () => {
    const form = defaultEventForm("cal1", new Date(2026, 6, 31), 570, 660, false);
    expect(form).toMatchObject({
      calendarId: "cal1",
      allDay: false,
      startDate: "2026-07-31",
      startTime: "09:30",
      endDate: "2026-07-31",
      endTime: "11:00",
      title: "",
    });
  });

  it("seeds an all-day slot", () => {
    const form = defaultEventForm("cal1", new Date(2026, 6, 31), 0, 1440, true);
    expect(form.allDay).toBe(true);
    expect(form.startDate).toBe("2026-07-31");
    expect(form.endDate).toBe("2026-07-31");
  });

  it("wraps a slot that runs past midnight onto the next day", () => {
    const form = defaultEventForm("cal1", new Date(2026, 6, 31), 1410, 1440, false);
    expect(form.startTime).toBe("23:30");
    expect(form.endDate).toBe("2026-08-01");
    expect(form.endTime).toBe("00:00");
  });
});
