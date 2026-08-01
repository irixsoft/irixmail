import * as React from "react";
import { cn } from "@irixmail/shared";

import { calendarColor } from "./colors";
import { formatHourLabel, formatWeekday } from "./display";
import { EventBar, EventBlock } from "./event-chip";
import {
  DAY_MINUTES,
  HOUR_HEIGHT,
  clipToDay,
  instanceSpan,
  isBarInstance,
  isSameDay,
  layoutDayEvents,
  slotMath,
  weekSegments,
} from "./layout";
import { useTimeGridDrag } from "./use-time-grid-drag";
import type { CalendarSummary, EventInstance } from "./types";

const GUTTER = "w-14";
const BAR_HEIGHT = 20;

function useNowMinutes(): { date: Date; minutes: number } {
  const [now, setNow] = React.useState(() => new Date());
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  return { date: now, minutes: now.getHours() * 60 + now.getMinutes() };
}

export interface TimeGridProps {
  days: Date[];
  instances: EventInstance[];
  calendars: Record<string, CalendarSummary>;
  enableDrag: boolean;
  showDayHeaders: boolean;
  onSelect: (instance: EventInstance, element: HTMLElement) => void;
  onCreate: (day: Date, startMinutes: number, endMinutes: number, allDay: boolean) => void;
  onMove: (instance: EventInstance, day: Date, startMinutes: number) => void;
  onResize: (instance: EventInstance, endMinutes: number) => void;
}

