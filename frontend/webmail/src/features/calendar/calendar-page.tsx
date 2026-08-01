import * as React from "react";
import { useSearchParams } from "react-router-dom";
import { EmptyState, ErrorState, Spinner, toast } from "@irixmail/shared";
import { CalendarDays } from "lucide-react";

import { useIsMobile } from "@/app/use-is-mobile";
import { AgendaView } from "./agenda-view";
import { CalendarSidebar } from "./calendar-sidebar";
import { CalendarToolbar } from "./calendar-toolbar";
import { DayView } from "./day-view";
import { EventDialog } from "./event-dialog";
import { EventPopover } from "./event-popover";
import { MonthView } from "./month-view";
import { WeekView } from "./week-view";
import {
  anchorFromParam,
  anchorToParam,
  loadCalendarView,
  loadHiddenCalendars,
  saveCalendarView,
  saveHiddenCalendars,
  toggleHidden,
  viewFromParam,
} from "./calendar-store";
import {
  defaultEventForm,
  eventToForm,
  formatDuration,
  naiveAtMinutes,
  type EventFormValues,
  type EventPayload,
} from "./event-form";
import { instanceStart } from "./display";
import { shiftAnchor, startOfDay, visibleRange } from "./layout";
import { useCalendarEvents, useEventMutation } from "./use-calendar-events";
import { useCalendars } from "./use-calendars";
import type { CalendarView, EventInstance } from "./types";

interface DialogState {
  key: number;
  initial: EventFormValues;
  editingId: string | null;
}

