import * as React from "react";
import { useSearchParams } from "react-router-dom";
import { Button, Input, cn } from "@irixmail/shared";
import { Check, Minus, Search, SlidersHorizontal, X } from "lucide-react";

import { buildEmailFilter, countActiveFilters, emptyFilters, type SearchFilters } from "@/jmap/search-filter";
import { loadTagDefinitions, tagKeyword } from "@/jmap/tags";
import { initDensity } from "@/features/mail/density";
import { MessageList } from "@/features/mail/message-list";
import { useMailboxes } from "@/features/mail/use-mailboxes";
import { mailboxLabel } from "@/lib/mail-types";

function TriChip({
  label,
  value,
  onChange,
}: {
  label: string;
  value: boolean | null;
  onChange: (value: boolean | null) => void;
}) {
  const next = value === null ? true : value === true ? false : null;
  return (
    <button
      type="button"
      onClick={() => onChange(next)}
      className={cn(
        "flex items-center gap-1 rounded-full border px-2.5 py-1 text-[12px] transition-colors",
        value === true && "border-primary bg-primary/15 text-primary",
        value === false && "border-destructive/50 bg-destructive/10 text-destructive",
        value === null && "text-muted-foreground hover:bg-accent",
      )}
    >
      {value === true ? <Check className="size-3" /> : value === false ? <Minus className="size-3" /> : null}
      {value === false ? `Not ${label.toLowerCase()}` : label}
    </button>
  );
}

export function SearchPage() {
  const [params, setParams] = useSearchParams();
  const [filters, setFilters] = React.useState<SearchFilters>(() => ({
    ...emptyFilters,
    text: params.get("q") ?? "",
    tag: params.get("tag"),
  }));
  const [showPanel, setShowPanel] = React.useState(false);
  const [draft, setDraft] = React.useState(filters.text);
  const { list: mailboxes } = useMailboxes();
  const tags = React.useMemo(loadTagDefinitions, []);
  const [density] = React.useState(initDensity);

  React.useEffect(() => {
    const tag = params.get("tag");
    const q = params.get("q") ?? "";
    setFilters((current) => ({ ...current, tag, text: q }));
    setDraft(q);
  }, [params]);

  const commitText = (text: string) => {
    setFilters((current) => ({ ...current, text }));
    setParams(
      (current) => {
        if (text) current.set("q", text);
        else current.delete("q");
        return current;
      },
      { replace: true },
    );
  };

  const set = <K extends keyof SearchFilters>(key: K, value: SearchFilters[K]) =>
    setFilters((current) => ({ ...current, [key]: value }));

  const active = countActiveFilters(filters);
  const filter = buildEmailFilter(filters);
  const hasQuery = filters.text.length > 0 || active > 0;

  const appliedChips: { label: string; clear: () => void }[] = [];
  if (filters.from) appliedChips.push({ label: `from: ${filters.from}`, clear: () => set("from", "") });
  if (filters.to) appliedChips.push({ label: `to: ${filters.to}`, clear: () => set("to", "") });
  if (filters.subject) appliedChips.push({ label: `subject: ${filters.subject}`, clear: () => set("subject", "") });
  if (filters.mailboxId) {
    const box = mailboxes.find((mailbox) => mailbox.id === filters.mailboxId);
    appliedChips.push({ label: `in: ${box ? mailboxLabel(box) : "folder"}`, clear: () => set("mailboxId", null) });
  }
  if (filters.tag) {
    const tag = tags.find((entry) => tagKeyword(entry.id) === filters.tag);
    appliedChips.push({ label: `tag: ${tag?.label ?? filters.tag}`, clear: () => set("tag", null) });
  }
  if (filters.after) appliedChips.push({ label: `after: ${filters.after}`, clear: () => set("after", null) });
  if (filters.before) appliedChips.push({ label: `before: ${filters.before}`, clear: () => set("before", null) });

  return (
    <div className="flex h-full min-w-0 flex-col">
      <div className="shrink-0 border-b px-3 py-2">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            commitText(draft);
          }}
          className="flex items-center gap-2"
        >
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              autoFocus
              data-search-input
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="Search mail…"
              className="pl-8"
            />
          </div>
          <Button type="submit" variant="secondary">
            Search
          </Button>
          <Button
            type="button"
            variant={active > 0 ? "default" : "ghost"}
            size="icon"
            aria-label="Search filters"
            onClick={() => setShowPanel((current) => !current)}
          >
            <SlidersHorizontal className="size-4" />
          </Button>
        </form>

        {showPanel ? (
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <TriChip label="Attachment" value={filters.hasAttachment} onChange={(value) => set("hasAttachment", value)} />
            <TriChip label="Unread" value={filters.unread} onChange={(value) => set("unread", value)} />
            <TriChip label="Starred" value={filters.starred} onChange={(value) => set("starred", value)} />
            <Input
              value={filters.from}
              onChange={(event) => set("from", event.target.value)}
              placeholder="From"
              className="h-7 w-32 text-[12px]"
            />
            <Input
              value={filters.to}
              onChange={(event) => set("to", event.target.value)}
              placeholder="To"
              className="h-7 w-32 text-[12px]"
            />
            <Input
              value={filters.subject}
              onChange={(event) => set("subject", event.target.value)}
              placeholder="Subject"
              className="h-7 w-36 text-[12px]"
            />
            <select
              value={filters.mailboxId ?? ""}
              onChange={(event) => set("mailboxId", event.target.value || null)}
              className="h-7 rounded-md border bg-background px-2 text-[12px]"
            >
              <option value="">Any folder</option>
              {mailboxes.map((mailbox) => (
                <option key={mailbox.id} value={mailbox.id}>
                  {mailboxLabel(mailbox)}
                </option>
              ))}
            </select>
            <input
              type="date"
              value={filters.after ?? ""}
              onChange={(event) => set("after", event.target.value || null)}
              aria-label="After date"
              className="h-7 rounded-md border bg-background px-2 font-mono text-[12px]"
            />
            <input
              type="date"
              value={filters.before ?? ""}
              onChange={(event) => set("before", event.target.value || null)}
              aria-label="Before date"
              className="h-7 rounded-md border bg-background px-2 font-mono text-[12px]"
            />
            {active > 0 ? (
              <Button variant="ghost" size="sm" onClick={() => setFilters((current) => ({ ...emptyFilters, text: current.text }))}>
                Clear
              </Button>
            ) : null}
          </div>
        ) : null}

        {appliedChips.length > 0 ? (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {appliedChips.map((chip) => (
              <span
                key={chip.label}
                className="flex items-center gap-1 rounded-full bg-accent px-2 py-0.5 font-mono text-[11px]"
              >
                {chip.label}
                <button type="button" aria-label={`Remove ${chip.label}`} onClick={chip.clear}>
                  <X className="size-3" />
                </button>
              </span>
            ))}
          </div>
        ) : null}
      </div>

      <div className="min-h-0 flex-1">
        {hasQuery ? (
          <MessageList
            filter={filter}
            filterKey={JSON.stringify(filter)}
            title="Search results"
            density={density}
            openPath={(group) => {
              const mailboxId = Object.keys(group.newest.mailboxIds)[0];
              return `/${mailboxId}/${group.newest.id}`;
            }}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            Search your mail, or add filters.
          </div>
        )}
      </div>
    </div>
  );
}
