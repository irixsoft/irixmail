import * as React from "react";
import { motion } from "motion/react";
import { Avatar, AvatarFallback, cn } from "@irixmail/shared";
import { Archive, Check, Mail, MailOpen, MoreHorizontal, Paperclip, Star, Trash2 } from "lucide-react";

import { resolveSwipe } from "./swipe";

import { TAG_PALETTE, messageTagIds, type TagDefinition } from "@/jmap/tags";
import { formatListDate, senderName } from "@/lib/format";
import type { Density } from "./density";
import type { ThreadGroup } from "./thread-groups";

const AVATAR_HUES = [
  "bg-amber-600",
  "bg-orange-700",
  "bg-rose-700",
  "bg-emerald-700",
  "bg-teal-700",
  "bg-sky-700",
  "bg-indigo-600",
  "bg-purple-700",
];

function avatarHue(key: string): string {
  let hash = 0;
  for (let index = 0; index < key.length; index += 1) {
    hash = (hash * 31 + key.charCodeAt(index)) | 0;
  }
  return AVATAR_HUES[Math.abs(hash) % AVATAR_HUES.length]!;
}

function initialsOf(name: string): string {
  const parts = name.trim().split(/\s+/);
  const first = parts[0]?.[0] ?? "?";
  const second = parts.length > 1 ? (parts[parts.length - 1]?.[0] ?? "") : (parts[0]?.[1] ?? "");
  return (first + second).toUpperCase();
}

export interface MessageRowProps {
  group: ThreadGroup;
  density: Density;
  selected: boolean;
  focused?: boolean;
  selectionActive: boolean;
  tagDefinitions: TagDefinition[];
  onOpen: () => void;
  onSelectToggle: (range: boolean) => void;
  onArchive: () => void;
  onDelete: () => void;
  onToggleRead: () => void;
  onToggleFlag: () => void;
  swipeEnabled?: boolean;
  onSwipeMenu?: () => void;
}

