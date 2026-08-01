import { Button, Popover, PopoverAnchor, PopoverContent, Separator } from "@irixmail/shared";
import { CalendarDays, MapPin, Pencil, Repeat, Trash2 } from "lucide-react";

import { calendarColor } from "./colors";
import { formatInstanceDetail } from "./display";
import { recurrenceSummary } from "./event-form";
import type { CalendarSummary, EventInstance } from "./types";

export function EventPopover({
  instance,
  rect,
  calendars,
  onOpenChange,
  onEdit,
  onDelete,
}: {
  instance: EventInstance | null;
  rect: DOMRect | null;
  calendars: Record<string, CalendarSummary>;
  onOpenChange: (open: boolean) => void;
  onEdit: (instance: EventInstance) => void;
  onDelete: (instance: EventInstance) => void;
}) {
  const calendar = instance ? calendars[instance.event.calendarId] : undefined;
  const color = instance
    ? calendarColor(calendar ?? { id: instance.event.calendarId, color: null })
    : "transparent";

  return (
    <Popover open={Boolean(instance)} onOpenChange={onOpenChange}>
      <PopoverAnchor asChild>
        <div
          aria-hidden
          className="pointer-events-none fixed"
          style={{
            left: rect?.left ?? 0,
            top: rect?.top ?? 0,
            width: rect?.width ?? 0,
            height: rect?.height ?? 0,
          }}
        />
      </PopoverAnchor>
      {instance ? (
        <PopoverContent side="right" align="start" className="w-80 p-0">
          <div className="space-y-2 p-3.5">
            <div className="flex items-start gap-2.5">
              <span style={{ backgroundColor: color }} className="mt-1.5 size-2.5 shrink-0 rounded-full" />
              <div className="min-w-0 space-y-0.5">
                <h2 className="text-[15px] font-semibold leading-snug">
                  {instance.event.title || "Untitled"}
                </h2>
                <p className="font-mono text-[11px] text-muted-foreground">
                  {formatInstanceDetail(instance)}
                </p>
              </div>
            </div>

            <div className="space-y-1.5 pl-[22px] text-[13px] text-muted-foreground">
              {instance.event.location ? (
                <p className="flex items-start gap-2">
                  <MapPin className="mt-0.5 size-3.5 shrink-0" />
                  <span className="min-w-0 break-words">{instance.event.location}</span>
                </p>
              ) : null}
              <p className="flex items-center gap-2">
                <CalendarDays className="size-3.5 shrink-0" />
                <span className="truncate">{calendar?.name ?? "Calendar"}</span>
              </p>
              {instance.event.recurrenceRule ? (
                <p className="flex items-center gap-2">
                  <Repeat className="size-3.5 shrink-0" />
                  <span className="truncate">{recurrenceSummary(instance.event.recurrenceRule)}</span>
                </p>
              ) : null}
              {instance.event.description ? (
                <p className="whitespace-pre-wrap break-words pt-0.5 text-foreground/80">
                  {instance.event.description}
                </p>
              ) : null}
            </div>
          </div>
          <Separator />
          <div className="flex items-center justify-end gap-1 p-1.5">
            <Button variant="ghost" size="sm" onClick={() => onEdit(instance)}>
              <Pencil className="size-3.5" /> Edit
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onDelete(instance)}
              className="text-destructive hover:bg-destructive/10 hover:text-destructive"
            >
              <Trash2 className="size-3.5" /> Delete
            </Button>
          </div>
        </PopoverContent>
      ) : null}
    </Popover>
  );
}