export function TimeGrid({
  days,
  instances,
  calendars,
  enableDrag,
  showDayHeaders,
  onSelect,
  onCreate,
  onMove,
  onResize,
}: TimeGridProps) {
  const columnsRef = React.useRef<HTMLDivElement | null>(null);
  const scrollRef = React.useRef<HTMLDivElement | null>(null);
  const now = useNowMinutes();

  const drag = useTimeGridDrag({
    containerRef: columnsRef,
    days,
    enabled: enableDrag,
    onCreate: (dayIndex, startMinutes, endMinutes) => {
      const day = days[dayIndex];
      if (day) onCreate(day, startMinutes, endMinutes, false);
    },
    onMove: (instance, dayIndex, startMinutes) => {
      const day = days[dayIndex];
      if (day) onMove(instance, day, startMinutes);
    },
    onResize,
  });

  const scrollToNow = React.useRef(now.minutes);
  React.useEffect(() => {
    const element = scrollRef.current;
    if (element) element.scrollTop = Math.max(0, slotMath.minutesToPx(scrollToNow.current) - 160);
  }, []);

  const bars = instances.filter(isBarInstance);
  const segments = weekSegments(bars.map(instanceSpan), days);
  const barRows = segments.reduce((rows, segment) => Math.max(rows, segment.row + 1), 0);

  const timed = instances.filter((instance) => !isBarInstance(instance));
  const perDay = days.map((day) => {
    const forDay = timed
      .map((instance) => ({ instance, clip: clipToDay(instance.occurrence, day) }))
      .filter((entry): entry is { instance: EventInstance; clip: { startMinutes: number; endMinutes: number } } =>
        Boolean(entry.clip),
      );
    const packed = layoutDayEvents(forDay.map((entry) => ({ start: entry.clip.startMinutes, end: entry.clip.endMinutes })));
    return forDay.map((entry, index) => ({ ...entry, slot: packed[index] ?? { column: 0, columns: 1 } }));
  });

  const preview = drag.preview;

  return (
    <div className="flex h-full min-w-0 flex-col">
      <div className="flex shrink-0 border-b bg-background">
        <div className={cn(GUTTER, "shrink-0")} />
        <div className="grid flex-1" style={{ gridTemplateColumns: `repeat(${days.length}, minmax(0, 1fr))` }}>
          {days.map((day) => {
            const today = isSameDay(day, now.date);
            return (
              <div key={day.getTime()} className="flex items-center justify-center gap-1.5 py-1.5">
                {showDayHeaders ? (
                  <span className="font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
                    {formatWeekday(day)}
                  </span>
                ) : null}
                <span
                  className={cn(
                    "flex size-6 items-center justify-center rounded-full font-mono text-[12px] tabular-nums",
                    today ? "bg-primary font-semibold text-primary-foreground" : "text-foreground",
                  )}
                >
                  {day.getDate()}
                </span>
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex shrink-0 border-b bg-muted/20">
        <div className={cn(GUTTER, "shrink-0 pr-2 pt-1 text-right font-mono text-[9px] uppercase tracking-wide text-muted-foreground")}>
          all day
        </div>
        <div className="relative flex-1" style={{ height: Math.max(barRows * BAR_HEIGHT + 6, 22) }}>
          {segments.map((segment) => {
            const instance = bars[segment.index];
            if (!instance) return null;
            const calendar = calendars[instance.event.calendarId];
            return (
              <EventBar
                key={instance.key}
                instance={instance}
                color={calendarColor(calendar ?? { id: instance.event.calendarId, color: null })}
                onSelect={onSelect}
                style={{
                  position: "absolute",
                  top: segment.row * BAR_HEIGHT + 2,
                  left: `${(segment.startIndex / days.length) * 100}%`,
                  width: `calc(${(segment.span / days.length) * 100}% - 4px)`,
                  marginLeft: 2,
                }}
              />
            );
          })}
        </div>
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
        <div className="flex" style={{ height: (DAY_MINUTES / 60) * HOUR_HEIGHT }}>
          <div className={cn(GUTTER, "relative shrink-0")}>
            {Array.from({ length: 23 }, (_, index) => index + 1).map((hour) => (
              <span
                key={hour}
                className="absolute right-2 -translate-y-1/2 font-mono text-[10px] tabular-nums text-muted-foreground"
                style={{ top: slotMath.minutesToPx(hour * 60) }}
              >
                {formatHourLabel(hour)}
              </span>
            ))}
            {days.some((day) => isSameDay(day, now.date)) ? (
              <span
                className="absolute right-1 size-2 -translate-y-1/2 rounded-full bg-primary"
                style={{ top: slotMath.minutesToPx(now.minutes) }}
              />
            ) : null}
          </div>

          <div
            ref={columnsRef}
            onPointerDown={drag.onBackgroundPointerDown}
            className="grid flex-1"
            style={
              {
                gridTemplateColumns: `repeat(${days.length}, minmax(0, 1fr))`,
                "--cal-hour": `${HOUR_HEIGHT}px`,
              } as React.CSSProperties
            }
          >
            {days.map((day, dayIndex) => (
              <div
                key={day.getTime()}
                onClick={
                  enableDrag
                    ? undefined
                    : (event) => {
                        if (event.target !== event.currentTarget) return;
                        const rect = event.currentTarget.getBoundingClientRect();
                        const raw = slotMath.pxToMinutes(event.clientY - rect.top);
                        const start = Math.min(slotMath.snapMinutes(slotMath.clampMinutes(raw), 60), DAY_MINUTES - 60);
                        onCreate(day, start, start + 60, false);
                      }
                }
                className="cal-hours relative border-l first:border-l-0"
              >
                {perDay[dayIndex]?.map((entry) => {
                  const calendar = calendars[entry.instance.event.calendarId];
                  const hidden = preview?.instanceKey === entry.instance.key;
                  const top = slotMath.minutesToPx(entry.clip.startMinutes);
                  const height = Math.max(slotMath.minutesToPx(entry.clip.endMinutes - entry.clip.startMinutes) - 2, 16);
                  return (
                    <EventBlock
                      key={entry.instance.key}
                      instance={entry.instance}
                      color={calendarColor(calendar ?? { id: entry.instance.event.calendarId, color: null })}
                      dragging={hidden}
                      onSelect={(instance, element) => {
                        if (!drag.consumedClick()) onSelect(instance, element);
                      }}
                      onMoveStart={enableDrag ? drag.onBlockPointerDown : undefined}
                      onResizeStart={enableDrag ? drag.onResizePointerDown : undefined}
                      style={{
                        top,
                        height,
                        left: `calc(${(entry.slot.column / entry.slot.columns) * 100}% + 1px)`,
                        width: `calc(${100 / entry.slot.columns}% - 3px)`,
                      }}
                    />
                  );
                })}

                {preview && preview.dayIndex === dayIndex ? (
                  <div
                    style={{
                      top: slotMath.minutesToPx(preview.startMinutes),
                      height: Math.max(slotMath.minutesToPx(preview.endMinutes - preview.startMinutes), 16),
                    }}
                    className="pointer-events-none absolute inset-x-0.5 z-20 rounded-md border border-primary/50 bg-primary/15"
                  />
                ) : null}

                {isSameDay(day, now.date) ? (
                  <div
                    className="pointer-events-none absolute inset-x-0 z-30 h-px bg-primary"
                    style={{ top: slotMath.minutesToPx(now.minutes) }}
                  />
                ) : null}
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