export function CalendarPage() {
  const isMobile = useIsMobile();
  const [params, setParams] = useSearchParams();
  const [hidden, setHidden] = React.useState<string[]>(loadHiddenCalendars);
  const [dialog, setDialog] = React.useState<DialogState | null>(null);
  const [selected, setSelected] = React.useState<{ instance: EventInstance; rect: DOMRect } | null>(null);
  const dialogSeq = React.useRef(0);

  const fallbackView: CalendarView = isMobile ? "agenda" : "month";
  const view = viewFromParam(params.get("view"), loadCalendarView(fallbackView));
  const anchor = anchorFromParam(params.get("date"), startOfDay(new Date()));

  const { list: calendars, byId, defaultId, query: calendarQuery } = useCalendars();
  const [rangeStart, rangeEnd] = visibleRange(view, anchor);
  const eventsQuery = useCalendarEvents(rangeStart, rangeEnd);
  const mutation = useEventMutation();

  const setView = (next: CalendarView) => {
    saveCalendarView(next);
    setParams((current) => {
      current.set("view", next);
      return current;
    });
  };

  const setAnchor = (next: Date) =>
    setParams((current) => {
      current.set("date", anchorToParam(next));
      return current;
    });

  const toggleCalendar = (id: string) => {
    const next = toggleHidden(hidden, id);
    setHidden(next);
    saveHiddenCalendars(next);
  };

  const instances = (eventsQuery.data ?? []).filter(
    (instance) => !hidden.includes(instance.event.calendarId),
  );

  const openDialog = (initial: EventFormValues, editingId: string | null) => {
    dialogSeq.current += 1;
    setSelected(null);
    setDialog({ key: dialogSeq.current, initial, editingId });
  };

  const openCreate = (day: Date, startMinutes: number, endMinutes: number, allDay: boolean) => {
    if (!defaultId) {
      toast.error("Create a calendar first");
      return;
    }
    openDialog(defaultEventForm(defaultId, day, startMinutes, endMinutes, allDay), null);
  };

  React.useEffect(() => {
    if (params.get("create") !== "1" || !defaultId) return;
    setParams(
      (current) => {
        current.delete("create");
        return current;
      },
      { replace: true },
    );
    openDialog(defaultEventForm(defaultId, startOfDay(new Date()), 540, 600, false), null);
  }, [params, defaultId, setParams]);

  const submit = (payload: EventPayload) => {
    const editingId = dialog?.editingId;
    mutation.mutate(
      editingId ? { update: { [editingId]: payload } } : { create: { e1: payload } },
      {
        onSuccess: () => setDialog(null),
        onError: (error) => toast.error(error.message),
      },
    );
  };

  const remove = (instance: EventInstance) => {
    setSelected(null);
    mutation.mutate({ destroy: [instance.event.id] }, { onError: (error) => toast.error(error.message) });
  };

  const moveInstance = (instance: EventInstance, day: Date, startMinutes: number) => {
    if (instance.event.showWithoutTime) return;
    mutation.mutate(
      { update: { [instance.event.id]: { start: naiveAtMinutes(day, startMinutes) } } },
      { onError: (error) => toast.error(error.message) },
    );
  };

  const resizeInstance = (instance: EventInstance, endMinutes: number) => {
    const start = instanceStart(instance);
    const startMinutes = start.getHours() * 60 + start.getMinutes();
    mutation.mutate(
      { update: { [instance.event.id]: { duration: formatDuration(endMinutes - startMinutes) } } },
      { onError: (error) => toast.error(error.message) },
    );
  };

  const select = (instance: EventInstance, element: HTMLElement) =>
    setSelected({ instance, rect: element.getBoundingClientRect() });

  const gridProps = {
    instances,
    calendars: byId,
    onSelect: select,
    onCreate: openCreate,
    onMove: moveInstance,
    onResize: resizeInstance,
    enableDrag: !isMobile,
  };

  const body = () => {
    if (eventsQuery.isError) {
      return (
        <div className="flex h-full items-center justify-center p-6">
          <ErrorState
            title="Could not load the calendar"
            description={(eventsQuery.error as Error).message}
            onRetry={() => void eventsQuery.refetch()}
          />
        </div>
      );
    }
    if (calendarQuery.isSuccess && calendars.length === 0) {
      return (
        <div className="flex h-full items-center justify-center p-6">
          <EmptyState
            icon={CalendarDays}
            title="No calendars yet"
            description="Add a calendar from the sidebar to start scheduling."
          />
        </div>
      );
    }
    if (eventsQuery.isPending) {
      return (
        <div className="flex h-full items-center justify-center">
          <Spinner className="size-5 text-muted-foreground" />
        </div>
      );
    }
    if (view === "month") {
      return (
        <MonthView
          anchor={anchor}
          instances={instances}
          calendars={byId}
          onSelect={select}
          onOpenDay={(day) => {
            setAnchor(day);
            setView("day");
          }}
          onCreate={(day) => openCreate(day, 540, 600, false)}
          cellTap={isMobile ? "day" : "create"}
        />
      );
    }
    if (view === "week") return <WeekView anchor={anchor} {...gridProps} />;
    if (view === "day") return <DayView anchor={anchor} {...gridProps} />;
    return <AgendaView instances={instances} calendars={byId} onSelect={select} />;
  };

  const main = (
    <div className="flex h-full min-w-0 flex-col">
      <CalendarToolbar
        view={view}
        anchor={anchor}
        compact={isMobile}
        onView={setView}
        onShift={(direction) => setAnchor(shiftAnchor(view, anchor, direction))}
        onToday={() => setAnchor(startOfDay(new Date()))}
        onCreate={() => openCreate(startOfDay(new Date()), 540, 600, false)}
      />
      <div className="min-h-0 flex-1">{body()}</div>
    </div>
  );

  return (
    <div className="flex h-full min-w-0">
      {isMobile ? null : (
        <div className="w-60 shrink-0 border-r border-sidebar-border">
          <CalendarSidebar
            anchor={anchor}
            instances={instances}
            calendars={calendars}
            hidden={hidden}
            loading={calendarQuery.isPending}
            onPickDay={(day) => {
              setAnchor(day);
              if (view === "agenda") setView("day");
            }}
            onToggle={toggleCalendar}
            onCreate={() => openCreate(anchor, 540, 600, false)}
          />
        </div>
      )}
      {main}

      <EventPopover
        instance={selected?.instance ?? null}
        rect={selected?.rect ?? null}
        calendars={byId}
        onOpenChange={(open) => {
          if (!open) setSelected(null);
        }}
        onEdit={(instance) => openDialog(eventToForm(instance.event), instance.event.id)}
        onDelete={remove}
      />

      {dialog ? (
        <EventDialog
          key={dialog.key}
          open
          initial={dialog.initial}
          calendars={calendars}
          editing={Boolean(dialog.editingId)}
          pending={mutation.isPending}
          onOpenChange={(open) => {
            if (!open) setDialog(null);
          }}
          onSubmit={submit}
        />
      ) : null}
    </div>
  );
}
