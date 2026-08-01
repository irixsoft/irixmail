import { useNavigate } from "react-router-dom";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@irixmail/shared";
import { Folder, Moon, Search, Settings, SquarePen, Sun } from "lucide-react";

import { loadThemeMode, setThemeMode } from "@/lib/theme";
import { useMailboxes } from "@/features/mail/use-mailboxes";
import { mailboxLabel } from "@/lib/mail-types";

export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const navigate = useNavigate();
  const { list: mailboxes } = useMailboxes();

  const run = (action: () => void) => {
    onOpenChange(false);
    action();
  };

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} title="Command palette" description="Jump anywhere">
      <CommandInput placeholder="Type a folder or command…" />
      <CommandList>
        <CommandEmpty>Nothing found.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem onSelect={() => run(() => navigate("/compose"))}>
            <SquarePen className="size-4" /> Compose
          </CommandItem>
          <CommandItem onSelect={() => run(() => navigate("/search"))}>
            <Search className="size-4" /> Search mail
          </CommandItem>
          <CommandItem onSelect={() => run(() => navigate("/settings"))}>
            <Settings className="size-4" /> Settings
          </CommandItem>
          <CommandItem
            onSelect={() =>
              run(() => setThemeMode(loadThemeMode() === "dark" ? "light" : "dark"))
            }
          >
            {loadThemeMode() === "dark" ? <Sun className="size-4" /> : <Moon className="size-4" />}
            Toggle theme
          </CommandItem>
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Folders">
          {mailboxes.map((mailbox) => (
            <CommandItem key={mailbox.id} onSelect={() => run(() => navigate(`/${mailbox.id}`))}>
              <Folder className="size-4" /> {mailboxLabel(mailbox)}
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
