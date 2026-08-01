import { dayKey } from "./layout";
import type { CalendarEvent, EventAlert, EventStatus, RecurrenceRule } from "./types";

export type RecurrencePreset = "none" | "daily" | "weekly" | "monthly" | "yearly" | "custom";

export interface EventFormValues {
  title: string;
  calendarId: string;
  allDay: boolean;
  startDate: string;
  startTime: string;
  endDate: string;
  endTime: string;
  location: string;
  description: string;
  status: EventStatus;
  recurrence: RecurrenceRule | null;
  alertMinutes: number | null;
}

export interface EventPayload {
  calendarId: string;
  title: string;
  start: string;
  timeZone: string;
  duration: string;
  showWithoutTime: boolean;
  location: string | null;
  description: string | null;
  status: EventStatus;
  recurrenceRule: RecurrenceRule | null;
  alerts: EventAlert[];
}

export const WEEKDAY_IDS = ["su", "mo", "tu", "we", "th", "fr", "sa"] as const;

export const WEEKDAYS: { id: string; label: string }[] = [
  { id: "mo", label: "Mon" },
  { id: "tu", label: "Tue" },
  { id: "we", label: "Wed" },
  { id: "th", label: "Thu" },
  { id: "fr", label: "Fri" },
  { id: "sa", label: "Sat" },
  { id: "su", label: "Sun" },
];

export const ALERT_OPTIONS: { value: number | null; label: string }[] = [
  { value: null, label: "No reminder" },
  { value: 5, label: "5 minutes before" },
  { value: 10, label: "10 minutes before" },
  { value: 30, label: "30 minutes before" },
  { value: 60, label: "1 hour before" },
];

const DURATION_PATTERN = /^P(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/;
const NAIVE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/;
const DATE_PATTERN = /^\d{4}-\d{2}-\d{2}$/;
const TIME_PATTERN = /^\d{2}:\d{2}$/;

export function deviceTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}

export function parseDuration(value: string): number {
  const match = DURATION_PATTERN.exec(value.trim());
  if (!match) return 0;
  const [, weeks, days, hours, minutes, secondsPart] = match;
  return (
    Number(weeks ?? 0) * 10_080 +
    Number(days ?? 0) * 1440 +
    Number(hours ?? 0) * 60 +
    Number(minutes ?? 0) +
    Math.floor(Number(secondsPart ?? 0) / 60)
  );
}

export function formatDuration(minutes: number): string {
  const total = Math.max(0, Math.round(minutes));
  if (total === 0) return "PT0S";
  const hours = Math.floor(total / 60);
  const rest = total % 60;
  return `PT${hours > 0 ? `${hours}H` : ""}${rest > 0 ? `${rest}M` : ""}`;
}

export function splitNaive(value: string): { date: string; time: string } {
  const match = NAIVE_PATTERN.exec(value);
  if (!match) return { date: "", time: "00:00" };
  const [, year, month, day, hour, minute] = match;
  return { date: `${year}-${month}-${day}`, time: `${hour}:${minute}` };
}

export function joinNaive(date: string, time: string): string {
  return `${date}T${time}:00`;
}

function naiveToUtcMs(value: string): number {
  const match = NAIVE_PATTERN.exec(value.length === 10 ? `${value}T00:00` : value);
  if (!match) return Number.NaN;
  const [, year, month, day, hour, minute] = match;
  return Date.UTC(Number(year), Number(month) - 1, Number(day), Number(hour), Number(minute));
}

function naiveFromUtcMs(ms: number): string {
  const date = new Date(ms);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}T${pad(
    date.getUTCHours(),
  )}:${pad(date.getUTCMinutes())}:00`;
}

export function addNaiveMinutes(value: string, minutes: number): string {
  return naiveFromUtcMs(naiveToUtcMs(value) + minutes * 60_000);
}

export function naiveDiffMinutes(from: string, to: string): number {
  return Math.round((naiveToUtcMs(to) - naiveToUtcMs(from)) / 60_000);
}

export function naiveDiffDays(from: string, to: string): number {
  return Math.round((naiveToUtcMs(to.slice(0, 10)) - naiveToUtcMs(from.slice(0, 10))) / 86_400_000);
}

export function naiveFromDate(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${dayKey(date)}T${pad(date.getHours())}:${pad(date.getMinutes())}:00`;
}

export function dateFromNaive(value: string): Date {
  const match = NAIVE_PATTERN.exec(value.length === 10 ? `${value}T00:00` : value);
  if (!match) return new Date(Number.NaN);
  const [, year, month, day, hour, minute] = match;
  return new Date(Number(year), Number(month) - 1, Number(day), Number(hour), Number(minute));
}

export function weekdayIdFromDate(date: Date): string {
  return WEEKDAY_IDS[date.getDay()] ?? "mo";
}

