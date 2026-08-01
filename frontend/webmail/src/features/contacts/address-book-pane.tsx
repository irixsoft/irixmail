import * as React from "react";
import { Button, Input, Skeleton, cn } from "@irixmail/shared";
import { BookUser, Check, Download, Plus, Upload, UserPlus, Users, X } from "lucide-react";

import { displayName, sortContacts } from "./contact-display";
import type { AddressBook, ContactCard } from "./types";

interface RowProps {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  count?: number;
  onClick: () => void;
}

function Row({ active, icon, label, count, onClick }: RowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
        "focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60",
        active
          ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground shadow-[inset_2.5px_0_0_0_var(--primary)]"
          : "text-muted-foreground hover:bg-sidebar-accent/50 hover:text-foreground",
      )}
    >
      <span className="shrink-0">{icon}</span>
      <span className="flex-1 truncate">{label}</span>
      {count === undefined ? null : (
        <span className="font-mono text-[11px] tabular-nums text-muted-foreground">{count}</span>
      )}
    </button>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-2 pb-1 pt-4 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
      {children}
    </div>
  );
}

export interface AddressBookPaneProps {
  books: AddressBook[];
  groups: ContactCard[];
  counts: Record<string, number>;
  total: number;
  activeBook: string | null;
  activeGroup: string | null;
  loading: boolean;
  onSelectBook: (id: string | null) => void;
  onSelectGroup: (id: string) => void;
  onCreateBook: (name: string) => void;
  onNewContact: () => void;
  onNewGroup: () => void;
  onImport: () => void;
  onExportAll: () => void;
}

export function AddressBookPane({
  books,
  groups,
  counts,
  total,
  activeBook,
  activeGroup,
  loading,
  onSelectBook,
  onSelectGroup,
  onCreateBook,
  onNewContact,
  onNewGroup,
  onImport,
  onExportAll,
}: AddressBookPaneProps) {
  const [adding, setAdding] = React.useState(false);
  const [draft, setDraft] = React.useState("");

  const submit = () => {
    const name = draft.trim();
    if (!name) return;
    onCreateBook(name);
    setDraft("");
    setAdding(false);
  };

  return (
    <div className="flex h-full flex-col overflow-y-auto bg-sidebar">
      <div className="p-3 pb-1">
        <Button
          onClick={onNewContact}
          className="w-full justify-start gap-2 bg-gradient-to-br from-primary to-primary/80 shadow-sm"
        >
          <UserPlus className="size-4" /> New contact
        </Button>
      </div>

      <div className="min-h-0 flex-1 px-2 pb-2">
        <SectionLabel>Address books</SectionLabel>
        {loading ? (
          <div className="space-y-1 px-1">
            {Array.from({ length: 3 }).map((_, index) => (
              <Skeleton key={index} className="h-7 w-full" />
            ))}
          </div>
        ) : (
          <nav className="flex flex-col gap-px">
            <Row
              active={activeBook === null && activeGroup === null}
              icon={<BookUser className="size-4" />}
              label="All contacts"
              count={total}
              onClick={() => onSelectBook(null)}
            />
            {books.map((book) => (
              <Row
                key={book.id}
                active={activeBook === book.id}
                icon={<BookUser className="size-4" />}
                label={book.name}
                count={counts[book.id] ?? 0}
                onClick={() => onSelectBook(book.id)}
              />
            ))}
          </nav>
        )}

        {adding ? (
          <div className="mt-1 flex items-center gap-1 px-1">
            <Input
              autoFocus
              value={draft}
              aria-label="Address book name"
              placeholder="Address book name"
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  submit();
                }
                if (event.key === "Escape") {
                  event.preventDefault();
                  setAdding(false);
                  setDraft("");
                }
              }}
              className="h-7 text-sm"
            />
            <Button variant="ghost" size="icon" className="size-7" aria-label="Create address book" onClick={submit}>
              <Check className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              aria-label="Cancel"
              onClick={() => {
                setAdding(false);
                setDraft("");
              }}
            >
              <X className="size-3.5" />
            </Button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => setAdding(true)}
            className="mt-1 flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground transition-colors hover:bg-sidebar-accent/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60"
          >
            <Plus className="size-4" /> New address book
          </button>
        )}

        <SectionLabel>Groups</SectionLabel>
        <nav className="flex flex-col gap-px">
          {sortContacts(groups).map((group) => (
            <Row
              key={group.id}
              active={activeGroup === group.id}
              icon={<Users className="size-4" />}
              label={displayName(group)}
              count={group.members?.length ?? 0}
              onClick={() => onSelectGroup(group.id)}
            />
          ))}
          <button
            type="button"
            onClick={onNewGroup}
            className="flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground transition-colors hover:bg-sidebar-accent/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60"
          >
            <Plus className="size-4" /> New group
          </button>
        </nav>
      </div>

      <div className="flex shrink-0 items-center gap-1 border-t border-sidebar-border p-2">
        <Button variant="ghost" size="sm" className="flex-1 justify-start gap-2" onClick={onImport}>
          <Upload className="size-3.5" /> Import
        </Button>
        <Button variant="ghost" size="sm" className="flex-1 justify-start gap-2" onClick={onExportAll}>
          <Download className="size-3.5" /> Export all
        </Button>
      </div>
    </div>
  );
}
