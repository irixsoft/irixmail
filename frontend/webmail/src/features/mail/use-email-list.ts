import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import type { JmapResponse } from "@irixmail/shared";

import { emailListCalls } from "@/jmap/requests";
import type { EmailPatch } from "@/jmap/mutations";
import { useJmap, useJmapSession } from "@/lib/jmap";
import type { EmailListItem } from "@/lib/mail-types";

export const PAGE_SIZE = 50;

interface EmailListPage {
  ids: string[];
  emails: EmailListItem[];
  total: number;
  queryState: string | null;
}

function pageFromResponse(response: JmapResponse): EmailListPage {
  let ids: string[] = [];
  let total = 0;
  let queryState: string | null = null;
  let emails: EmailListItem[] = [];
  for (const [name, args, callId] of response.methodResponses) {
    if (name === "Email/query" && callId === "q") {
      ids = (args["ids"] as string[]) ?? [];
      total = (args["total"] as number) ?? 0;
      queryState = (args["queryState"] as string) ?? null;
    }
    if (name === "Email/get" && callId === "g") {
      emails = (args["list"] as EmailListItem[]) ?? [];
    }
    if (name === "error") throw new Error(String(args["type"] ?? "jmap error"));
  }
  const byId = new Map(emails.map((email) => [email.id, email]));
  return {
    ids,
    emails: ids.map((id) => byId.get(id)).filter((email): email is EmailListItem => Boolean(email)),
    total,
    queryState,
  };
}

export function useEmailList(filter: Record<string, unknown>, keyExtra: string) {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  const query = useInfiniteQuery({
    queryKey: ["emails", accountId, keyExtra],
    enabled: Boolean(accountId),
    initialPageParam: 0,
    queryFn: async ({ pageParam }) => {
      const response = await jmap.request(
        emailListCalls(accountId!, { filter, position: pageParam, limit: PAGE_SIZE }),
      );
      return pageFromResponse(response);
    },
    getNextPageParam: (_last, pages) => {
      const loaded = pages.reduce((sum, page) => sum + page.ids.length, 0);
      const total = pages[pages.length - 1]?.total ?? 0;
      return loaded < total ? loaded : undefined;
    },
  });

  const pages = query.data?.pages ?? [];
  const seen = new Set<string>();
  const emails: EmailListItem[] = [];
  for (const page of pages) {
    for (const email of page.emails) {
      if (!seen.has(email.id)) {
        seen.add(email.id);
        emails.push(email);
      }
    }
  }
  const total = pages[pages.length - 1]?.total ?? 0;

  return { query, emails, total, accountId };
}

export function useEmailMutation() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (payload: { update?: Record<string, EmailPatch>; destroy?: string[] }) => {
      await jmap.call("Email/set", { accountId, ...payload });
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["emails"] });
      void queryClient.invalidateQueries({ queryKey: ["mailboxes"] });
      void queryClient.invalidateQueries({ queryKey: ["email"] });
    },
  });
}
