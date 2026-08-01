import type { EmailListItem } from "@/lib/mail-types";

export interface ThreadGroup {
  threadId: string;
  newest: EmailListItem;
  emailIds: string[];
  count: number;
  hasUnread: boolean;
  hasFlagged: boolean;
  hasAttachment: boolean;
}

function received(email: EmailListItem): number {
  return email.receivedAt ? Date.parse(email.receivedAt) : 0;
}

export function groupByThread(emails: EmailListItem[]): ThreadGroup[] {
  const groups = new Map<string, EmailListItem[]>();
  for (const email of emails) {
    const members = groups.get(email.threadId);
    if (members) members.push(email);
    else groups.set(email.threadId, [email]);
  }
  const result: ThreadGroup[] = [];
  for (const members of groups.values()) {
    members.sort((a, b) => received(b) - received(a));
    const newest = members[0]!;
    result.push({
      threadId: newest.threadId,
      newest,
      emailIds: members.map((email) => email.id),
      count: members.length,
      hasUnread: members.some((email) => !email.keywords["$seen"]),
      hasFlagged: members.some((email) => Boolean(email.keywords["$flagged"])),
      hasAttachment: members.some((email) => Boolean(email.hasAttachment)),
    });
  }
  result.sort((a, b) => received(b.newest) - received(a.newest));
  return result;
}
