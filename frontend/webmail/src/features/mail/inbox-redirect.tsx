import * as React from "react";
import { useNavigate } from "react-router-dom";
import { Skeleton } from "@irixmail/shared";

import { useMailboxes } from "./use-mailboxes";

export function InboxRedirect() {
  const { byRole, list, query } = useMailboxes();
  const navigate = useNavigate();
  const inboxId = byRole["inbox"]?.id ?? list[0]?.id;

  React.useEffect(() => {
    if (inboxId) navigate(`/${inboxId}`, { replace: true });
  }, [inboxId, navigate]);

  if (query.isError) return null;
  return (
    <div className="space-y-2 p-4">
      <Skeleton className="h-10 w-full" />
      <Skeleton className="h-10 w-full" />
    </div>
  );
}
