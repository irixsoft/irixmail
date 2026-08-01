import * as React from "react";
import { NavLink, useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  Button,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  cn,
} from "@irixmail/shared";
import {
  Archive,
  ChevronRight,
  FileText,
  Folder,
  Inbox,
  PanelLeftClose,
  PanelLeftOpen,
  Send,
  ShieldAlert,
  SquarePen,
  Tag,
  Trash2,
  type LucideIcon,
} from "lucide-react";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { buildMailboxTree, flattenTree, type MailboxNode } from "@/jmap/mailbox-tree";
import { TAG_PALETTE, loadTagDefinitions, tagKeyword } from "@/jmap/tags";
import { mailboxLabel, type Mailbox } from "@/lib/mail-types";

const ROLE_ICON: Record<string, LucideIcon> = {
  inbox: Inbox,
  drafts: FileText,
  sent: Send,
  junk: ShieldAlert,
  archive: Archive,
  trash: Trash2,
};

function FolderRow({ node, depth, onNavigate }: { node: MailboxNode; depth: number; onNavigate?: () => void }) {
  const [expanded, setExpanded] = React.useState(true);
  const mailbox = node.mailbox;
  const Icon = (mailbox.role && ROLE_ICON[mailbox.role]) || Folder;
  return (
    <>
      <NavLink
        to={`/${mailbox.id}`}
        onClick={onNavigate}
        style={{ paddingLeft: 8 + depth * 14 }}
        className={({ isActive }) =>
          cn(
            "group relative flex items-center gap-2.5 rounded-md py-1.5 pr-2 text-sm transition-colors",
            isActive
              ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground shadow-[inset_2.5px_0_0_0_var(--primary)]"
              : "text-muted-foreground hover:bg-sidebar-accent/50 hover:text-foreground",
          )
        }
      >
        {node.children.length > 0 ? (
          <button
            type="button"
            aria-label={expanded ? "Collapse folder" : "Expand folder"}
            onClick={(event) => {
              event.preventDefault();
              setExpanded((current) => !current);
            }}
            className="-ml-1 flex size-4 items-center justify-center rounded hover:bg-sidebar-accent"
          >
            <ChevronRight className={cn("size-3.5 transition-transform", expanded && "rotate-90")} />
          </button>
        ) : (
          <Icon className="size-4 shrink-0" />
        )}
        <span className="flex-1 truncate">{mailboxLabel(mailbox)}</span>
        {mailbox.unreadEmails > 0 ? (
          <span className="font-mono text-[11px] tabular-nums text-primary">{mailbox.unreadEmails}</span>
        ) : null}
      </NavLink>
      {expanded
        ? node.children.map((child) => (
            <FolderRow key={child.mailbox.id} node={child} depth={depth + 1} onNavigate={onNavigate} />
          ))
        : null}
    </>
  );
}

function useMailboxTree() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  const query = useQuery({
    queryKey: ["mailboxes", accountId],
    queryFn: () => jmap.call<{ list: Mailbox[] }>("Mailbox/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });

  const tree = React.useMemo(() => buildMailboxTree(query.data?.list ?? []), [query.data]);
  return { tree, accountId, isLoading: query.isLoading };
}

function RailFolder({ mailbox }: { mailbox: Mailbox }) {
  const Icon = (mailbox.role && ROLE_ICON[mailbox.role]) || Folder;
  const label = mailboxLabel(mailbox);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <NavLink
          to={`/${mailbox.id}`}
          aria-label={label}
          className={({ isActive }) =>
            cn(
              "relative flex size-9 shrink-0 items-center justify-center rounded-md outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
              isActive
                ? "bg-primary/12 text-primary"
                : "text-muted-foreground hover:bg-sidebar-accent/50 hover:text-foreground",
            )
          }
        >
          <Icon className="size-4.5" />
          {mailbox.unreadEmails > 0 ? (
            <span className="absolute right-1.5 top-1.5 size-1.5 rounded-full bg-primary" />
          ) : null}
        </NavLink>
      </TooltipTrigger>
      <TooltipContent side="right">
        {mailbox.unreadEmails > 0 ? `${label} · ${mailbox.unreadEmails}` : label}
      </TooltipContent>
    </Tooltip>
  );
}

export function FolderRail({ onExpand }: { onExpand: () => void }) {
  const navigate = useNavigate();
  const { tree, accountId, isLoading } = useMailboxTree();
  const mailboxes = React.useMemo(() => flattenTree(tree).map((entry) => entry.node.mailbox), [tree]);

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-sidebar-border bg-sidebar p-1.5">
        <Button variant="ghost" size="icon" aria-label="Expand folder pane" onClick={onExpand}>
          <PanelLeftOpen className="size-4" />
        </Button>
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label="New mail"
              onClick={() => navigate("/compose")}
              className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-primary to-primary/80 text-primary-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <SquarePen className="size-4" />
            </button>
          </TooltipTrigger>
          <TooltipContent side="right">New mail</TooltipContent>
        </Tooltip>
        <nav className="flex min-h-0 flex-1 flex-col items-center gap-0.5 overflow-y-auto pt-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {isLoading || !accountId
            ? Array.from({ length: 6 }).map((_, index) => <Skeleton key={index} className="size-9 shrink-0 rounded-md" />)
            : mailboxes.map((mailbox) => <RailFolder key={mailbox.id} mailbox={mailbox} />)}
        </nav>
      </div>
    </TooltipProvider>
  );
}

export function FolderPane({ onCollapse, onNavigate }: { onCollapse?: () => void; onNavigate?: () => void }) {
  const navigate = useNavigate();
  const tags = React.useMemo(loadTagDefinitions, []);
  const { tree, accountId, isLoading } = useMailboxTree();

  return (
    <div className="flex h-full flex-col bg-sidebar">
      <div className="flex items-center gap-2 p-3 pb-2">
        <Button
          onClick={() => navigate("/compose")}
          className="flex-1 justify-start gap-2 bg-gradient-to-br from-primary to-primary/80 shadow-sm"
        >
          <SquarePen className="size-4" /> New mail
        </Button>
        {onCollapse ? (
          <Button variant="ghost" size="icon" aria-label="Collapse folder pane" onClick={onCollapse}>
            <PanelLeftClose className="size-4" />
          </Button>
        ) : null}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {isLoading || !accountId ? (
          <div className="space-y-1 p-1">
            {Array.from({ length: 6 }).map((_, index) => (
              <Skeleton key={index} className="h-7 w-full" />
            ))}
          </div>
        ) : (
          <nav className="flex flex-col gap-px">
            {tree.map((node) => (
              <FolderRow key={node.mailbox.id} node={node} depth={0} onNavigate={onNavigate} />
            ))}
          </nav>
        )}
        {tags.length > 0 ? (
          <div className="mt-4">
            <div className="flex items-center gap-1.5 px-2 pb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              <Tag className="size-3" /> Tags
            </div>
            <nav className="flex flex-col gap-px">
              {tags.map((tag) => (
                <NavLink
                  key={tag.id}
                  to={`/search?tag=${encodeURIComponent(tagKeyword(tag.id))}`}
                  onClick={onNavigate}
                  className="flex items-center gap-2.5 rounded-md px-2 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-sidebar-accent/50 hover:text-foreground"
                >
                  <span className={cn("size-2.5 rounded-full", TAG_PALETTE[tag.color]?.dot ?? "bg-muted-foreground")} />
                  <span className="flex-1 truncate">{tag.label}</span>
                </NavLink>
              ))}
            </nav>
          </div>
        ) : null}
      </div>
    </div>
  );
}
