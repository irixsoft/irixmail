import type { EventPayload } from "./event-form";
import { addNaiveMinutes, formatDuration, naiveDiffDays, naiveDiffMinutes, parseDuration } from "./event-form";
import type { CalendarEvent, EventStatus, RecurrenceRule } from "./types";

export interface ParsedIcsEvent {
  uid: string | null;
  payload: Omit<EventPayload, "calendarId">;
}

interface Line {
  name: string;
  params: Record<string, string>;
  value: string;
}

function unfold(text: string): string[] {
  const lines: string[] = [];
  for (const raw of text.split(/\r?\n/)) {
    if ((raw.startsWith(" ") || raw.startsWith("\t")) && lines.length > 0) {
      lines[lines.length - 1] += raw.slice(1);
    } else if (raw.length > 0) {
      lines.push(raw);
    }
  }
  return lines;
}

function parseLine(line: string): Line | null {
  let inQuotes = false;
  let colon = -1;
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if (char === '"') inQuotes = !inQuotes;
    else if (char === ":" && !inQuotes) {
      colon = index;
      break;
    }
  }
  if (colon < 1) return null;
  const head = line.slice(0, colon).split(";");
  const name = head[0]!.toUpperCase();
  const params: Record<string, string> = {};
  for (const part of head.slice(1)) {
    const eq = part.indexOf("=");
    if (eq > 0) {
      params[part.slice(0, eq).toUpperCase()] = part.slice(eq + 1).replace(/^"|"$/g, "");
    }
  }
  return { name, params, value: line.slice(colon + 1) };
}

function unescapeText(value: string): string {
  return value.replace(/\\(.)/g, (_, char: string) => {
    if (char === "n" || char === "N") return "\n";
    return char;
  });
}

function escapeText(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/;/g, "\\;").replace(/,/g, "\\,").replace(/\n/g, "\\n");
}

interface StampInfo {
  naive: string;
  dateOnly: boolean;
  utc: boolean;
}

function parseStamp(value: string): StampInfo | null {
  const date = /^(\d{4})(\d{2})(\d{2})$/.exec(value);
  if (date) {
    return { naive: `${date[1]}-${date[2]}-${date[3]}T00:00:00`, dateOnly: true, utc: false };
  }
  const timed = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})(Z?)$/.exec(value);
  if (!timed) return null;
  return {
    naive: `${timed[1]}-${timed[2]}-${timed[3]}T${timed[4]}:${timed[5]}:${timed[6]}`,
    dateOnly: false,
    utc: timed[7] === "Z",
  };
}

function parseRrule(value: string): RecurrenceRule | null {
  const parts: Record<string, string> = {};
  for (const pair of value.split(";")) {
    const eq = pair.indexOf("=");
    if (eq > 0) parts[pair.slice(0, eq).toUpperCase()] = pair.slice(eq + 1);
  }
  const frequency = { DAILY: "daily", WEEKLY: "weekly", MONTHLY: "monthly", YEARLY: "yearly" }[
    parts["FREQ"] ?? ""
  ] as RecurrenceRule["frequency"] | undefined;
  if (!frequency) return null;
  const until = parts["UNTIL"] ? (parseStamp(parts["UNTIL"])?.naive ?? null) : null;
  return {
    frequency,
    interval: Math.max(1, Number(parts["INTERVAL"] ?? 1) || 1),
    count: parts["COUNT"] ? Number(parts["COUNT"]) || null : null,
    until,
    byDay: (parts["BYDAY"] ?? "")
      .split(",")
      .map((token) => token.slice(-2).toLowerCase())
      .filter((token) => ["su", "mo", "tu", "we", "th", "fr", "sa"].includes(token)),
  };
}

function statusFromIcs(value: string): EventStatus {
  const lowered = value.trim().toLowerCase();
  return lowered === "tentative" || lowered === "cancelled" ? lowered : "confirmed";
}

export function parseIcs(text: string): ParsedIcsEvent[] {
  const events: ParsedIcsEvent[] = [];
  let current: Line[] | null = null;
  let alarmDepth = 0;
  let alarms: number[] = [];
  let pendingTrigger: number | null = null;

  for (const raw of unfold(text)) {
    const line = parseLine(raw);
    if (!line) continue;
    if (line.name === "BEGIN" && line.value.toUpperCase() === "VEVENT") {
      current = [];
      alarms = [];
      alarmDepth = 0;
      continue;
    }
    if (current === null) continue;
    if (line.name === "BEGIN" && line.value.toUpperCase() === "VALARM") {
      alarmDepth += 1;
      pendingTrigger = null;
      continue;
    }
    if (line.name === "END" && line.value.toUpperCase() === "VALARM") {
      alarmDepth = Math.max(0, alarmDepth - 1);
      if (pendingTrigger !== null) alarms.push(pendingTrigger);
      pendingTrigger = null;
      continue;
    }
    if (alarmDepth > 0) {
      if (line.name === "TRIGGER" && line.value.startsWith("-")) {
        const minutes = parseDuration(line.value.slice(1));
        if (minutes > 0) pendingTrigger = minutes;
      }
      continue;
    }
    if (line.name === "END" && line.value.toUpperCase() === "VEVENT") {
      const parsed = buildEvent(current, alarms);
      if (parsed) events.push(parsed);
      current = null;
      continue;
    }
    current.push(line);
  }
  return events;
}

