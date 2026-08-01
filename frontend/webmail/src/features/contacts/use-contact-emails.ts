import { useQuery } from "@tanstack/react-query";

import { useJmap, useJmapSession } from "@/lib/jmap";
import type { EmailListItem } from "@/lib/mail-types";

const PROPERTIES = ["id", "threadId", "mailboxIds", "keywords", "from", "to", "subject", "receivedAt"];

export function useContactEmails(email: string, limit = 5) {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  return useQuery({
    queryKey: ["contact-emails", accountId, email.toLowerCase(), limit],
    enabled: Boolean(accountId) && Boolean(email),
    queryFn: async () => {
      const result = await jmap.call<{ ids: string[] }>("Email/query", {
        accountId,
        filter: { operator: "OR", conditions: [{ from: email }, { to: email }] },
        sort: [{ property: "receivedAt", isAscending: false }],
        limit,
      });
      const ids = result.ids ?? [];
      if (ids.length === 0) return [] as EmailListItem[];
      const emails = await jmap.call<{ list: EmailListItem[] }>("Email/get", {
        accountId,
        ids,
        properties: PROPERTIES,
      });
      const byId = new Map((emails.list ?? []).map((entry) => [entry.id, entry]));
      return ids.map((id) => byId.get(id)).filter((entry): entry is EmailListItem => Boolean(entry));
    },
  });
}
