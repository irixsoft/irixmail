import { beforeEach, describe, expect, it } from "vitest";

import {
  anchorFromParam,
  anchorToParam,
  loadCalendarView,
  loadHiddenCalendars,
  saveCalendarView,
  saveHiddenCalendars,
  toggleHidden,
  viewFromParam,
} from "./calendar-store";

beforeEach(() => {
  localStorage.clear();
});

describe("view persistence", () => {
  it("falls back when nothing is stored", () => {
    expect(loadCalendarView("month")).toBe("month");
    expect(loadCalendarView("agenda")).toBe("agenda");
  });

  it("round-trips a stored view and rejects junk", () => {
    saveCalendarView("week");
    expect(loadCalendarView("month")).toBe("week");
    localStorage.setItem("irixmail.webmail.calendar.view", "sideways");
    expect(loadCalendarView("month")).toBe("month");
  });
});

describe("viewFromParam", () => {
  it("accepts known views only", () => {
    expect(viewFromParam("day", "month")).toBe("day");
    expect(viewFromParam(null, "agenda")).toBe("agenda");
    expect(viewFromParam("spiral", "month")).toBe("month");
  });
});

describe("anchor params", () => {
  it("parses a day key into a local date", () => {
    const fallback = new Date(2026, 0, 1);
    const parsed = anchorFromParam("2026-07-31", fallback);
    expect(parsed.getFullYear()).toBe(2026);
    expect(parsed.getMonth()).toBe(6);
    expect(parsed.getDate()).toBe(31);
    expect(parsed.getHours()).toBe(0);
  });

  it("falls back on junk", () => {
    const fallback = new Date(2026, 0, 1);
    expect(anchorFromParam(null, fallback).getTime()).toBe(fallback.getTime());
    expect(anchorFromParam("last tuesday", fallback).getTime()).toBe(fallback.getTime());
    expect(anchorFromParam("2026-13-40", fallback).getTime()).toBe(fallback.getTime());
  });

  it("serialises back to a day key", () => {
    expect(anchorToParam(new Date(2026, 6, 31, 18))).toBe("2026-07-31");
  });
});

describe("hidden calendars", () => {
  it("defaults to nothing hidden", () => {
    expect(loadHiddenCalendars()).toEqual([]);
  });

  it("round-trips and ignores malformed storage", () => {
    saveHiddenCalendars(["a", "b"]);
    expect(loadHiddenCalendars()).toEqual(["a", "b"]);
    localStorage.setItem("irixmail.webmail.calendar.hidden", "{oops");
    expect(loadHiddenCalendars()).toEqual([]);
    localStorage.setItem("irixmail.webmail.calendar.hidden", JSON.stringify([1, "b", null]));
    expect(loadHiddenCalendars()).toEqual(["b"]);
  });

  it("toggles membership", () => {
    expect(toggleHidden([], "a")).toEqual(["a"]);
    expect(toggleHidden(["a", "b"], "a")).toEqual(["b"]);
  });
});
