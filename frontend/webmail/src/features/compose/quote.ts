import { plainTextToHtml, sanitizeEmailHtml } from "@/features/mail/sanitize";
import { formatDateTime } from "@/lib/format";
import type { EmailAddress } from "@/lib/mail-types";

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function senderLabel(from?: EmailAddress[] | null): string {
  const first = from?.[0];
  if (!first) return "the sender";
  const name = first.name?.trim();
  return name ? `${name} <${first.email}>` : first.email;
}

export function attributionLine(
  from?: EmailAddress[] | null,
  date?: string | number | null,
): string {
  return `On ${formatDateTime(date ?? null)}, ${senderLabel(from)} wrote:`;
}

export function quotedHtml(line: string, html: string): string {
  return `<p></p><p>${escapeHtml(line)}</p><blockquote>${sanitizeEmailHtml(html)}</blockquote>`;
}

export function quotedHtmlFromText(line: string, text: string): string {
  return `<p></p><p>${escapeHtml(line)}</p><blockquote>${plainTextToHtml(text)}</blockquote>`;
}

export interface ThreadingSource {
  messageId?: string[];
  references?: string[];
}

export interface ThreadingHeaders {
  inReplyTo?: string[];
  references?: string[];
}

export function threadingHeaders(
  mode: "reply" | "replyAll" | "forward" | undefined,
  source: ThreadingSource | null | undefined,
): ThreadingHeaders {
  if (!mode) return {};
  const messageId = source?.messageId ?? [];
  if (messageId.length === 0) return {};
  const references = [...(source?.references ?? []), ...messageId];
  if (mode === "forward") return { references };
  return { inReplyTo: messageId, references };
}

export function quotedText(line: string, text: string): string {
  const body = text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((entry) => (entry ? `> ${entry}` : ">"))
    .join("\n");
  return `\n\n${line}\n${body}\n`;
}