export function MessageRow({
  group,
  density,
  selected,
  focused,
  selectionActive,
  tagDefinitions,
  onOpen,
  onSelectToggle,
  onArchive,
  onDelete,
  onToggleRead,
  onToggleFlag,
  swipeEnabled,
  onSwipeMenu,
}: MessageRowProps) {
  const email = group.newest;
  const sender = senderName(email.from) || "(unknown)";
  const unread = group.hasUnread;
  const tagIds = messageTagIds(email.keywords);
  const tags = tagDefinitions.filter((tag) => tagIds.includes(tag.id)).slice(0, 3);

  const onClick = (event: React.MouseEvent) => {
    if (event.metaKey || event.ctrlKey) {
      onSelectToggle(false);
      return;
    }
    if (event.shiftKey) {
      onSelectToggle(true);
      return;
    }
    if (selectionActive) {
      onSelectToggle(false);
      return;
    }
    onOpen();
  };

  const action = (handler: () => void) => (event: React.MouseEvent) => {
    event.stopPropagation();
    handler();
  };

  const row = (
    <div
      role="row"
      aria-selected={selected}
      onClick={onClick}
      style={{ paddingBlock: "var(--list-row-py)" }}
      className={cn(
        "group relative flex cursor-pointer items-start gap-2.5 rounded-lg px-2.5 transition-colors",
        selected
          ? "bg-accent shadow-[inset_3px_0_0_0_var(--primary)]"
          : unread
            ? "bg-card hover:bg-accent/40"
            : "hover:bg-accent/40",
        focused && "ring-2 ring-primary/40",
      )}
    >
      {unread && !selected ? (
        <span className="beacon-dot absolute left-0.5 top-1/2 size-1.5 -translate-y-1/2 rounded-full bg-primary" />
      ) : null}

      {density === "cozy" ? (
        <div className="relative size-8 shrink-0 self-center">
          <Avatar
            className={cn(
              "size-8 transition-opacity",
              (selectionActive || selected) && "opacity-0",
              "group-hover:opacity-0",
            )}
          >
            <AvatarFallback className={cn("text-[11px] font-medium text-white", avatarHue(email.from?.[0]?.email ?? sender))}>
              {initialsOf(sender)}
            </AvatarFallback>
          </Avatar>
          <button
            type="button"
            aria-label={selected ? "Deselect" : "Select"}
            onClick={action(() => onSelectToggle(false))}
            className={cn(
              "absolute inset-0 flex items-center justify-center rounded-full border shadow-xs outline-none transition-opacity focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring",
              selected
                ? "border-primary bg-primary text-primary-foreground opacity-100"
                : "border-muted-foreground/60 bg-card opacity-0 group-hover:opacity-100",
              selectionActive && !selected && "opacity-100",
            )}
          >
            <Check className={cn("size-4", !selected && "opacity-60")} />
          </button>
        </div>
      ) : (
        <button
          type="button"
          aria-label={selected ? "Deselect" : "Select"}
          onClick={action(() => onSelectToggle(false))}
          className={cn(
            "flex size-4 shrink-0 self-center items-center justify-center rounded-[4px] border shadow-xs outline-none transition-opacity focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring",
            selected
              ? "border-primary bg-primary text-primary-foreground opacity-100"
              : "border-muted-foreground/60 bg-card opacity-0 group-hover:opacity-100",
            selectionActive && "opacity-100",
          )}
        >
          {selected ? <Check className="size-3" /> : null}
        </button>
      )}

      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className={cn("min-w-0 flex-1 truncate text-sm", unread ? "font-semibold" : "font-medium text-foreground/90")}>
            {sender}
          </span>
          {group.hasAttachment ? <Paperclip className="size-3 shrink-0 text-muted-foreground" /> : null}
          <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground group-hover:opacity-0">
            {formatListDate(email.receivedAt)}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <span className={cn("min-w-0 flex-1 truncate text-[13px]", unread ? "font-medium" : "text-foreground/80")}>
            {email.subject || "(no subject)"}
          </span>
          {group.count > 1 ? (
            <span className="shrink-0 rounded-full bg-secondary px-1.5 font-mono text-[10px] tabular-nums text-secondary-foreground">
              {group.count}
            </span>
          ) : null}
          {group.hasFlagged ? <Star className="size-3 shrink-0 fill-warning text-warning" /> : null}
          {tags.map((tag) => (
            <span key={tag.id} className={cn("size-2 shrink-0 rounded-full", TAG_PALETTE[tag.color]?.dot)} />
          ))}
        </div>
        {density === "cozy" ? (
          <div className="truncate text-[13px] text-muted-foreground">{email.preview ?? ""}</div>
        ) : null}
      </div>

      <div className="absolute right-1.5 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded-md border bg-card p-0.5 shadow-sm group-hover:flex">
        <button type="button" aria-label="Archive" onClick={action(onArchive)} className="rounded p-1.5 hover:bg-accent">
          <Archive className="size-3.5" />
        </button>
        <button type="button" aria-label="Delete" onClick={action(onDelete)} className="rounded p-1.5 hover:bg-accent">
          <Trash2 className="size-3.5" />
        </button>
        <button
          type="button"
          aria-label={unread ? "Mark read" : "Mark unread"}
          onClick={action(onToggleRead)}
          className="rounded p-1.5 hover:bg-accent"
        >
          {unread ? <MailOpen className="size-3.5" /> : <Mail className="size-3.5" />}
        </button>
        <button type="button" aria-label="Toggle star" onClick={action(onToggleFlag)} className="rounded p-1.5 hover:bg-accent">
          <Star className={cn("size-3.5", group.hasFlagged && "fill-warning text-warning")} />
        </button>
      </div>
    </div>
  );

  if (!swipeEnabled) return row;

  return (
    <div className="relative overflow-hidden rounded-lg">
      <div aria-hidden className="absolute inset-0 flex items-center justify-between px-5">
        <Archive className="size-5 text-success" />
        <MoreHorizontal className="size-5 text-muted-foreground" />
      </div>
      <motion.div
        drag="x"
        dragConstraints={{ left: -140, right: 140 }}
        dragElastic={0.12}
        dragSnapToOrigin
        onDragEnd={(_event, info) => {
          const swipe = resolveSwipe(info.offset.x);
          if (swipe === "archive") onArchive();
          if (swipe === "menu") onSwipeMenu?.();
        }}
        className="bg-background"
      >
        {row}
      </motion.div>
    </div>
  );
}
