import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  Textarea,
} from "@irixmail/shared";

import { calendarColor } from "./colors";
import { ALERT_OPTIONS, type EventFormValues, type EventPayload } from "./event-form";
import { RecurrenceEditor } from "./recurrence-editor";
import { useEventForm } from "./use-event-form";
import type { CalendarSummary } from "./types";

export function EventDialog({
  open,
  initial,
  calendars,
  editing,
  pending,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  initial: EventFormValues;
  calendars: CalendarSummary[];
  editing: boolean;
  pending: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (payload: EventPayload) => void;
}) {
  const form = useEventForm(initial);
  const { values, set } = form;

  const submit = () => {
    form.markSubmitted();
    if (form.error) return;
    onSubmit(form.payload());
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90dvh] gap-4 overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{editing ? "Edit event" : "New event"}</DialogTitle>
          <DialogDescription className="sr-only">Event details</DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <Input
            autoFocus
            value={values.title}
            placeholder="Add a title"
            onChange={(event) => set("title", event.target.value)}
            className="h-10 text-base font-medium md:text-base"
          />

          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label className="text-xs text-muted-foreground">Calendar</Label>
              <Select value={values.calendarId} onValueChange={(value) => set("calendarId", value)}>
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Pick a calendar" />
                </SelectTrigger>
                <SelectContent>
                  {calendars.map((calendar) => (
                    <SelectItem key={calendar.id} value={calendar.id}>
                      <span className="flex items-center gap-2">
                        <span
                          style={{ backgroundColor: calendarColor(calendar) }}
                          className="size-2.5 rounded-full"
                        />
                        {calendar.name}
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex items-end gap-2 pb-2">
              <Switch
                id="calendar-all-day"
                checked={values.allDay}
                onCheckedChange={(checked) => set("allDay", checked)}
              />
              <Label htmlFor="calendar-all-day" className="text-[13px]">
                All day
              </Label>
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label className="text-xs text-muted-foreground">Starts</Label>
              <div className="flex gap-2">
                <Input
                  type="date"
                  value={values.startDate}
                  onChange={(event) => form.setStartDate(event.target.value)}
                  className="flex-1 font-mono text-[13px]"
                />
                {values.allDay ? null : (
                  <Input
                    type="time"
                    value={values.startTime}
                    onChange={(event) => form.setStartTime(event.target.value)}
                    className="w-28 font-mono text-[13px]"
                  />
                )}
              </div>
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs text-muted-foreground">Ends</Label>
              <div className="flex gap-2">
                <Input
                  type="date"
                  value={values.endDate}
                  onChange={(event) => set("endDate", event.target.value)}
                  className="flex-1 font-mono text-[13px]"
                />
                {values.allDay ? null : (
                  <Input
                    type="time"
                    value={values.endTime}
                    onChange={(event) => set("endTime", event.target.value)}
                    className="w-28 font-mono text-[13px]"
                  />
                )}
              </div>
            </div>
          </div>

          <Input
            value={values.location}
            placeholder="Location"
            onChange={(event) => set("location", event.target.value)}
          />

          <RecurrenceEditor
            rule={values.recurrence}
            startDate={values.startDate}
            onChange={(rule) => set("recurrence", rule)}
          />

          <div className="space-y-1.5">
            <Label className="text-xs text-muted-foreground">Reminder</Label>
            <Select
              value={String(values.alertMinutes ?? "none")}
              onValueChange={(value) => set("alertMinutes", value === "none" ? null : Number(value))}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {ALERT_OPTIONS.map((option) => (
                  <SelectItem key={String(option.value ?? "none")} value={String(option.value ?? "none")}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <Textarea
            value={values.description}
            placeholder="Notes"
            rows={3}
            onChange={(event) => set("description", event.target.value)}
          />

          {form.showError ? <p className="text-[13px] text-destructive">{form.showError}</p> : null}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={submit} loading={pending}>
            {editing ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
