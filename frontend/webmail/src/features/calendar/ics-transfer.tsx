import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  toast,
} from "@irixmail/shared";
import type { JmapClient } from "@irixmail/shared";

import { downloadText } from "../contacts/download";
import { assertSetResult } from "./use-calendars";
import { generateIcs, parseIcs, planIcsImport, type ParsedIcsEvent } from "./ics";
import type { CalendarEvent, CalendarSummary } from "./types";

const IMPORT_CHUNK = 50;

async function allEvents(jmap: JmapClient, accountId: string): Promise<CalendarEvent[]> {
  const result = await jmap.call<{ list: CalendarEvent[] }>("CalendarEvent/get", { accountId, ids: null });
  return result.list;
}

function utcStamp(now: Date): string {
  return now.toISOString().slice(0, 19).replace(/[-:]/g, "") + "Z";
}

export async function exportCalendarIcs(jmap: JmapClient, accountId: string, calendar: CalendarSummary) {
  const events = (await allEvents(jmap, accountId)).filter((event) => event.calendarId === calendar.id);
  const text = generateIcs(events, calendar.name, utcStamp(new Date()));
  downloadText(`${calendar.name.replace(/[^\w.-]+/g, "-") || "calendar"}.ics`, text, "text/calendar;charset=utf-8");
  return events.length;
}

export function IcsImportDialog({
  jmap,
  accountId,
  calendar,
  open,
  onOpenChange,
}: {
  jmap: JmapClient;
  accountId: string;
  calendar: CalendarSummary;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const client = useQueryClient();
  const [plan, setPlan] = React.useState<{ fresh: ParsedIcsEvent[]; duplicates: number } | null>(null);
  const [busy, setBusy] = React.useState(false);

  const pickFile = async (file: File) => {
    setBusy(true);
    try {
      const parsed = parseIcs(await file.text());
      const existing = await allEvents(jmap, accountId);
      setPlan(planIcsImport(parsed, existing.map((event) => event.uid)));
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Could not read that file");
    } finally {
      setBusy(false);
    }
  };

  const runImport = async () => {
    if (!plan) return;
    setBusy(true);
    try {
      for (let index = 0; index < plan.fresh.length; index += IMPORT_CHUNK) {
        const chunk = plan.fresh.slice(index, index + IMPORT_CHUNK);
        const create: Record<string, unknown> = {};
        chunk.forEach((event, offset) => {
          create[`i${index + offset}`] = { calendarId: calendar.id, ...event.payload };
        });
        const result = await jmap.call<Record<string, unknown>>("CalendarEvent/set", { accountId, create });
        assertSetResult(result);
      }
      void client.invalidateQueries({ queryKey: ["calendar-events"] });
      toast.success(`Imported ${plan.fresh.length} events into ${calendar.name}`);
      onOpenChange(false);
      setPlan(null);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Import failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        onOpenChange(next);
        if (!next) setPlan(null);
      }}
    >
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>Import events</DialogTitle>
          <DialogDescription>Add events from an .ics file to {calendar.name}.</DialogDescription>
        </DialogHeader>
        {plan ? (
          <p className="text-sm">
            {plan.fresh.length} events ready to import.
            {plan.duplicates > 0 ? ` ${plan.duplicates} already exist and are skipped.` : ""}
          </p>
        ) : (
          <input
            type="file"
            accept=".ics,text/calendar"
            disabled={busy}
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) void pickFile(file);
            }}
            className="text-sm file:mr-3 file:rounded-md file:border file:bg-secondary file:px-3 file:py-1.5 file:text-sm"
          />
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => void runImport()} disabled={busy || !plan || plan.fresh.length === 0}>
            Import
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
