import { Link } from "react-router-dom";
import { Skeleton } from "@irixmail/shared";
import { Mail } from "lucide-react";

import { formatListDate, senderName } from "@/lib/format";
import { useContactEmails } from "./use-contact-emails";

export function RecentEmails({ email }: { email: string }) {
  const query = useContactEmails(email);
  const emails = query.data ?? [];

  if (!email) return null;

  return (
    <section>
      <h3 className="pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        Recent email
      </h3>
      {query.isPending ? (
        <div className="space-y-1.5">
          {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-10 w-full" />
          ))}
        </div>
      ) : query.isError ? (
        <p className="text-sm text-muted-foreground">Could not load recent mail.</p>
      ) : emails.length === 0 ? (
        <p className="text-sm text-muted-foreground">No messages with this address yet.</p>
      ) : (
        <ul className="divide-y rounded-lg border">
          {emails.map((entry) => {
            const mailboxId = Object.keys(entry.mailboxIds ?? {})[0];
            const row = (
              <span className="flex min-w-0 items-center gap-2.5 px-3 py-2">
                <Mail className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm">{entry.subject || "(no subject)"}</span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {senderName(entry.from)}
                  </span>
                </span>
                <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">
                  {formatListDate(entry.receivedAt)}
                </span>
              </span>
            );
            return (
              <li key={entry.id}>
                {mailboxId ? (
                  <Link
                    to={`/${mailboxId}/${entry.id}`}
                    className="flex transition-colors hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60"
                  >
                    {row}
                  </Link>
                ) : (
                  <span className="flex">{row}</span>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
