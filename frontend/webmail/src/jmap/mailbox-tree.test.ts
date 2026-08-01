import { describe, expect, it } from "vitest";
import type { Mailbox } from "@/lib/mail-types";
import { buildMailboxTree, flattenTree } from "./mailbox-tree";

function box(id: string, name: string, role: string | null = null, parentId: string | null = null): Mailbox {
  return { id, name, role, parentId, sortOrder: 0, totalEmails: 0, unreadEmails: 0 };
}

describe("buildMailboxTree", () => {
  it("orders roles first then custom folders alphabetically", () => {
    const tree = buildMailboxTree([
      box("1", "Zebra"),
      box("2", "Trash", "trash"),
      box("3", "Inbox", "inbox"),
      box("4", "Alpha"),
      box("5", "Sent", "sent"),
    ]);
    expect(tree.map((node) => node.mailbox.id)).toEqual(["3", "5", "2", "4", "1"]);
  });

  it("nests children under their parent sorted by name", () => {
    const tree = buildMailboxTree([
      box("p", "Projects"),
      box("b", "Beta", null, "p"),
      box("a", "Alpha", null, "p"),
    ]);
    expect(tree).toHaveLength(1);
    expect(tree[0]!.children.map((node) => node.mailbox.name)).toEqual(["Alpha", "Beta"]);
  });

  it("treats a child of a missing parent as a root", () => {
    const tree = buildMailboxTree([box("x", "Orphan", null, "gone")]);
    expect(tree.map((node) => node.mailbox.id)).toEqual(["x"]);
  });
});

describe("flattenTree", () => {
  it("yields nodes depth-first with depths", () => {
    const tree = buildMailboxTree([
      box("p", "Projects"),
      box("a", "Alpha", null, "p"),
      box("i", "Inbox", "inbox"),
    ]);
    const flat = flattenTree(tree);
    expect(flat.map((entry) => [entry.node.mailbox.id, entry.depth])).toEqual([
      ["i", 0],
      ["p", 0],
      ["a", 1],
    ]);
  });
});
