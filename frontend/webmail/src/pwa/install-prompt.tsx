import * as React from "react";
import { Button } from "@irixmail/shared";
import { Download, X } from "lucide-react";

const DISMISS_KEY = "irixmail.webmail.install-dismissed";

interface BeforeInstallPromptEvent extends Event {
  prompt: () => Promise<void>;
}

export function InstallPrompt() {
  const [deferred, setDeferred] = React.useState<BeforeInstallPromptEvent | null>(null);

  React.useEffect(() => {
    if (localStorage.getItem(DISMISS_KEY)) return;
    const onPrompt = (event: Event) => {
      event.preventDefault();
      setDeferred(event as BeforeInstallPromptEvent);
    };
    window.addEventListener("beforeinstallprompt", onPrompt);
    return () => window.removeEventListener("beforeinstallprompt", onPrompt);
  }, []);

  if (!deferred) return null;

  return (
    <div className="fixed bottom-4 right-4 z-40 flex max-w-xs items-start gap-3 rounded-xl border bg-card p-3.5 shadow-lg max-md:bottom-20 max-md:left-4 max-md:right-4 max-md:max-w-none">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-primary to-primary/75 text-primary-foreground">
        <Download className="size-4" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold">Install IRIXMAIL</p>
        <p className="text-[12px] text-muted-foreground">Get the app feel and push notifications.</p>
        <div className="mt-2 flex gap-2">
          <Button
            size="sm"
            onClick={() => {
              void deferred.prompt();
              setDeferred(null);
            }}
          >
            Install
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              localStorage.setItem(DISMISS_KEY, "1");
              setDeferred(null);
            }}
          >
            Don't ask again
          </Button>
        </div>
      </div>
      <button type="button" aria-label="Not now" onClick={() => setDeferred(null)}>
        <X className="size-4 text-muted-foreground" />
      </button>
    </div>
  );
}
