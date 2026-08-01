import * as React from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Spinner, Textarea, cn, toast, useAuth } from "@irixmail/shared";
import { Paperclip, Send, Trash2, X } from "lucide-react";
import DOMPurify from "dompurify";

import { useMailboxes } from "@/features/mail/use-mailboxes";
import { plainTextToHtml } from "@/features/mail/sanitize";
import { useJmap } from "@/lib/jmap";
import { formatBytes } from "@/lib/format";
import type { EmailAddress, EmailFull, Identity } from "@/lib/mail-types";
import type { RichEditorValue } from "@/components/rich-editor";
import { RecipientField } from "./recipient-field";
import { dedupeRecipients, invalidRecipients, parseRecipients } from "./recipients";
import {
  attributionLine,
  quotedHtml,
  quotedHtmlFromText,
  quotedText,
  threadingHeaders,
  type ThreadingSource,
} from "./quote";

const RichEditor = React.lazy(() =>
  import("@/components/rich-editor").then((module) => ({ default: module.RichEditor })),
);

const RICH_PREF_KEY = "irixmail.compose.rich";
const SANITIZE_CONFIG = { USE_PROFILES: { html: true } };

const EDITOR_SHELL = cn(
  "[&>div]:rounded-none [&>div]:border-0 [&>div]:bg-transparent [&>div]:shadow-none",
  "[&>div]:focus-within:ring-0",
);

function readRichPref(): boolean {
  try {
    return window.localStorage.getItem(RICH_PREF_KEY) !== "0";
  } catch {
    return true;
  }
}

function isMac(): boolean {
  return typeof navigator !== "undefined" && /mac/i.test(navigator.userAgent);
}

interface AttachedFile {
  id: string;
  name: string;
  size: number;
  type: string;
  blobId?: string;
  status: "uploading" | "done" | "error";
}

interface ComposeState {
  mode?: "reply" | "replyAll" | "forward";
  emailId?: string;
}

interface FieldRowProps {
  label: string;
  htmlFor?: string;
  trailing?: React.ReactNode;
  children: React.ReactNode;
}

function FieldRow({ label, htmlFor, trailing, children }: FieldRowProps) {
  return (
    <div className="flex items-start gap-3 border-b border-border/70 px-4 transition-colors focus-within:bg-muted/25">
      <label
        htmlFor={htmlFor}
        className="w-11 shrink-0 pt-2.5 text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground"
      >
        {label}
      </label>
      <div className="min-w-0 flex-1 py-1">{children}</div>
      {trailing ? <div className="shrink-0 pt-2">{trailing}</div> : null}
    </div>
  );
}

