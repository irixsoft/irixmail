export type CalendarView = "month" | "week" | "day" | "agenda";

export type RecurrenceFrequency = "daily" | "weekly" | "monthly" | "yearly";

export type EventStatus = "confirmed" | "tentative" | "cancelled";

export interface CalendarSummary {
  id: string;
  name: string;
  color: string | null;
  sortOrder: number;
  timeZone: string | null;
  isDefault: boolean;
}

export interface RecurrenceRule {
  frequency: RecurrenceFrequency;
  interval: number;
  count: number | null;
  until: string | null;
  byDay: string[];
}

export interface EventAlert {
  minutesBefore: number;
}

export interface CalendarEvent {
  id: string;
  calendarId: string;
  uid: string;
  title: string;
  description: string | null;
  location: string | null;
  start: string;
  timeZone: string | null;
  duration: string;
  showWithoutTime: boolean;
  status: EventStatus | null;
  recurrenceRule: RecurrenceRule | null;
  alerts: EventAlert[];
  etag?: string;
  created?: string;
  updated?: string;
}

export interface Occurrence {
  id: string;
  start: number;
  end: number;
}

export interface EventInstance {
  key: string;
  occurrence: Occurrence;
  event: CalendarEvent;
}

export interface TimeSpan {
  start: number;
  end: number;
}
