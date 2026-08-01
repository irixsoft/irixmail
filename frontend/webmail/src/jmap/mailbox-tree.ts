import type { Mailbox } from "@/lib/mail-types";

export interface MailboxNode {
  mailbox: Mailbox;
  children: MailboxNode[];
}

export interface FlatMailbox {
  node: MailboxNode;
  depth: number;
}

const ROLE_RANK: Record<string, number> = {
  inbox: 0,
  drafts: 1,
  sent: 2,
  junk: 3,
  archive: 4,
  trash: 5,
};

function rank(mailbox: Mailbox): number {
  return mailbox.role != null && mailbox.role in ROLE_RANK ? ROLE_RANK[mailbox.role]! : 100;
}

function compare(a: Mailbox, b: Mailbox): number {
  return rank(a) - rank(b) || a.name.localeCompare(b.name);
}

export function buildMailboxTree(mailboxes: Mailbox[]): MailboxNode[] {
  const ids = new Set(mailboxes.map((mailbox) => mailbox.id));
  const nodes = new Map<string, MailboxNode>(
    mailboxes.map((mailbox) => [mailbox.id, { mailbox, children: [] }]),
  );
  const roots: MailboxNode[] = [];
  for (const mailbox of mailboxes) {
    const node = nodes.get(mailbox.id)!;
    if (mailbox.parentId != null && ids.has(mailbox.parentId)) {
      nodes.get(mailbox.parentId)!.children.push(node);
    } else {
      roots.push(node);
    }
  }
  const sortNodes = (list: MailboxNode[]) => {
    list.sort((a, b) => compare(a.mailbox, b.mailbox));
    for (const node of list) sortNodes(node.children);
  };
  sortNodes(roots);
  return roots;
}

export function flattenTree(tree: MailboxNode[], depth = 0): FlatMailbox[] {
  return tree.flatMap((node) => [{ node, depth }, ...flattenTree(node.children, depth + 1)]);
}