export function ComposePage() {
  const navigate = useNavigate();
  const location = useLocation();
  const state = (location.state as ComposeState | null) ?? {};
  const [searchParams] = useSearchParams();
  const jmap = useJmap();
  const queryClient = useQueryClient();
  const { username } = useAuth();
  const { byRole, query: mailboxesQuery, accountId } = useMailboxes();

  const [to, setTo] = React.useState<EmailAddress[]>([]);
  const [cc, setCc] = React.useState<EmailAddress[]>([]);
  const [bcc, setBcc] = React.useState<EmailAddress[]>([]);
  const [subject, setSubject] = React.useState("");
  const [bodyText, setBodyText] = React.useState("");
  const [richMode, setRichMode] = React.useState(readRichPref);
  const [rich, setRich] = React.useState<RichEditorValue>({ html: "", text: "" });
  const [seedHtml, setSeedHtml] = React.useState("");
  const [showCc, setShowCc] = React.useState(false);
  const [showBcc, setShowBcc] = React.useState(false);
  const [dragging, setDragging] = React.useState(false);
  const [attachments, setAttachments] = React.useState<AttachedFile[]>([]);
  const fileInputRef = React.useRef<HTMLInputElement>(null);
  const dragDepth = React.useRef(0);
  const prefilled = React.useRef(false);

  const identitiesQuery = useQuery({
    queryKey: ["identities", accountId],
    queryFn: () => jmap.call<{ list: Identity[] }>("Identity/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });
  const identity = identitiesQuery.data?.list[0];

  const sourceQuery = useQuery({
    queryKey: ["email-source", accountId, state.emailId],
    enabled: Boolean(accountId) && Boolean(state.emailId) && Boolean(state.mode),
    queryFn: async () => {
      const result = await jmap.call<{ list: EmailFull[] }>("Email/get", {
        accountId,
        ids: [state.emailId],
        properties: [
          "from",
          "to",
          "cc",
          "replyTo",
          "subject",
          "sentAt",
          "receivedAt",
          "messageId",
          "references",
          "textBody",
          "htmlBody",
          "bodyValues",
        ],
        bodyProperties: ["partId", "type"],
        fetchTextBodyValues: true,
        fetchHTMLBodyValues: true,
      });
      return result.list[0] ?? null;
    },
  });

  React.useEffect(() => {
    const to = searchParams.get("to");
    if (!to || state.mode || prefilled.current) return;
    prefilled.current = true;
    setTo(dedupeRecipients(parseRecipients(to)));
    const prefillSubject = searchParams.get("subject");
    if (prefillSubject) setSubject(prefillSubject);
  }, [searchParams, state.mode]);

  React.useEffect(() => {
    const source = sourceQuery.data;
    if (!source || !state.mode || prefilled.current) return;
    prefilled.current = true;

    const replyTargets = source.replyTo?.length ? source.replyTo : (source.from ?? []);
    if (state.mode === "reply" || state.mode === "replyAll") {
      setTo(dedupeRecipients(replyTargets));
    }
    if (state.mode === "replyAll") {
      const targets = new Set(replyTargets.map((entry) => entry.email.toLowerCase()));
      const others = dedupeRecipients([...(source.to ?? []), ...(source.cc ?? [])]).filter(
        (entry) =>
          entry.email !== username && !targets.has(entry.email.toLowerCase()),
      );
      if (others.length > 0) {
        setCc(others);
        setShowCc(true);
      }
    }

    const baseSubject = source.subject ?? "";
    if (state.mode === "forward") {
      setSubject(baseSubject.startsWith("Fwd:") ? baseSubject : `Fwd: ${baseSubject}`);
    } else {
      setSubject(baseSubject.startsWith("Re:") ? baseSubject : `Re: ${baseSubject}`);
    }

    const line = attributionLine(source.from, source.sentAt ?? source.receivedAt ?? null);
    const textPart = source.textBody?.[0];
    const original = textPart?.partId ? source.bodyValues?.[textPart.partId]?.value : "";
    const htmlPart = source.htmlBody?.[0];
    const originalHtml = htmlPart?.partId ? source.bodyValues?.[htmlPart.partId]?.value : "";
    if (originalHtml) {
      if (original) setBodyText(quotedText(line, original));
      setSeedHtml(quotedHtml(line, originalHtml));
      setRichMode(true);
    } else if (original) {
      setBodyText(quotedText(line, original));
      setSeedHtml(quotedHtmlFromText(line, original));
    }
  }, [sourceQuery.data, state.mode, username]);

  const switchMode = () => {
    if (richMode) {
      setBodyText(rich.text);
      setRichMode(false);
    } else {
      setSeedHtml(plainTextToHtml(bodyText));
      setRichMode(true);
    }
    try {
      window.localStorage.setItem(RICH_PREF_KEY, richMode ? "0" : "1");
    } catch {
      // storage unavailable, the choice just does not persist
    }
  };

  const updateAttachment = (id: string, patch: Partial<AttachedFile>) =>
    setAttachments((prev) => prev.map((item) => (item.id === id ? { ...item, ...patch } : item)));

  const handleFiles = async (files: FileList | null) => {
    if (!files || !accountId) return;
    for (const file of Array.from(files)) {
      const id = crypto.randomUUID();
      setAttachments((prev) => [
        ...prev,
        {
          id,
          name: file.name,
          size: file.size,
          type: file.type || "application/octet-stream",
          status: "uploading",
        },
      ]);
      try {
        const result = await jmap.uploadBlob(accountId, file);
        updateAttachment(id, {
          blobId: result.blobId,
          type: result.type || file.type || "application/octet-stream",
          size: result.size || file.size,
          status: "done",
        });
      } catch {
        updateAttachment(id, { status: "error" });
        toast.error(`Could not upload ${file.name}`);
      }
    }
  };

  const buildBody = (): Record<string, unknown> => {
    if (!richMode) {
      return {
        bodyValues: { body: { value: bodyText } },
        textBody: [{ partId: "body", type: "text/plain" }],
      };
    }
    const html = DOMPurify.sanitize(rich.html, SANITIZE_CONFIG);
    return {
      bodyValues: { text: { value: rich.text }, html: { value: html } },
      textBody: [{ partId: "text", type: "text/plain" }],
      htmlBody: [{ partId: "html", type: "text/html" }],
    };
  };

  const buildDraft = (draftsId: string): Record<string, unknown> => {
    const ready = attachments.filter((item) => item.status === "done" && item.blobId);
    return {
      mailboxIds: { [draftsId]: true },
      keywords: { $draft: true, $seen: true },
      ...threadingHeaders(state.mode, sourceQuery.data as ThreadingSource | null),
      from: identity ? [{ name: identity.name, email: identity.email }] : undefined,
      to,
      cc: cc.length > 0 ? cc : undefined,
      bcc: bcc.length > 0 ? bcc : undefined,
      subject,
      ...buildBody(),
      attachments: ready.length
        ? ready.map((item) => ({
            blobId: item.blobId,
            type: item.type,
            name: item.name,
            size: item.size,
            disposition: "attachment",
          }))
        : undefined,
    };
  };

  const saveDraft = useMutation({
    mutationFn: async () => {
      const draftsId = byRole["drafts"]?.id;
      if (!draftsId) throw new Error("no drafts mailbox");
      await jmap.call("Email/set", { accountId, create: { draft: buildDraft(draftsId) } });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["emails"] });
      void queryClient.invalidateQueries({ queryKey: ["mailboxes"] });
      toast.success("Draft saved");
      navigate(-1);
    },
    onError: () => toast.error("Could not save the draft"),
  });

  const send = useMutation({
    mutationFn: async () => {
      const draftsId = byRole["drafts"]?.id;
      const sentId = byRole["sent"]?.id;
      if (!draftsId || !identity) throw new Error("missing draft mailbox or identity");
      const response = await jmap.request([
        ["Email/set", { accountId, create: { draft: buildDraft(draftsId) } }, "create"],
        [
          "EmailSubmission/set",
          {
            accountId,
            create: { send: { emailId: "#draft", identityId: identity.id } },
            onSuccessUpdateEmail: sentId
              ? { "#send": { mailboxIds: { [sentId]: true }, "keywords/$draft": null } }
              : undefined,
          },
          "submit",
        ],
      ]);
      const createArgs = response.methodResponses.find((entry) => entry[2] === "create")?.[1] as
        | { notCreated?: Record<string, unknown> }
        | undefined;
      if (createArgs?.notCreated && Object.keys(createArgs.notCreated).length > 0) {
        throw new Error("draft rejected");
      }
      const submitArgs = response.methodResponses.find((entry) => entry[2] === "submit")?.[1] as
        | { notCreated?: Record<string, unknown> }
        | undefined;
      if (submitArgs?.notCreated && Object.keys(submitArgs.notCreated).length > 0) {
        throw new Error("submission rejected");
      }
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["emails"] });
      void queryClient.invalidateQueries({ queryKey: ["mailboxes"] });
      toast.success("Message sent");
      navigate(-1);
    },
    onError: () => toast.error("Could not send the message"),
  });

  const onSend = () => {
    if (send.isPending) return;
    if (attachments.some((item) => item.status === "uploading")) {
      toast.error("Wait for attachments to finish uploading");
      return;
    }
    if (to.length === 0) {
      toast.error("Add at least one recipient");
      return;
    }
    const invalid = invalidRecipients([...to, ...cc, ...bcc]);
    if (invalid.length > 0) {
      toast.error(`${invalid[0]?.email ?? "A recipient"} is not a valid address`);
      return;
    }
    send.mutate();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      onSend();
    }
  };

  const uploading = attachments.some((item) => item.status === "uploading");
  const canSend = Boolean(accountId) && to.length > 0 && !send.isPending && !uploading;
  const title = subject.trim() || "New message";
  const sendHint = isMac() ? "⌘↵" : "Ctrl+↵";

  return (
    <div
      className="flex h-full min-h-0 w-full justify-center overflow-hidden sm:p-4"
      onKeyDown={onKeyDown}
    >
      <div
        className="relative flex h-full min-h-0 w-full max-w-3xl flex-col overflow-hidden border-border bg-card shadow-sm sm:rounded-xl sm:border"
        onDragEnter={(event) => {
          if (!event.dataTransfer.types.includes("Files")) return;
          dragDepth.current += 1;
          setDragging(true);
        }}
        onDragOver={(event) => {
          if (event.dataTransfer.types.includes("Files")) event.preventDefault();
        }}
        onDragLeave={() => {
          dragDepth.current = Math.max(0, dragDepth.current - 1);
          if (dragDepth.current === 0) setDragging(false);
        }}
        onDrop={(event) => {
          if (!event.dataTransfer.types.includes("Files")) return;
          event.preventDefault();
          dragDepth.current = 0;
          setDragging(false);
          void handleFiles(event.dataTransfer.files);
        }}
      >
        {dragging ? (
          <div className="pointer-events-none absolute inset-2 z-20 grid place-items-center rounded-lg border-2 border-dashed border-primary/50 bg-background/85 backdrop-blur-[2px]">
            <div className="flex flex-col items-center gap-2 text-sm text-muted-foreground">
              <Paperclip className="size-5 text-primary" />
              Drop files to attach
            </div>
          </div>
        ) : null}

        <header className="flex h-14 shrink-0 items-center gap-3 border-b border-border/70 px-4">
          <span aria-hidden="true" className="h-6 w-1 shrink-0 rounded-full bg-primary/70" />
          <div className="min-w-0 flex-1">
            <h1 className="truncate text-sm font-semibold tracking-tight">{title}</h1>
            {identity ? (
              <p className="truncate font-mono text-[11px] text-muted-foreground">
                {identity.email}
              </p>
            ) : null}
          </div>
          <Button variant="ghost" size="icon" aria-label="Close" onClick={() => navigate(-1)}>
            <X className="size-4" />
          </Button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <FieldRow
            label="To"
            htmlFor="compose-to"
            trailing={
              !showCc || !showBcc ? (
                <div className="flex items-center gap-2 text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground">
                  {!showCc ? (
                    <button
                      type="button"
                      className="transition-colors hover:text-primary"
                      onClick={() => setShowCc(true)}
                    >
                      Cc
                    </button>
                  ) : null}
                  {!showBcc ? (
                    <button
                      type="button"
                      className="transition-colors hover:text-primary"
                      onClick={() => setShowBcc(true)}
                    >
                      Bcc
                    </button>
                  ) : null}
                </div>
              ) : null
            }
          >
            <RecipientField
              inputId="compose-to"
              label="To"
              value={to}
              onChange={setTo}
              placeholder="recipient@example.com"
              autoFocus
            />
          </FieldRow>

          {showCc ? (
            <FieldRow label="Cc" htmlFor="compose-cc">
              <RecipientField inputId="compose-cc" label="Cc" value={cc} onChange={setCc} />
            </FieldRow>
          ) : null}

          {showBcc ? (
            <FieldRow label="Bcc" htmlFor="compose-bcc">
              <RecipientField inputId="compose-bcc" label="Bcc" value={bcc} onChange={setBcc} />
            </FieldRow>
          ) : null}

          <FieldRow label="Subj" htmlFor="compose-subject">
            <input
              id="compose-subject"
              value={subject}
              onChange={(event) => setSubject(event.target.value)}
              placeholder="Subject"
              className="w-full bg-transparent py-1.5 text-sm font-medium outline-none placeholder:font-normal placeholder:text-muted-foreground/70"
            />
          </FieldRow>

          <div className="flex items-center justify-between border-b border-border/70 px-4 py-1.5">
            <span className="text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
              {richMode ? "Rich text" : "Plain text"}
            </span>
            <button
              type="button"
              className="text-[11px] text-muted-foreground transition-colors hover:text-primary"
              onClick={switchMode}
            >
              {richMode ? "Switch to plain text" : "Switch to rich text"}
            </button>
          </div>

          {richMode ? (
            <React.Suspense
              fallback={
                <div className="flex min-h-[21rem] items-center justify-center">
                  <Spinner className="size-5" />
                </div>
              }
            >
              <div className={EDITOR_SHELL}>
                <RichEditor initialHtml={seedHtml} onChange={setRich} />
              </div>
            </React.Suspense>
          ) : (
            <Textarea
              id="compose-body"
              aria-label="Message body"
              value={bodyText}
              onChange={(event) => setBodyText(event.target.value)}
              className="min-h-[21rem] resize-none rounded-none border-0 bg-transparent px-4 py-3 text-sm leading-relaxed shadow-none focus-visible:ring-0"
            />
          )}

          {attachments.length > 0 ? (
            <div className="flex flex-wrap gap-2 border-t border-border/70 px-4 py-3">
              {attachments.map((item) => (
                <div
                  key={item.id}
                  className={cn(
                    "flex items-center gap-2 rounded-md border border-border/70 bg-muted/40 px-2 py-1 text-xs",
                    item.status === "error" && "border-destructive/50 bg-destructive/10 text-destructive",
                  )}
                >
                  {item.status === "uploading" ? (
                    <Spinner className="size-3.5" />
                  ) : (
                    <Paperclip className="size-3.5 text-muted-foreground" />
                  )}
                  <span className="max-w-[12rem] truncate">{item.name}</span>
                  <span className="font-mono text-[10px] text-muted-foreground">
                    {formatBytes(item.size)}
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove ${item.name}`}
                    onClick={() =>
                      setAttachments((prev) => prev.filter((entry) => entry.id !== item.id))
                    }
                    className="text-muted-foreground/70 transition-colors hover:text-foreground"
                  >
                    <X className="size-3" />
                  </button>
                </div>
              ))}
            </div>
          ) : null}
        </div>

        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(event) => {
            void handleFiles(event.target.files);
            event.target.value = "";
          }}
        />

        <footer className="flex shrink-0 items-center gap-2 border-t border-border/70 px-3 py-2.5">
          <Button
            variant="ghost"
            size="icon"
            aria-label="Attach files"
            title="Attach files"
            onClick={() => fileInputRef.current?.click()}
          >
            <Paperclip className="size-4" />
          </Button>
          <span className="hidden font-mono text-[11px] text-muted-foreground sm:inline">
            {sendHint} to send
          </span>
          <div className="ml-auto flex items-center gap-2">
            <Button
              variant="ghost"
              size="icon"
              aria-label="Discard"
              title="Discard"
              onClick={() => navigate(-1)}
              className="text-muted-foreground hover:text-destructive"
            >
              <Trash2 className="size-4" />
            </Button>
            <Button
              variant="outline"
              onClick={() => saveDraft.mutate()}
              loading={saveDraft.isPending}
              disabled={!accountId || mailboxesQuery.isPending}
            >
              Save draft
            </Button>
            <Button onClick={onSend} loading={send.isPending} disabled={!canSend}>
              <Send className="size-4" />
              Send
            </Button>
          </div>
        </footer>
      </div>
    </div>
  );
}
