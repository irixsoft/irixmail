import { Button, cn } from "@irixmail/shared";
import { ChevronLeft, ChevronRight, Plus } from "lucide-react";

import { periodLabel } from "./display";
import type { CalendarView } from "./types";

const VIEWS: { id: CalendarView; label: string; short: string }[] = [
  { id: "month", label: "Month", short: "M" },
  { id: "week", label: "Week", short: "W" },
  { id: "day", label: "Day", short: "D" },
  { id: "agenda", label: "Agenda", short: "A" },
];

export function CalendarToolbar({
  view,
  anchor,
  compact,
  onView,
  onShift,
  onToday,
  onCreate,
}: {
  view: CalendarView;
  anchor: Date;
  compact: boolean;
  onView: (view: CalendarView) => void;
  onShift: (direction: number) => void;
  onToday: () => void;
  onCreate: () => void;
}) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b px-2 py-2 sm:px-3">
      <div className="flex items-center">
        <button
          type="button"
          aria-label="Previous period"
          onClick={() => onShift(-1)}
          className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <ChevronLeft className="size-4" />
        </button>
        <button
          type="button"
          aria-label="Next period"
          onClick={() => onShift(1)}
          className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <ChevronRight className="size-4" />
        </button>
      </div>

      <h1 className="min-w-0 flex-1 truncate text-[15px] font-semibold tracking-tight sm:text-base">
        {periodLabel(view, anchor)}
      </h1>

      <Button variant="ghost" size="sm" onClick={onToday} className="font-mono text-[11px] uppercase">
        Today
      </Button>

      <div className="flex items-center gap-px rounded-md bg-muted p-0.5">
        {VIEWS.map((entry) => (
          <button
            key={entry.id}
            type="button"
            aria-pressed={view === entry.id}
            onClick={() => onView(entry.id)}
            className={cn(
              "rounded-[5px] px-2 py-1 text-[12px] transition-colors",
              view === entry.id
                ? "bg-card font-medium text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {compact ? entry.short : entry.label}
          </button>
        ))}
      </div>

      {compact ? (
        <button
          type="button"
          aria-label="New event"
          onClick={onCreate}
          className="flex size-8 items-center justify-center rounded-md bg-primary text-primary-foreground"
        >
          <Plus className="size-4" />
        </button>
      ) : null}
    </div>
  );
}
