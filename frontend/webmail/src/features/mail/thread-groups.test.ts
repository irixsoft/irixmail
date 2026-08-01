import { describe, expect, it } from "vitest";
import type { EmailListItem } from "@/lib/mail-types";
import { groupByThread } from "./thread-groups";

function email(id: string, threadId: string, receivedAt: string, keywords: Record<string, boolean> = {}): EmailListItem {
  return { id, threadId, mailboxIds: { m: true }, keywords, receivedAt, subject: id, hasAttachment: false };
}

describe("groupByThread", () => {
  it("collapses a thread to one group faced by its newest email", () => {
    const groups = groupByThread([
      email("old", "t1", "2026-01-01T10:00:00Z"),
      email("new", "t1", "2026-01-02T10:00:00Z"),
      email("solo", "t2", "2026-01-03T10:00:00Z"),
    ]);
    expect(groups.map((group) => group.newest.id)).toEqual(["solo", "new"]);
    expect(groups[1]!.emailIds).toEqual(["new", "old"]);
    expect(groups[1]!.count).toBe(2);
  });

  it("derives unread, flagged and attachment from any member", () => {
    const groups = groupByThread([
      email("a", "t", "2026-01-01T10:00:00Z", { $seen: true, $flagged: true }),
      { ...email("b", "t", "2026-01-02T10:00:00Z", { $seen: true }), hasAttachment: true },
    ]);
    expect(groups[0]!.hasUnread).toBe(false);
    expect(groups[0]!.hasFlagged).toBe(true);
    expect(groups[0]!.hasAttachment).toBe(true);
  });

  it("marks unread when any member lacks the seen keyword", () => {
    const groups = groupByThread([
      email("a", "t", "2026-01-01T10:00:00Z", { $seen: true }),
      email("b", "t", "2026-01-02T10:00:00Z"),
    ]);
    expect(groups[0]!.hasUnread).toBe(true);
  });

  it("keeps result order stable by newest member desc", () => {
    const groups = groupByThread([
      email("a", "t1", "2026-01-05T10:00:00Z"),
      email("b", "t2", "2026-01-04T10:00:00Z"),
      email("c", "t1", "2026-01-01T10:00:00Z"),
    ]);
    expect(groups.map((group) => group.threadId)).toEqual(["t1", "t2"]);
  });
});
