import * as React from "react";
import { cn } from "@irixmail/shared";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { formatWeekday, instanceStart } from "./display";
import { addMonths, dayKey, isSameDay, monthGrid, startOfDay } from "./layout";
import type { EventInstance } from "./types";

export function MiniMonth({
  anchor,
  instances,
  onPick,
}: {
  anchor: Date;
  instances: EventInstance[];
  onPick: (day: Date) => void;
}) {
  const [cursor, setCursor] = React.useState(() => startOfDay(anchor));
  React.useEffect(() => setCursor(startOfDay(anchor)), [anchor]);

  const today = startOfDay(new Date());
  const grid = monthGrid(cursor);
  const busy = new Set(instances.map((instance) => dayKey(instanceStart(instance))));
  const label = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric" }).format(cursor);

  return (
    <div className="px-3 pb-3 pt-2">
      <div className="mb-1 flex items-center justify-between">
        <span className="text-[13px] font-medium">{label}</span>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            aria-label="Previous month"
            onClick={() => setCursor((current) => addMonths(current, -1))}
            className="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <ChevronLeft className="size-3.5" />
          </button>
          <button
            type="button"
            aria-label="Next month"
            onClick={() => setCursor((current) => addMonths(current, 1))}
            className="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <ChevronRight className="size-3.5" />
          </button>
        </div>
      </div>

      <div className="grid grid-cols-7">
        {(grid[0] ?? []).map((day) => (
          <div
            key={day.getTime()}
            className="pb-0.5 text-center font-mono text-[9px] uppercase text-muted-foreground"
          >
            {formatWeekday(day).slice(0, 1)}
          </div>
        ))}
        {grid.flat().map((day) => {
          const isToday = isSameDay(day, today);
          const selected = isSameDay(day, anchor);
          return (
            <button
              key={day.getTime()}
              type="button"
              onClick={() => onPick(day)}
              className={cn(
                "relative flex h-6 items-center justify-center rounded font-mono text-[11px] tabular-nums transition-colors",
                day.getMonth() === cursor.getMonth() ? "text-foreground" : "text-muted-foreground/60",
                isToday && "bg-primary font-semibold text-primary-foreground",
                !isToday && selected && "bg-accent font-medium",
                !isToday && "hover:bg-accent",
              )}
            >
              {day.getDate()}
              {busy.has(dayKey(day)) ? (
                <span
                  className={cn(
                    "absolute bottom-0.5 size-1 rounded-full",
                    isToday ? "bg-primary-foreground/80" : "bg-primary/70",
                  )}
                />
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}
