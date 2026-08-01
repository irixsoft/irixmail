import * as React from "react";
import { cn } from "@irixmail/shared";

import { calendarColor } from "./colors";
import { formatWeekday } from "./display";
import { EventBar, EventChipInline } from "./event-chip";
import {
  clipToDay,
  instanceSpan,
  isBarInstance,
  isSameDay,
  monthGrid,
  startOfDay,
} from "./layout";
import { weekSegments } from "./layout";
import type { CalendarSummary, EventInstance } from "./types";

const BAR_HEIGHT = 20;
const MAX_CHIPS = 3;

export interface MonthViewProps {
  anchor: Date;
  instances: EventInstance[];
  calendars: Record<string, CalendarSummary>;
  onSelect: (instance: EventInstance, element: HTMLElement) => void;
  onOpenDay: (day: Date) => void;
  onCreate: (day: Date) => void;
  cellTap: "day" | "create";
}

export function MonthView({
  anchor,
  instances,
  calendars,
  onSelect,
  onOpenDay,
  onCreate,
  cellTap,
}: MonthViewProps) {
  const today = React.useMemo(() => startOfDay(new Date()), []);
  const grid = monthGrid(anchor);
  const bars = instances.filter(isBarInstance);
  const timed = instances.filter((instance) => !isBarInstance(instance));
  const month = anchor.getMonth();

  const colorOf = (instance: EventInstance) =>
    calendarColor(calendars[instance.event.calendarId] ?? { id: instance.event.calendarId, color: null });

  return (
    <div className="flex h-full min-w-0 flex-col">
      <div className="grid shrink-0 grid-cols-7 border-b">
        {(grid[0] ?? []).map((day) => (
          <div
            key={day.getTime()}
            className="py-1.5 text-center font-mono text-[10px] uppercase tracking-wide text-muted-foreground"
          >
            {formatWeekday(day)}
          </div>
        ))}
      </div>

      <div className="grid min-h-0 flex-1 grid-rows-6">
        {grid.map((week, weekIndex) => {
          const segments = weekSegments(bars.map(instanceSpan), week);
          const barRows = segments.reduce((rows, segment) => Math.max(rows, segment.row + 1), 0);
          const barsHeight = barRows * BAR_HEIGHT;
          return (
            <div key={weekIndex} className="relative grid min-h-0 grid-cols-7 border-b last:border-b-0">
              {week.map((day) => {
                const inMonth = day.getMonth() === month;
                const isToday = isSameDay(day, today);
                const dayEvents = timed.filter((instance) => clipToDay(instance.occurrence, day) !== null);
                const visible = dayEvents.slice(0, Math.max(1, MAX_CHIPS - barRows));
                const hidden = dayEvents.length - visible.length;
                return (
                  <div
                    key={day.getTime()}
                    onClick={(event) => {
                      if (event.target !== event.currentTarget) return;
                      if (cellTap === "day") onOpenDay(day);
                      else onCreate(day);
                    }}
                    className={cn(
                      "flex min-h-0 min-w-0 flex-col overflow-hidden border-l px-1 pb-1 first:border-l-0",
                      !inMonth && "bg-muted/25",
                    )}
                  >
                    <div className="flex shrink-0 justify-end py-1">
                      <button
                        type="button"
                        onClick={() => onOpenDay(day)}
                        className={cn(
                          "flex size-6 items-center justify-center rounded-full font-mono text-[11px] tabular-nums transition-colors",
                          isToday
                            ? "bg-primary font-semibold text-primary-foreground"
                            : inMonth
                              ? "text-foreground hover:bg-accent"
                              : "text-muted-foreground hover:bg-accent",
                        )}
                      >
                        {day.getDate()}
                      </button>
                    </div>
                    <div style={{ height: barsHeight }} className="shrink-0" />
                    <div className="flex min-h-0 flex-col gap-px">
                      {visible.map((instance) => (
                        <EventChipInline
                          key={instance.key}
                          instance={instance}
                          color={colorOf(instance)}
                          onSelect={onSelect}
                        />
                      ))}
                      {hidden > 0 ? (
                        <button
                          type="button"
                          onClick={() => onOpenDay(day)}
                          className="px-1 text-left font-mono text-[10px] text-muted-foreground hover:text-foreground"
                        >
                          +{hidden} more
                        </button>
                      ) : null}
                    </div>
                  </div>
                );
              })}

              <div className="pointer-events-none absolute inset-x-0 top-8">
                {segments.map((segment) => {
                  const instance = bars[segment.index];
                  if (!instance) return null;
                  return (
                    <EventBar
                      key={instance.key}
                      instance={instance}
                      color={colorOf(instance)}
                      onSelect={onSelect}
                      style={{
                        position: "absolute",
                        pointerEvents: "auto",
                        top: segment.row * BAR_HEIGHT,
                        left: `calc(${(segment.startIndex / 7) * 100}% + 3px)`,
                        width: `calc(${(segment.span / 7) * 100}% - 6px)`,
                      }}
                    />
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
