import { Button, Skeleton } from "@irixmail/shared";
import { Plus } from "lucide-react";

import { CalendarList } from "./calendar-list";
import { MiniMonth } from "./mini-month";
import type { CalendarSummary, EventInstance } from "./types";

export function CalendarSidebar({
  anchor,
  instances,
  calendars,
  hidden,
  loading,
  onPickDay,
  onToggle,
  onCreate,
}: {
  anchor: Date;
  instances: EventInstance[];
  calendars: CalendarSummary[];
  hidden: string[];
  loading: boolean;
  onPickDay: (day: Date) => void;
  onToggle: (id: string) => void;
  onCreate: () => void;
}) {
  return (
    <div className="flex h-full flex-col overflow-y-auto bg-sidebar">
      <div className="p-3 pb-1">
        <Button
          onClick={onCreate}
          className="w-full justify-start gap-2 bg-gradient-to-br from-primary to-primary/80 shadow-sm"
        >
          <Plus className="size-4" /> New event
        </Button>
      </div>
      <MiniMonth anchor={anchor} instances={instances} onPick={onPickDay} />
      {loading ? (
        <div className="space-y-1 border-t p-3">
          {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-6 w-full" />
          ))}
        </div>
      ) : (
        <CalendarList calendars={calendars} hidden={hidden} onToggle={onToggle} />
      )}
    </div>
  );
}
