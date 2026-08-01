import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useJmap, useJmapSession } from "@/lib/jmap";
import type { CalendarSummary } from "./types";

export interface CalendarSetPayload {
  create?: Record<string, { name: string; color?: string | null }>;
  update?: Record<string, { name?: string; color?: string | null; sortOrder?: number }>;
  destroy?: string[];
}

export function useCalendars() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  const query = useQuery({
    queryKey: ["calendars", accountId],
    queryFn: () => jmap.call<{ list: CalendarSummary[] }>("Calendar/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });

  const list = [...(query.data?.list ?? [])].sort(
    (a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name),
  );
  const byId: Record<string, CalendarSummary> = {};
  for (const calendar of list) byId[calendar.id] = calendar;
  const defaultId = list.find((calendar) => calendar.isDefault)?.id ?? list[0]?.id ?? "";

  return { query, list, byId, defaultId, accountId };
}

export function useCalendarMutation() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const client = useQueryClient();

  return useMutation({
    mutationFn: async (payload: CalendarSetPayload) => {
      const result = await jmap.call<Record<string, unknown>>("Calendar/set", { accountId, ...payload });
      assertSetResult(result);
      return result;
    },
    onSettled: () => {
      void client.invalidateQueries({ queryKey: ["calendars"] });
      void client.invalidateQueries({ queryKey: ["calendar-events"] });
    },
  });
}

export function assertSetResult(result: Record<string, unknown>) {
  for (const key of ["notCreated", "notUpdated", "notDestroyed"]) {
    const failures = result[key] as Record<string, { description?: string; type?: string }> | null | undefined;
    const first = failures ? Object.values(failures)[0] : undefined;
    if (first) throw new Error(first.description ?? first.type ?? "The server rejected the change");
  }
}