export function eventToForm(event: CalendarEvent): EventFormValues {
  const start = splitNaive(event.start);
  const minutes = parseDuration(event.duration);
  let endDate = start.date;
  let endTime = start.time;
  if (event.showWithoutTime) {
    const days = Math.max(1, Math.round(minutes / 1440));
    endDate = splitNaive(addNaiveMinutes(joinNaive(start.date, "00:00"), (days - 1) * 1440)).date;
  } else {
    const end = splitNaive(addNaiveMinutes(joinNaive(start.date, start.time), minutes));
    endDate = end.date;
    endTime = end.time;
  }
  return {
    title: event.title,
    calendarId: event.calendarId,
    allDay: event.showWithoutTime,
    startDate: start.date,
    startTime: start.time,
    endDate,
    endTime,
    location: event.location ?? "",
    description: event.description ?? "",
    status: event.status ?? "confirmed",
    recurrence: event.recurrenceRule,
    alertMinutes: event.alerts[0]?.minutesBefore ?? null,
  };
}

export function formToPayload(values: EventFormValues, timeZone: string): EventPayload {
  const start = values.allDay
    ? joinNaive(values.startDate, "00:00")
    : joinNaive(values.startDate, values.startTime);
  const duration = values.allDay
    ? `P${Math.max(1, naiveDiffDays(values.startDate, values.endDate) + 1)}D`
    : formatDuration(naiveDiffMinutes(start, joinNaive(values.endDate, values.endTime)));
  return {
    calendarId: values.calendarId,
    title: values.title.trim(),
    start,
    timeZone,
    duration,
    showWithoutTime: values.allDay,
    location: values.location.trim() || null,
    description: values.description.trim() || null,
    status: values.status,
    recurrenceRule: values.recurrence,
    alerts: values.alertMinutes === null ? [] : [{ minutesBefore: values.alertMinutes }],
  };
}

export function validateEventForm(values: EventFormValues): string | null {
  if (!values.title.trim()) return "Title is required";
  if (!values.calendarId) return "Pick a calendar";
  if (!DATE_PATTERN.test(values.startDate) || !DATE_PATTERN.test(values.endDate)) return "Enter a valid date";
  if (values.allDay) {
    return naiveDiffDays(values.startDate, values.endDate) < 0
      ? "End date must not be before the start date"
      : null;
  }
  if (!TIME_PATTERN.test(values.startTime) || !TIME_PATTERN.test(values.endTime)) return "Enter a valid time";
  const minutes = naiveDiffMinutes(
    joinNaive(values.startDate, values.startTime),
    joinNaive(values.endDate, values.endTime),
  );
  return minutes <= 0 ? "End must be after the start" : null;
}

const FREQUENCY_LABEL: Record<RecurrenceRule["frequency"], { one: string; many: string }> = {
  daily: { one: "Daily", many: "days" },
  weekly: { one: "Weekly", many: "weeks" },
  monthly: { one: "Monthly", many: "months" },
  yearly: { one: "Yearly", many: "years" },
};

export function recurrenceSummary(rule: RecurrenceRule | null): string {
  if (!rule) return "Does not repeat";
  const label = FREQUENCY_LABEL[rule.frequency];
  let text = rule.interval > 1 ? `Every ${rule.interval} ${label.many}` : label.one;
  if (rule.frequency === "weekly" && rule.byDay.length > 0) {
    const days = WEEKDAYS.filter((day) => rule.byDay.includes(day.id)).map((day) => day.label);
    if (days.length > 0) text += ` on ${days.join(", ")}`;
  }
  if (rule.count !== null) text += ` · ${rule.count} times`;
  else if (rule.until) text += ` · until ${rule.until.slice(0, 10)}`;
  return text;
}

export function presetFromRule(rule: RecurrenceRule | null): RecurrencePreset {
  if (!rule) return "none";
  if (rule.interval !== 1 || rule.count !== null || rule.until) return "custom";
  if (rule.frequency === "weekly") return rule.byDay.length <= 1 ? "weekly" : "custom";
  return rule.byDay.length === 0 ? rule.frequency : "custom";
}

export function ruleForPreset(preset: RecurrencePreset, startDate: string): RecurrenceRule | null {
  if (preset === "none") return null;
  const frequency = preset === "custom" ? "weekly" : preset;
  return {
    frequency,
    interval: 1,
    count: null,
    until: null,
    byDay: frequency === "weekly" ? [weekdayIdFromDate(dateFromNaive(startDate))] : [],
  };
}

export function naiveAtMinutes(day: Date, minutes: number): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  const clamped = Math.max(0, Math.min(1439, Math.round(minutes)));
  return `${dayKey(day)}T${pad(Math.floor(clamped / 60))}:${pad(clamped % 60)}:00`;
}

export function defaultEventForm(
  calendarId: string,
  day: Date,
  startMinutes: number,
  endMinutes: number,
  allDay: boolean,
): EventFormValues {
  const date = dayKey(day);
  const pad = (value: number) => String(value).padStart(2, "0");
  const startNaive = `${date}T${pad(Math.floor(startMinutes / 60))}:${pad(startMinutes % 60)}:00`;
  const end = splitNaive(addNaiveMinutes(startNaive, Math.max(0, endMinutes - startMinutes)));
  const start = splitNaive(startNaive);
  return {
    title: "",
    calendarId,
    allDay,
    startDate: date,
    startTime: allDay ? "09:00" : start.time,
    endDate: allDay ? date : end.date,
    endTime: allDay ? "10:00" : end.time,
    location: "",
    description: "",
    status: "confirmed",
    recurrence: null,
    alertMinutes: null,
  };
}
