import * as React from "react";
import { Button, ConfirmDialog, EmptyState, Input, Label, cn, toast } from "@irixmail/shared";
import { Check, Pencil, Plus, Tag, Trash2, X } from "lucide-react";

import {
  TAG_PALETTE,
  loadTagDefinitions,
  saveTagDefinitions,
  type TagDefinition,
} from "@/jmap/tags";
import { SettingsCard } from "./section-card";

const PALETTE_KEYS = Object.keys(TAG_PALETTE);
const DEFAULT_COLOR = PALETTE_KEYS[0] ?? "amber";

function slugify(label: string): string {
  return label
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function dotClass(color: string): string {
  return TAG_PALETTE[color]?.dot ?? "bg-muted-foreground";
}

function ColorPicker({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (color: string) => void;
  label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap items-center gap-1.5">
      {PALETTE_KEYS.map((key) => (
        <button
          key={key}
          type="button"
          role="radio"
          aria-checked={value === key}
          aria-label={key}
          onClick={() => onChange(key)}
          className={cn(
            "flex size-6 items-center justify-center rounded-full border transition-colors",
            value === key ? "border-foreground/50" : "border-transparent hover:border-border",
          )}
        >
          <span className={cn("size-3.5 rounded-full", dotClass(key))} />
        </button>
      ))}
    </div>
  );
}

export function TagsSection() {
  const [tags, setTags] = React.useState<TagDefinition[]>(() => loadTagDefinitions());
  const [newLabel, setNewLabel] = React.useState("");
  const [newColor, setNewColor] = React.useState(DEFAULT_COLOR);
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [draftLabel, setDraftLabel] = React.useState("");
  const [draftColor, setDraftColor] = React.useState(DEFAULT_COLOR);
  const [pendingDelete, setPendingDelete] = React.useState<TagDefinition | null>(null);

  const commit = (next: TagDefinition[]) => {
    setTags(next);
    saveTagDefinitions(next);
  };

  const add = () => {
    const label = newLabel.trim();
    const id = slugify(label);
    if (!id) {
      toast.error("Give the tag a name");
      return;
    }
    if (tags.some((tag) => tag.id === id)) {
      toast.error("A tag with that name already exists");
      return;
    }
    commit([...tags, { id, label, color: newColor }]);
    setNewLabel("");
    setNewColor(DEFAULT_COLOR);
    toast.success("Tag added");
  };

  const startEdit = (tag: TagDefinition) => {
    setEditingId(tag.id);
    setDraftLabel(tag.label);
    setDraftColor(tag.color);
  };

  // renaming keeps the id so keywords already on messages stay attached
  const saveEdit = (tag: TagDefinition) => {
    const label = draftLabel.trim();
    if (!label) {
      toast.error("Give the tag a name");
      return;
    }
    commit(tags.map((entry) => (entry.id === tag.id ? { ...entry, label, color: draftColor } : entry)));
    setEditingId(null);
    toast.success("Tag updated");
  };

  const remove = (tag: TagDefinition) => {
    commit(tags.filter((entry) => entry.id !== tag.id));
    setPendingDelete(null);
    toast.success("Tag deleted");
  };

  return (
    <div className="space-y-4">
      <SettingsCard
        title="Your tags"
        description={`${tags.length} tag${tags.length === 1 ? "" : "s"} available when labelling mail.`}
        bodyClassName={tags.length === 0 ? "p-4" : "p-0"}
      >
        {tags.length === 0 ? (
          <EmptyState
            icon={Tag}
            title="No tags yet"
            description="Tags let you colour-code conversations across folders."
            className="border-0 bg-transparent py-8"
          />
        ) : (
          <ul className="divide-y">
            {tags.map((tag) =>
              editingId === tag.id ? (
                <li key={tag.id} className="space-y-3 px-4 py-3">
                  <Input
                    value={draftLabel}
                    onChange={(event) => setDraftLabel(event.target.value)}
                    aria-label="Tag name"
                  />
                  <ColorPicker value={draftColor} onChange={setDraftColor} label="Tag colour" />
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-mono text-xs text-muted-foreground">{tag.id}</span>
                    <div className="flex gap-2">
                      <Button variant="ghost" size="sm" onClick={() => setEditingId(null)}>
                        <X className="size-4" />
                        Cancel
                      </Button>
                      <Button size="sm" onClick={() => saveEdit(tag)}>
                        <Check className="size-4" />
                        Save
                      </Button>
                    </div>
                  </div>
                </li>
              ) : (
                <li key={tag.id} className="flex items-center justify-between gap-3 px-4 py-3">
                  <div className="flex min-w-0 items-center gap-2.5">
                    <span className={cn("size-2.5 shrink-0 rounded-full", dotClass(tag.color))} />
                    <span className="truncate text-[13px] font-medium">{tag.label}</span>
                    <span className="truncate font-mono text-xs text-muted-foreground">{tag.id}</span>
                  </div>
                  <div className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Rename ${tag.label}`}
                      onClick={() => startEdit(tag)}
                    >
                      <Pencil className="size-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Delete ${tag.label}`}
                      onClick={() => setPendingDelete(tag)}
                    >
                      <Trash2 className="size-4 text-destructive" />
                    </Button>
                  </div>
                </li>
              ),
            )}
          </ul>
        )}
      </SettingsCard>

      <SettingsCard title="Add a tag" bodyClassName="grid gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="tag-label" className="text-[13px]">
            Name
          </Label>
          <Input
            id="tag-label"
            value={newLabel}
            placeholder="e.g. Invoices"
            onChange={(event) => setNewLabel(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                add();
              }
            }}
          />
        </div>
        <div className="grid gap-1.5">
          <span className="text-[13px] font-medium">Colour</span>
          <ColorPicker value={newColor} onChange={setNewColor} label="New tag colour" />
        </div>
        <div className="flex items-center justify-between gap-3">
          <span className="font-mono text-xs text-muted-foreground">{slugify(newLabel) || "—"}</span>
          <Button size="sm" onClick={add} disabled={!newLabel.trim()}>
            <Plus className="size-4" />
            Add tag
          </Button>
        </div>
      </SettingsCard>

      <ConfirmDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title="Delete tag"
        description={
          pendingDelete
            ? `"${pendingDelete.label}" disappears from the sidebar. Messages keep the keyword until you remove it from them.`
            : undefined
        }
        confirmLabel="Delete"
        destructive
        onConfirm={() => {
          if (pendingDelete) remove(pendingDelete);
        }}
      />
    </div>
  );
}
