export interface Mailbox {
  id: string;
  name: string;
  role: string | null;
  parentId: string | null;
  sortOrder: number;
  totalEmails: number;
  unreadEmails: number;
}

export interface EmailAddress {
  name?: string | null;
  email: string;
}

export interface Identity {
  id: string;
  name: string;
  email: string;
}

export interface EmailListItem {
  id: string;
  threadId: string;
  mailboxIds: Record<string, boolean>;
  keywords: Record<string, boolean>;
  from?: EmailAddress[] | null;
  to?: EmailAddress[] | null;
  subject?: string | null;
  receivedAt?: string | null;
  preview?: string | null;
  hasAttachment?: boolean;
  size?: number;
}

export interface EmailBodyValue {
  value: string;
  isTruncated?: boolean;
}

export interface EmailBodyPart {
  partId?: string | null;
  blobId?: string | null;
  type?: string | null;
  name?: string | null;
  size?: number;
  disposition?: string | null;
}

export interface EmailFull extends EmailListItem {
  cc?: EmailAddress[] | null;
  bcc?: EmailAddress[] | null;
  replyTo?: EmailAddress[] | null;
  sentAt?: string | null;
  bodyValues?: Record<string, EmailBodyValue>;
  textBody?: EmailBodyPart[];
  htmlBody?: EmailBodyPart[];
  attachments?: EmailBodyPart[];
}

export const ROLE_LABELS: Record<string, string> = {
  inbox: "Inbox",
  drafts: "Drafts",
  sent: "Sent",
  junk: "Spam",
  trash: "Trash",
  archive: "Archive",
};

export function mailboxLabel(mailbox: Mailbox): string {
  return (mailbox.role && ROLE_LABELS[mailbox.role]) || mailbox.name;
}
