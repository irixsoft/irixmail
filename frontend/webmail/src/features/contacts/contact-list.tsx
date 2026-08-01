import * as React from "react";
import { Button, EmptyState, ErrorState, Input, Skeleton, cn } from "@irixmail/shared";
import { BookUser, Search, UserPlus, X } from "lucide-react";

import { ContactAvatar } from "./contact-avatar";
import { displayName, groupBySection, primaryEmail, sortContacts } from "./contact-display";
import type { ContactCard } from "./types";

export interface ContactListProps {
  title: string;
  contacts: ContactCard[];
  query: string;
  onQuery: (value: string) => void;
  selectedId: string | null;
  loading: boolean;
  error: Error | null;
  onRetry: () => void;
  onOpen: (card: ContactCard) => void;
  onNew: () => void;
  leading?: React.ReactNode;
  trailing?: React.ReactNode;
}

function ContactRow({
  card,
  selected,
  onOpen,
}: {
  card: ContactCard;
  selected: boolean;
  onOpen: () => void;
}) {
  const email = primaryEmail(card);
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors",
        "focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60",
        selected ? "bg-accent text-accent-foreground" : "hover:bg-accent/50",
      )}
    >
      <ContactAvatar card={card} size="sm" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm">{displayName(card)}</span>
        {email ? (
          <span className="block truncate font-mono text-[11px] text-muted-foreground">{email}</span>
        ) : card.kind === "group" ? (
          <span className="block truncate font-mono text-[11px] text-muted-foreground">
            {card.members?.length ?? 0} members
          </span>
        ) : null}
      </span>
    </button>
  );
}

export function ContactList({
  title,
  contacts,
  query,
  onQuery,
  selectedId,
  loading,
  error,
  onRetry,
  onOpen,
  onNew,
  leading,
  trailing,
}: ContactListProps) {
  const sections = React.useMemo(
    () => (query.trim() ? [] : groupBySection(contacts)),
    [contacts, query],
  );
  const flat = React.useMemo(() => (query.trim() ? sortContacts(contacts) : []), [contacts, query]);

  const body = () => {
    if (error) {
      return (
        <div className="p-3">
          <ErrorState title="Could not load contacts" description={error.message} onRetry={onRetry} />
        </div>
      );
    }
    if (loading) {
      return (
        <div className="space-y-1.5 p-2">
          {Array.from({ length: 8 }).map((_, index) => (
            <Skeleton key={index} className="h-11 w-full" />
          ))}
        </div>
      );
    }
    if (contacts.length === 0) {
      return (
        <div className="p-3">
          {query.trim() ? (
            <EmptyState icon={Search} title="No matches" description={`Nothing matches “${query.trim()}”.`} />
          ) : (
            <EmptyState
              icon={BookUser}
              title="No contacts yet"
              description="Add someone, or import a vCard file."
              action={
                <Button size="sm" onClick={onNew}>
                  <UserPlus className="size-3.5" /> New contact
                </Button>
              }
            />
          )}
        </div>
      );
    }
    if (flat.length > 0) {
      return (
        <div className="flex flex-col gap-px p-1.5">
          {flat.map((card) => (
            <ContactRow
              key={card.id}
              card={card}
              selected={card.id === selectedId}
              onOpen={() => onOpen(card)}
            />
          ))}
        </div>
      );
    }
    return (
      <div className="p-1.5">
        {sections.map((section) => (
          <section key={section.letter}>
            <h2 className="sticky top-0 z-10 bg-background/90 px-2 py-1 font-mono text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground backdrop-blur">
              {section.letter}
            </h2>
            <div className="flex flex-col gap-px pb-1">
              {section.contacts.map((card) => (
                <ContactRow
                  key={card.id}
                  card={card}
                  selected={card.id === selectedId}
                  onOpen={() => onOpen(card)}
                />
              ))}
            </div>
          </section>
        ))}
      </div>
    );
  };

  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        {leading}
        <h1 className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</h1>
        <span className="font-mono text-[11px] tabular-nums text-muted-foreground">{contacts.length}</span>
        {trailing}
      </header>
      <div className="shrink-0 border-b px-2 py-2">
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            aria-label="Search contacts"
            placeholder="Search contacts…"
            onChange={(event) => onQuery(event.target.value)}
            className="h-8 pl-8 pr-8"
          />
          {query ? (
            <button
              type="button"
              aria-label="Clear search"
              onClick={() => onQuery("")}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-full p-1 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">{body()}</div>
    </div>
  );
}
