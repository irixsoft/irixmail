import * as React from "react";
import {
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  cn,
} from "@irixmail/shared";
import { Search } from "lucide-react";

import { ContactAvatar } from "./contact-avatar";
import { displayName, filterContacts, primaryEmail } from "./contact-display";
import { emptyName, type AddressBook, type ContactCard, type ContactPayload } from "./types";

export interface GroupDialogProps {
  open: boolean;
  group: ContactCard | null;
  contacts: ContactCard[];
  books: AddressBook[];
  defaultBookId: string;
  pending: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (payload: ContactPayload, id: string | null) => void;
}

export function GroupDialog({
  open,
  group,
  contacts,
  books,
  defaultBookId,
  pending,
  onOpenChange,
  onSubmit,
}: GroupDialogProps) {
  const [name, setName] = React.useState(group ? displayName(group) : "");
  const [bookId, setBookId] = React.useState(group?.addressBookId || defaultBookId);
  const [members, setMembers] = React.useState<string[]>(group?.members ?? []);
  const [search, setSearch] = React.useState("");
  const [submitted, setSubmitted] = React.useState(false);

  const visible = React.useMemo(() => filterContacts(contacts, search), [contacts, search]);
  const selected = new Set(members);
  const error = !name.trim() ? "Name the group" : !bookId ? "Pick an address book" : null;

  const toggle = (id: string) =>
    setMembers((current) => (current.includes(id) ? current.filter((entry) => entry !== id) : [...current, id]));

  const submit = () => {
    setSubmitted(true);
    if (error) return;
    onSubmit(
      {
        addressBookId: bookId,
        kind: "group",
        name: emptyName,
        fullName: name.trim(),
        nickname: null,
        emails: [],
        phones: [],
        organization: null,
        jobTitle: null,
        addresses: [],
        birthday: null,
        note: null,
        members,
        photo: null,
      },
      group?.id ?? null,
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90dvh] gap-4 overflow-hidden sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{group ? "Edit group" : "New group"}</DialogTitle>
          <DialogDescription className="sr-only">Group name and members</DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="group-name" className="text-xs text-muted-foreground">
                Name
              </Label>
              <Input
                id="group-name"
                autoFocus
                value={name}
                placeholder="Team, family, book club…"
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs text-muted-foreground">Address book</Label>
              <Select value={bookId} onValueChange={setBookId}>
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Pick an address book" />
                </SelectTrigger>
                <SelectContent>
                  {books.map((book) => (
                    <SelectItem key={book.id} value={book.id}>
                      {book.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <Label className="text-xs text-muted-foreground">Members</Label>
              <span className="font-mono text-[11px] tabular-nums text-muted-foreground">
                {members.length} selected
              </span>
            </div>
            <div className="relative">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={search}
                aria-label="Search people"
                placeholder="Search people…"
                onChange={(event) => setSearch(event.target.value)}
                className="h-8 pl-8"
              />
            </div>
            <div className="max-h-64 overflow-y-auto rounded-lg border">
              {visible.length === 0 ? (
                <p className="px-3 py-6 text-center text-sm text-muted-foreground">No people to add.</p>
              ) : (
                visible.map((card) => (
                  <label
                    key={card.id}
                    className={cn(
                      "flex cursor-pointer items-center gap-2.5 px-3 py-1.5 transition-colors",
                      selected.has(card.id) ? "bg-accent/60" : "hover:bg-accent/40",
                    )}
                  >
                    <Checkbox checked={selected.has(card.id)} onCheckedChange={() => toggle(card.id)} />
                    <ContactAvatar card={card} size="sm" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm">{displayName(card)}</span>
                      <span className="block truncate font-mono text-[11px] text-muted-foreground">
                        {primaryEmail(card)}
                      </span>
                    </span>
                  </label>
                ))
              )}
            </div>
          </div>

          {submitted && error ? (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          ) : null}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={pending}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={pending}>
            {pending ? <Spinner className="size-4" /> : null}
            {group ? "Save group" : "Create group"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
