import * as React from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  Avatar,
  AvatarFallback,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  ErrorState,
  Skeleton,
  cn,
} from "@irixmail/shared";
import {
  Archive,
  ArrowLeft,
  Forward as ForwardIcon,
  FolderInput,
  ImageOff,
  Mail,
  MoreHorizontal,
  Reply,
  ReplyAll,
  ShieldAlert,
  Star,
  Tag,
  Trash2,
} from "lucide-react";

import { useLayout } from "@/app/layout-store";
import { AttachmentList } from "@/components/attachment-list";
import { flaggedPatch, movePatch, seenPatch, tagPatch, updateMap } from "@/jmap/mutations";
import { threadCalls } from "@/jmap/requests";
import { TAG_PALETTE, loadTagDefinitions, messageTagIds } from "@/jmap/tags";
import { useJmap, useJmapSession } from "@/lib/jmap";
import { addressList, formatDateTime, senderName } from "@/lib/format";
import { mailboxLabel, type EmailFull } from "@/lib/mail-types";
import { EmailFrame } from "./email-frame";
import {
  blockExternalResources,
  plainTextToHtml,
  resolveCids,
  sanitizeEmailHtml,
  splitQuote,
  unblockExternalResources,
} from "./sanitize";
import { useShortcuts } from "@/features/shortcuts/use-shortcuts";
import { useEmailMutation } from "./use-email-list";
import { useMailboxes } from "./use-mailboxes";

const FULL_PROPS = [
  "id",
  "blobId",
  "threadId",
  "mailboxIds",
  "keywords",
  "from",
  "to",
  "cc",
  "replyTo",
  "messageId",
  "references",
  "subject",
  "sentAt",
  "receivedAt",
  "preview",
  "hasAttachment",
  "bodyValues",
  "textBody",
  "htmlBody",
  "attachments",
];

const BODY_PROPS = ["partId", "blobId", "size", "name", "type", "disposition", "cid"];

function extractBody(email: EmailFull): { content: string; isHtml: boolean } | null {
  const htmlPart = email.htmlBody?.[0];
  if (htmlPart?.partId) {
    const value = email.bodyValues?.[htmlPart.partId]?.value;
    if (value) return { content: value, isHtml: true };
  }
  const textPart = email.textBody?.[0];
  if (textPart?.partId) {
    const value = email.bodyValues?.[textPart.partId]?.value;
    if (value != null) return { content: value, isHtml: false };
  }
  return null;
}

