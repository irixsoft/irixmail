import * as React from "react";
import { Button, toast } from "@irixmail/shared";
import { Check, Copy } from "lucide-react";

export function CopyField({ value }: { value: string }) {
  const [copied, setCopied] = React.useState(false);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error("Could not copy to the clipboard");
    }
  };

  return (
    <div className="flex items-center gap-2 rounded-md border bg-muted/40 p-3">
      <code className="flex-1 font-mono text-sm break-all">{value}</code>
      <Button type="button" variant="ghost" size="icon" aria-label="Copy" onClick={onCopy}>
        {copied ? <Check className="size-4 text-success" /> : <Copy className="size-4" />}
      </Button>
    </div>
  );
}
