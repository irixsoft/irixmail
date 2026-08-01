import * as React from "react";
import {
  Button,
  ConfirmDialog,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
  cn,
  toast,
} from "@irixmail/shared";
import { Check, Download, MoreHorizontal, Plus, Trash2, Upload } from "lucide-react";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { CALENDAR_PALETTE, calendarColor } from "./colors";
import { exportCalendarIcs, IcsImportDialog } from "./ics-transfer";
import { useCalendarMutation } from "./use-calendars";
import type { CalendarSummary } from "./types";

function Swatches({ value, onChange }: { value: string; onChange: (hex: string) => void }) {
  return (
    <div className="flex flex-wrap gap-1.5">
      {CALENDAR_PALETTE.map((swatch) => (
        <button
          key={swatch.id}
          type="button"
          aria-label={swatch.label}
          onClick={() => onChange(swatch.hex)}
          style={{ backgroundColor: swatch.hex }}
          className={cn(
            "flex size-5 items-center justify-center rounded-full transition-transform",
            value === swatch.hex ? "ring-2 ring-ring ring-offset-2 ring-offset-sidebar" : "hover:scale-110",
          )}
        >
          {value === swatch.hex ? <Check className="size-3 text-white" /> : null}
        </button>
      ))}
    </div>
  );
}

function CalendarRow({
  calendar,
  hidden,
  onToggle,
}: {
  calendar: CalendarSummary;
  hidden: boolean;
  onToggle: () => void;
}) {
  const mutation = useCalendarMutation();
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const [renaming, setRenaming] = React.useState(false);
  const [recoloring, setRecoloring] = React.useState(false);
  const [name, setName] = React.useState(calendar.name);
  const [confirming, setConfirming] = React.useState(false);
  const [importing, setImporting] = React.useState(false);
  const color = calendarColor(calendar);

  const exportIcs = () => {
    if (!accountId) return;
    void exportCalendarIcs(jmap, accountId, calendar)
      .then((count) => toast.success(`Exported ${count} events`))
      .catch((error: Error) => toast.error(error.message));
  };

  const commitName = () => {
    setRenaming(false);
    const next = name.trim();
    if (!next || next === calendar.name) return;
    mutation.mutate(
      { update: { [calendar.id]: { name: next } } },
      { onError: (error) => toast.error(error.message) },
    );
  };

  return (
    <div className="group rounded-md px-1 py-0.5">
      <div className="flex items-center gap-2">
        <button
          type="button"
          role="checkbox"
          aria-checked={!hidden}
          aria-label={`Toggle ${calendar.name}`}
          onClick={onToggle}
          style={{ backgroundColor: hidden ? "transparent" : color, borderColor: color }}
          className="flex size-4 shrink-0 items-center justify-center rounded-[4px] border-2"
        >
          {hidden ? null : <Check className="size-2.5 text-white" />}
        </button>
        {renaming ? (
          <Input
            autoFocus
            value={name}
            onChange={(event) => setName(event.target.value)}
            onBlur={commitName}
            onKeyDown={(event) => {
              if (event.key === "Enter") commitName();
              if (event.key === "Escape") {
                setName(calendar.name);
                setRenaming(false);
              }
            }}
            className="h-6 flex-1 text-[13px]"
          />
        ) : (
          <span className={cn("flex-1 truncate text-[13px]", hidden && "text-muted-foreground")}>
            {calendar.name}
          </span>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label={`${calendar.name} options`}
              className="flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground opacity-0 hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
            >
              <MoreHorizontal className="size-3.5" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-40">
            <DropdownMenuItem onClick={() => setRenaming(true)}>Rename</DropdownMenuItem>
            <DropdownMenuItem onClick={() => setRecoloring((current) => !current)}>Colour</DropdownMenuItem>
            <DropdownMenuItem onClick={() => setImporting(true)}>
              <Upload className="size-4" /> Import events
            </DropdownMenuItem>
            <DropdownMenuItem onClick={exportIcs}>
              <Download className="size-4" /> Export .ics
            </DropdownMenuItem>
            <DropdownMenuItem
              variant="destructive"
              disabled={calendar.isDefault}
              onClick={() => setConfirming(true)}
            >
              <Trash2 className="size-4" /> Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {recoloring ? (
        <div className="py-2 pl-6">
          <Swatches
            value={color}
            onChange={(hex) => {
              setRecoloring(false);
              mutation.mutate(
                { update: { [calendar.id]: { color: hex } } },
                { onError: (error) => toast.error(error.message) },
              );
            }}
          />
        </div>
      ) : null}

      <IcsImportDialog
        jmap={jmap}
        accountId={accountId ?? ""}
        calendar={calendar}
        open={importing}
        onOpenChange={setImporting}
      />

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={`Delete ${calendar.name}?`}
        description="Every event in this calendar is removed. This cannot be undone."
        confirmLabel="Delete"
        destructive
        onConfirm={() =>
          mutation.mutate({ destroy: [calendar.id] }, { onError: (error) => toast.error(error.message) })
        }
      />
    </div>
  );
}

export function CalendarList({
  calendars,
  hidden,
  onToggle,
}: {
  calendars: CalendarSummary[];
  hidden: string[];
  onToggle: (id: string) => void;
}) {
  const mutation = useCalendarMutation();
  const [adding, setAdding] = React.useState(false);
  const [name, setName] = React.useState("");
  const [color, setColor] = React.useState(CALENDAR_PALETTE[0]!.hex);

  const create = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    mutation.mutate(
      { create: { c1: { name: trimmed, color } } },
      {
        onSuccess: () => {
          setName("");
          setAdding(false);
        },
        onError: (error) => toast.error(error.message),
      },
    );
  };

  return (
    <div className="border-t px-2 py-3">
      <div className="flex items-center justify-between px-1 pb-1.5">
        <span className="font-mono text-[10px] uppercase tracking-wide text-muted-foreground">Calendars</span>
        <button
          type="button"
          aria-label="New calendar"
          onClick={() => setAdding((current) => !current)}
          className="flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <Plus className="size-3.5" />
        </button>
      </div>

      <div className="flex flex-col gap-px">
        {calendars.map((calendar) => (
          <CalendarRow
            key={calendar.id}
            calendar={calendar}
            hidden={hidden.includes(calendar.id)}
            onToggle={() => onToggle(calendar.id)}
          />
        ))}
      </div>

      {adding ? (
        <div className="mt-2 space-y-2 rounded-md border bg-card p-2">
          <Input
            autoFocus
            value={name}
            placeholder="Calendar name"
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") create();
              if (event.key === "Escape") setAdding(false);
            }}
            className="h-7 text-[13px]"
          />
          <Swatches value={color} onChange={setColor} />
          <div className="flex justify-end gap-1.5">
            <Button variant="ghost" size="sm" onClick={() => setAdding(false)}>
              Cancel
            </Button>
            <Button size="sm" disabled={!name.trim() || mutation.isPending} onClick={create}>
              Add
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