function useCidUrls(email: EmailFull, accountId: string | undefined, token: string | null) {
  const jmap = useJmap();
  const [urls, setUrls] = React.useState<Record<string, string>>({});

  React.useEffect(() => {
    const inline = (email.attachments ?? []).filter(
      (part): part is typeof part & { cid: string; blobId: string } =>
        Boolean((part as { cid?: string }).cid) && Boolean(part.blobId),
    );
    if (inline.length === 0 || !accountId || !token) return;
    let cancelled = false;
    const created: string[] = [];
    void Promise.all(
      inline.map(async (part) => {
        const response = await fetch(jmap.downloadUrl(accountId, part.blobId, part.name ?? "inline"), {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (!response.ok) return null;
        const url = URL.createObjectURL(await response.blob());
        created.push(url);
        return [(part as { cid: string }).cid, url] as const;
      }),
    ).then((entries) => {
      if (cancelled) return;
      setUrls(Object.fromEntries(entries.filter((entry): entry is [string, string] => Boolean(entry))));
    });
    return () => {
      cancelled = true;
      for (const url of created) URL.revokeObjectURL(url);
    };
  }, [email.id, accountId, token, jmap, email.attachments]);

  return urls;
}

function MessageCard({
  email,
  expanded,
  onToggle,
  accountId,
  token,
}: {
  email: EmailFull;
  expanded: boolean;
  onToggle: () => void;
  accountId: string | undefined;
  token: string | null;
}) {
  const [allowImages, setAllowImages] = React.useState(false);
  const [showQuote, setShowQuote] = React.useState(false);
  const cidUrls = useCidUrls(email, accountId, token);
  const mutation = useEmailMutation();

  const unread = !email.keywords["$seen"];
  React.useEffect(() => {
    if (expanded && unread) {
      mutation.mutate({ update: updateMap([email.id], seenPatch(true)) });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [expanded, unread, email.id]);

  const body = React.useMemo(() => extractBody(email), [email]);
  const processed = React.useMemo(() => {
    if (!body) return null;
    if (!body.isHtml) {
      return { html: plainTextToHtml(body.content), blocked: 0, quote: null as string | null };
    }
    const sanitized = sanitizeEmailHtml(body.content);
    const withCids = resolveCids(sanitized, cidUrls);
    const { main, quote } = splitQuote(withCids);
    const visible = showQuote && quote ? main + quote : main;
    const { html, blockedCount } = blockExternalResources(visible);
    return { html: allowImages ? unblockExternalResources(html) : html, blocked: blockedCount, quote };
  }, [body, cidUrls, allowImages, showQuote]);

  const sender = senderName(email.from) || "(unknown)";
  const attachments = (email.attachments ?? []).filter(
    (part) => !(part as { cid?: string }).cid || part.disposition === "attachment",
  );

  if (!expanded) {
    return (
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-3 rounded-lg border bg-card px-3 py-2.5 text-left transition-colors hover:bg-accent/40"
      >
        <Avatar className="size-7">
          <AvatarFallback className="bg-secondary text-[10px]">{sender.slice(0, 2).toUpperCase()}</AvatarFallback>
        </Avatar>
        <span className={cn("w-40 shrink-0 truncate text-sm", unread && "font-semibold")}>{sender}</span>
        <span className="min-w-0 flex-1 truncate text-[13px] text-muted-foreground">{email.preview}</span>
        <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
          {formatDateTime(email.receivedAt)}
        </span>
      </button>
    );
  }

  return (
    <article className="overflow-hidden rounded-lg border bg-card">
      <header className="flex items-start gap-3 px-4 pb-2 pt-3">
        <Avatar className="size-9">
          <AvatarFallback className="bg-primary/15 text-xs font-medium text-primary">
            {sender.slice(0, 2).toUpperCase()}
          </AvatarFallback>
        </Avatar>
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <button type="button" onClick={onToggle} className="truncate text-sm font-semibold">
              {sender}
            </button>
            <span className="truncate font-mono text-[11px] text-muted-foreground">
              {email.from?.[0]?.email}
            </span>
          </div>
          <div className="truncate text-[12px] text-muted-foreground">
            to {addressList(email.to) || "me"}
            {email.cc?.length ? `, cc ${addressList(email.cc)}` : ""}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <span className="font-mono text-[11px] text-muted-foreground">{formatDateTime(email.receivedAt)}</span>
          <button
            type="button"
            aria-label="Toggle star"
            onClick={() => mutation.mutate({ update: updateMap([email.id], flaggedPatch(!email.keywords["$flagged"])) })}
            className="rounded p-1 hover:bg-accent"
          >
            <Star className={cn("size-4", email.keywords["$flagged"] && "fill-warning text-warning")} />
          </button>
        </div>
      </header>

      {processed?.blocked ? (
        <div className="mx-4 mb-2 flex items-center gap-2 rounded-md bg-accent px-3 py-1.5 text-[12px]">
          <ImageOff className="size-3.5 shrink-0" />
          <span className="flex-1">Remote images blocked for privacy.</span>
          <Button variant="ghost" size="sm" onClick={() => setAllowImages(true)}>
            Show images
          </Button>
        </div>
      ) : null}

      {body == null ? (
        <p className="px-4 pb-4 text-sm text-muted-foreground">This message has no readable body.</p>
      ) : body.isHtml ? (
        <EmailFrame html={processed?.html ?? ""} allowExternal={allowImages} />
      ) : (
        <div
          className="px-4 pb-3 text-sm leading-relaxed [overflow-wrap:break-word]"
          // eslint-disable-next-line react/no-danger
          dangerouslySetInnerHTML={{ __html: processed?.html ?? "" }}
        />
      )}

      {processed?.quote ? (
        <button
          type="button"
          aria-label={showQuote ? "Hide quoted text" : "Show quoted text"}
          onClick={() => setShowQuote((current) => !current)}
          className="mx-4 mb-3 rounded-full border bg-secondary px-2.5 py-0.5 font-mono text-[11px] text-muted-foreground hover:bg-accent"
        >
          •••
        </button>
      ) : null}

      {attachments.length > 0 && accountId ? (
        <div className="border-t px-4 py-3">
          <AttachmentList accountId={accountId} attachments={attachments} />
        </div>
      ) : null}
    </article>
  );
}

export function ConversationView() {
  const { mailboxId, emailId } = useParams<{ mailboxId: string; emailId: string }>();
  const navigate = useNavigate();
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const mutation = useEmailMutation();
  const { readingPane } = useLayout();
  const { list: mailboxes, byRole } = useMailboxes();
  const tagDefinitions = React.useMemo(loadTagDefinitions, []);
  const [expandedIds, setExpandedIds] = React.useState<Set<string>>(new Set());
  const token = React.useMemo(() => {
    try {
      const raw = localStorage.getItem("irixmail.auth");
      return raw ? ((JSON.parse(raw) as { token?: string }).token ?? null) : null;
    } catch {
      return null;
    }
  }, []);

  const anchorQuery = useQuery({
    queryKey: ["email", accountId, emailId],
    enabled: Boolean(accountId && emailId),
    queryFn: () =>
      jmap.call<{ list: EmailFull[] }>("Email/get", {
        accountId,
        ids: [emailId],
        properties: ["id", "threadId"],
      }),
  });
  const threadId = anchorQuery.data?.list?.[0]?.threadId;

  const threadQuery = useQuery({
    queryKey: ["thread", accountId, threadId],
    enabled: Boolean(accountId && threadId),
    queryFn: async () => {
      const response = await jmap.request(
        threadCalls(accountId!, threadId!, {
          properties: FULL_PROPS,
          bodyProperties: BODY_PROPS,
          fetchBodies: true,
        }),
      );
      const get = response.methodResponses.find(([name, , callId]) => name === "Email/get" && callId === "g");
      const list = ((get?.[1]["list"] as EmailFull[] | undefined) ?? []).slice();
      list.sort((a, b) => Date.parse(a.receivedAt ?? "") - Date.parse(b.receivedAt ?? ""));
      return list;
    },
  });

  const emails = React.useMemo(() => threadQuery.data ?? [], [threadQuery.data]);
  const latest = emails[emails.length - 1];

  React.useEffect(() => {
    if (latest) setExpandedIds(new Set([latest.id]));
  }, [threadId, latest?.id]);

  const allIds = emails.map((email) => email.id);
  const anchor = latest;

  useShortcuts({
    r: () => latest && navigate("/compose", { state: { mode: "reply", emailId: latest.id } }),
    a: () => latest && navigate("/compose", { state: { mode: "replyAll", emailId: latest.id } }),
    f: () => latest && navigate("/compose", { state: { mode: "forward", emailId: latest.id } }),
    Escape: () => navigate(`/${mailboxId}`),
  });

  if (!emailId) return null;
  if (anchorQuery.isError || threadQuery.isError) {
    return <ErrorState title="Could not load the conversation" onRetry={() => void threadQuery.refetch()} />;
  }
  if (!threadQuery.data) {
    return (
      <div className="space-y-3 p-4">
        <Skeleton className="h-8 w-2/3" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  const compose = (mode: string) =>
    navigate("/compose", { state: { mode, emailId: latest?.id ?? emailId } });
  const moveAll = (targetId: string) => {
    mutation.mutate({ update: updateMap(allIds, movePatch(targetId)) });
    navigate(`/${mailboxId}`);
  };
  const anyUnread = emails.some((email) => !email.keywords["$seen"]);
  const anchorTags = latest ? messageTagIds(latest.keywords) : [];

  return (
    <div className="flex h-full min-w-0 flex-col">
      <header className="flex h-12 shrink-0 items-center gap-1 border-b px-3">
        <Button
          variant="ghost"
          size="icon"
          aria-label="Back to list"
          className={cn(readingPane !== "off" && "md:hidden")}
          onClick={() => navigate(`/${mailboxId}`)}
        >
          <ArrowLeft className="size-4" />
        </Button>
        <Button variant="ghost" size="sm" onClick={() => compose("reply")}>
          <Reply className="size-4" /> Reply
        </Button>
        <Button variant="ghost" size="sm" className="hidden md:inline-flex" onClick={() => compose("replyAll")}>
          <ReplyAll className="size-4" /> Reply all
        </Button>
        <Button variant="ghost" size="sm" onClick={() => compose("forward")}>
          <ForwardIcon className="size-4" /> Forward
        </Button>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="icon"
          aria-label="Archive"
          onClick={() => byRole["archive"] && moveAll(byRole["archive"].id)}
        >
          <Archive className="size-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label={anchor && Object.keys(anchor.mailboxIds).includes(byRole["junk"]?.id ?? "") ? "Not spam" : "Spam"}
          onClick={() => byRole["junk"] && moveAll(byRole["junk"].id)}
          className="hidden md:inline-flex"
        >
          <ShieldAlert className="size-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Delete"
          onClick={() => byRole["trash"] && moveAll(byRole["trash"].id)}
        >
          <Trash2 className="size-4" />
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label="More actions">
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-48">
            <DropdownMenuItem onClick={() => mutation.mutate({ update: updateMap(allIds, seenPatch(anyUnread)) })}>
              <Mail className="size-4" /> Mark {anyUnread ? "read" : "unread"}
            </DropdownMenuItem>
            {mailboxes
              .filter((mailbox) => mailbox.id !== mailboxId)
              .slice(0, 8)
              .map((mailbox) => (
                <DropdownMenuItem key={mailbox.id} onClick={() => moveAll(mailbox.id)}>
                  <FolderInput className="size-4" /> Move to {mailboxLabel(mailbox)}
                </DropdownMenuItem>
              ))}
            {tagDefinitions.map((tag) => (
              <DropdownMenuItem
                key={tag.id}
                onClick={() =>
                  latest &&
                  mutation.mutate({
                    update: updateMap([latest.id], tagPatch(tag.id, !anchorTags.includes(tag.id))),
                  })
                }
              >
                <Tag className="size-4" />
                <span className={cn("mr-1 size-2 rounded-full", TAG_PALETTE[tag.color]?.dot)} />
                {tag.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-2 p-4">
          <h1 className="text-lg font-semibold leading-snug">{latest?.subject || "(no subject)"}</h1>
          {emails.map((email) => (
            <MessageCard
              key={email.id}
              email={email}
              expanded={expandedIds.has(email.id)}
              onToggle={() =>
                setExpandedIds((current) => {
                  const next = new Set(current);
                  if (next.has(email.id)) next.delete(email.id);
                  else next.add(email.id);
                  return next;
                })
              }
              accountId={accountId}
              token={token}
            />
          ))}

          <div className="mt-2 flex items-center gap-2">
            <Button size="sm" onClick={() => compose("reply")}>
              <Reply className="size-4" /> Reply
            </Button>
            <Button variant="outline" size="sm" onClick={() => compose("replyAll")}>
              <ReplyAll className="size-4" /> Reply all
            </Button>
            <Button variant="outline" size="sm" onClick={() => compose("forward")}>
              <ForwardIcon className="size-4" /> Forward
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
