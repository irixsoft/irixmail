import { EmptyState, cn } from "@irixmail/shared";
import { CalendarDays } from "lucide-react";

import { calendarColor } from "./colors";
import { formatAgendaDay, formatInstanceRange, instanceStart } from "./display";
import { chipVars } from "./event-chip";
import { dayKey, isSameDay, startOfDay } from "./layout";
import type { CalendarSummary, EventInstance } from "./types";

export interface AgendaViewProps {
  instances: EventInstance[];
  calendars: Record<string, CalendarSummary>;
  onSelect: (instance: EventInstance, element: HTMLElement) => void;
}

export function AgendaView({ instances, calendars, onSelect }: AgendaViewProps) {
  const today = startOfDay(new Date());
  const groups = new Map<string, { day: Date; items: EventInstance[] }>();
  for (const instance of instances) {
    const day = startOfDay(instanceStart(instance));
    const key = dayKey(day);
    const group = groups.get(key) ?? { day, items: [] };
    group.items.push(instance);
    groups.set(key, group);
  }

  if (groups.size === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <EmptyState
          icon={CalendarDays}
          title="Nothing scheduled"
          description="The next 30 days are clear. Add an event to fill them in."
        />
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto overscroll-contain">
      <div className="divide-y">
        {[...groups.values()].map((group) => (
          <section key={group.day.getTime()} className="flex gap-3 px-3 py-3 sm:px-4">
            <div className="w-20 shrink-0 pt-0.5 sm:w-28">
              <div
                className={cn(
                  "font-mono text-[11px] uppercase tracking-wide",
                  isSameDay(group.day, today) ? "text-primary" : "text-muted-foreground",
                )}
              >
                {formatAgendaDay(group.day)}
              </div>
            </div>
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              {group.items.map((instance) => {
                const calendar = calendars[instance.event.calendarId];
                return (
                  <button
                    key={instance.key}
                    type="button"
                    style={chipVars(calendarColor(calendar ?? { id: instance.event.calendarId, color: null }))}
                    onClick={(event) => onSelect(instance, event.currentTarget)}
                    className={cn(
                      "cal-chip flex min-w-0 items-center gap-3 rounded-md px-2.5 py-2 text-left",
                      instance.event.status === "cancelled" && "line-through opacity-60",
                    )}
                  >
                    <span className="w-28 shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">
                      {formatInstanceRange(instance)}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">
                      {instance.event.title || "Untitled"}
                    </span>
                    {instance.event.location ? (
                      <span className="hidden shrink-0 truncate text-xs text-muted-foreground sm:block">
                        {instance.event.location}
                      </span>
                    ) : null}
                  </button>
                );
              })}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
