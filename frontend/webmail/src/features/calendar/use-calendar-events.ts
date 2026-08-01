import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { chunkIds, joinOccurrences } from "./layout";
import { assertSetResult } from "./use-calendars";
import type { EventPayload } from "./event-form";
import type { CalendarEvent, EventInstance, Occurrence } from "./types";

const GET_CHUNK = 100;

export interface EventSetPayload {
  create?: Record<string, EventPayload>;
  update?: Record<string, Partial<EventPayload>>;
  destroy?: string[];
}

export function useCalendarEvents(startIso: string, endIso: string) {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  return useQuery<EventInstance[]>({
    queryKey: ["calendar-events", accountId, startIso, endIso],
    enabled: Boolean(accountId),
    placeholderData: (previous) => previous,
    queryFn: async () => {
      const result = await jmap.call<{ ids: string[]; total: number; occurrences: Occurrence[] }>(
        "CalendarEvent/query",
        {
          accountId,
          filter: { after: startIso, before: endIso, inCalendars: null },
          expandRecurrences: true,
        },
      );
      const events: CalendarEvent[] = [];
      for (const chunk of chunkIds(result.ids ?? [], GET_CHUNK)) {
        const page = await jmap.call<{ list: CalendarEvent[] }>("CalendarEvent/get", {
          accountId,
          ids: chunk,
        });
        events.push(...(page.list ?? []));
      }
      return joinOccurrences(result.occurrences ?? [], events);
    },
  });
}

export function useEventMutation() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const client = useQueryClient();

  return useMutation({
    mutationFn: async (payload: EventSetPayload) => {
      const result = await jmap.call<Record<string, unknown>>("CalendarEvent/set", { accountId, ...payload });
      assertSetResult(result);
      return result;
    },
    onSettled: () => {
      void client.invalidateQueries({ queryKey: ["calendar-events"] });
    },
  });
}
