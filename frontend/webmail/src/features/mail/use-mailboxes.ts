import { useQuery } from "@tanstack/react-query";

import { useJmap, useJmapSession } from "@/lib/jmap";
import type { Mailbox } from "@/lib/mail-types";

export function useMailboxes() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  const query = useQuery({
    queryKey: ["mailboxes", accountId],
    queryFn: () => jmap.call<{ list: Mailbox[] }>("Mailbox/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });

  const list = query.data?.list ?? [];
  const byId: Record<string, Mailbox> = {};
  const byRole: Record<string, Mailbox> = {};
  for (const mailbox of list) {
    byId[mailbox.id] = mailbox;
    if (mailbox.role) byRole[mailbox.role] = mailbox;
  }
  return { query, list, byId, byRole, accountId };
}