function buildEvent(lines: Line[], alarms: number[]): ParsedIcsEvent | null {
  const find = (name: string) => lines.find((line) => line.name === name);
  if (find("RECURRENCE-ID")) return null;
  const startLine = find("DTSTART");
  if (!startLine) return null;
  const start = parseStamp(startLine.value);
  if (!start) return null;
  const dateOnly = start.dateOnly || startLine.params["VALUE"] === "DATE";
  const timeZone = dateOnly || start.utc ? "UTC" : (startLine.params["TZID"] ?? "UTC");

  let duration = dateOnly ? "P1D" : "PT1H";
  const durationLine = find("DURATION");
  const endLine = find("DTEND");
  if (durationLine && parseDuration(durationLine.value) > 0) {
    duration = durationLine.value.trim();
  } else if (endLine) {
    const end = parseStamp(endLine.value);
    if (end) {
      duration = dateOnly
        ? `P${Math.max(1, naiveDiffDays(start.naive, end.naive))}D`
        : formatDuration(Math.max(1, naiveDiffMinutes(start.naive, end.naive)));
    }
  }

  return {
    uid: find("UID")?.value.trim() || null,
    payload: {
      title: unescapeText(find("SUMMARY")?.value ?? ""),
      start: start.naive,
      timeZone,
      duration,
      showWithoutTime: dateOnly,
      location: find("LOCATION") ? unescapeText(find("LOCATION")!.value) : null,
      description: find("DESCRIPTION") ? unescapeText(find("DESCRIPTION")!.value) : null,
      status: statusFromIcs(find("STATUS")?.value ?? ""),
      recurrenceRule: find("RRULE") ? parseRrule(find("RRULE")!.value) : null,
      alerts: alarms.map((minutesBefore) => ({ minutesBefore })),
    },
  };
}

export function planIcsImport(
  parsed: ParsedIcsEvent[],
  existingUids: Iterable<string>,
): { fresh: ParsedIcsEvent[]; duplicates: number } {
  const known = new Set(existingUids);
  const fresh: ParsedIcsEvent[] = [];
  let duplicates = 0;
  for (const event of parsed) {
    if (event.uid && known.has(event.uid)) duplicates += 1;
    else fresh.push(event);
  }
  return { fresh, duplicates };
}

function toBasic(naive: string): string {
  return naive.replace(/[-:]/g, "");
}

function fold(line: string): string[] {
  if (line.length <= 75) return [line];
  const folded = [line.slice(0, 74)];
  let rest = line.slice(74);
  while (rest.length > 73) {
    folded.push(` ${rest.slice(0, 73)}`);
    rest = rest.slice(73);
  }
  folded.push(` ${rest}`);
  return folded;
}

function rruleToIcs(rule: RecurrenceRule): string {
  const parts = [`FREQ=${rule.frequency.toUpperCase()}`];
  if (rule.interval > 1) parts.push(`INTERVAL=${rule.interval}`);
  if (rule.byDay.length > 0) parts.push(`BYDAY=${rule.byDay.map((day) => day.toUpperCase()).join(",")}`);
  if (rule.count) parts.push(`COUNT=${rule.count}`);
  if (rule.until) parts.push(`UNTIL=${toBasic(rule.until)}`);
  return parts.join(";");
}

export function generateIcs(events: CalendarEvent[], calendarName: string, dtstamp: string): string {
  const lines: string[] = [
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//IRIXMAIL//EN",
    `X-WR-CALNAME:${escapeText(calendarName)}`,
  ];
  for (const event of events) {
    lines.push("BEGIN:VEVENT");
    lines.push(`UID:${event.uid}`);
    lines.push(`DTSTAMP:${dtstamp}`);
    if (event.showWithoutTime) {
      const days = Math.max(1, Math.round(parseDuration(event.duration) / 1440));
      lines.push(`DTSTART;VALUE=DATE:${toBasic(event.start).slice(0, 8)}`);
      lines.push(`DTEND;VALUE=DATE:${toBasic(addNaiveMinutes(event.start, days * 1440)).slice(0, 8)}`);
    } else {
      const end = addNaiveMinutes(event.start, Math.max(1, parseDuration(event.duration)));
      const suffix = event.timeZone === "UTC" || event.timeZone === null ? "Z" : "";
      const tzid = suffix === "" ? `;TZID=${event.timeZone}` : "";
      lines.push(`DTSTART${tzid}:${toBasic(event.start)}${suffix}`);
      lines.push(`DTEND${tzid}:${toBasic(end)}${suffix}`);
    }
    lines.push(`SUMMARY:${escapeText(event.title)}`);
    if (event.location) lines.push(`LOCATION:${escapeText(event.location)}`);
    if (event.description) lines.push(`DESCRIPTION:${escapeText(event.description)}`);
    if (event.status) lines.push(`STATUS:${event.status.toUpperCase()}`);
    if (event.recurrenceRule) lines.push(`RRULE:${rruleToIcs(event.recurrenceRule)}`);
    for (const alert of event.alerts) {
      lines.push("BEGIN:VALARM", `TRIGGER:-PT${alert.minutesBefore}M`, "ACTION:DISPLAY", "END:VALARM");
    }
    lines.push("END:VEVENT");
  }
  lines.push("END:VCALENDAR");
  return `${lines.flatMap(fold).join("\r\n")}\r\n`;
}
