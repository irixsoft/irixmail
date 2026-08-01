import * as React from "react";
import { Spinner, toast, useAuth } from "@irixmail/shared";
import { Paperclip } from "lucide-react";

import { useJmap } from "@/lib/jmap";
import { formatBytes } from "@/lib/format";
import type { EmailBodyPart } from "@/lib/mail-types";

export function AttachmentList({
  accountId,
  attachments,
}: {
  accountId: string;
  attachments: EmailBodyPart[];
}) {
  const jmap = useJmap();
  const { token } = useAuth();
  const [downloading, setDownloading] = React.useState<string | null>(null);

  const items = attachments.filter((part) => part.disposition !== "inline" && part.blobId);
  if (items.length === 0) return null;

  const download = async (part: EmailBodyPart) => {
    if (!part.blobId) return;
    const name = part.name ?? "attachment";
    setDownloading(part.blobId);
    try {
      const url = jmap.downloadUrl(accountId, part.blobId, name);
      const response = await fetch(url, {
        headers: token ? { Authorization: `Bearer ${token}` } : {},
      });
      if (!response.ok) throw new Error("download failed");
      const blob = await response.blob();
      const objectUrl = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = objectUrl;
      link.download = name;
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(objectUrl);
    } catch {
      toast.error("Could not download the attachment");
    } finally {
      setDownloading(null);
    }
  };

  return (
    <div className="space-y-2 border-t pt-4">
      <p className="font-mono text-[10px] tracking-wider text-muted-foreground uppercase">
        Attachments
      </p>
      <div className="flex flex-wrap gap-2">
        {items.map((part) => (
          <button
            key={part.blobId}
            type="button"
            onClick={() => download(part)}
            disabled={downloading === part.blobId}
            className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm transition-colors hover:bg-muted/40 disabled:opacity-60"
          >
            {downloading === part.blobId ? (
              <Spinner className="size-4" />
            ) : (
              <Paperclip className="size-4 text-muted-foreground" />
            )}
            <span className="max-w-[12rem] truncate">{part.name ?? "attachment"}</span>
            {part.size ? (
              <span className="text-xs text-muted-foreground">{formatBytes(part.size)}</span>
            ) : null}
          </button>
        ))}
      </div>
    </div>
  );
}
