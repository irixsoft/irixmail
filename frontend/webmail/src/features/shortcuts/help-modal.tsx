import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@irixmail/shared";

import { SHORTCUT_HELP } from "./shortcut-list";

export function ShortcutHelpModal({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
        </DialogHeader>
        <div className="grid gap-4 sm:grid-cols-2">
          {SHORTCUT_HELP.map((section) => (
            <div key={section.group}>
              <h3 className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                {section.group}
              </h3>
              <dl className="space-y-1">
                {section.items.map((item) => (
                  <div key={item.keys} className="flex items-baseline justify-between gap-3 text-sm">
                    <dt className="rounded bg-secondary px-1.5 py-0.5 font-mono text-[11px]">{item.keys}</dt>
                    <dd className="flex-1 text-right text-[13px] text-muted-foreground">{item.description}</dd>
                  </div>
                ))}
              </dl>
            </div>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
