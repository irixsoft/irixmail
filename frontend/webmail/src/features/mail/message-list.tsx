import * as React from "react";
import { useNavigate } from "react-router-dom";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Badge, Button, EmptyState, Sheet, SheetContent, SheetTitle, Skeleton, cn } from "@irixmail/shared";
import { Archive, Inbox, Mail, MailOpen, Star, Trash2, X } from "lucide-react";

import { flaggedPatch, movePatch, seenPatch, updateMap } from "@/jmap/mutations";
import { loadTagDefinitions } from "@/jmap/tags";
import { useOnline } from "@/lib/use-online";
import { useShortcuts } from "@/features/shortcuts/use-shortcuts";
import { MessageRow } from "./message-row";
import { useMailboxes } from "./use-mailboxes";
import { applySelectionClick, emptySelection, type Selection } from "./selection";
import { groupByThread, type ThreadGroup } from "./thread-groups";
import { useEmailList, useEmailMutation } from "./use-email-list";
import type { Density } from "./density";

export interface MessageListProps {
  filter: Record<string, unknown>;
  filterKey: string;
  title: string;
  density: Density;
  openPath: (group: ThreadGroup) => string;
  swipe?: boolean;
  leading?: React.ReactNode;
  trailing?: React.ReactNode;
}

export function MessageList({
  filter,
  filterKey,
  title,
  density,
  openPath,
  swipe,
  leading,
  trailing,
}: MessageListProps) {
  const navigate = useNavigate();
  const online = useOnline();
  const { query, emails, total } = useEmailList(filter, filterKey);
  const mutation = useEmailMutation();
  const { byRole } = useMailboxes();
  const [selection, setSelection] = React.useState<Selection>(emptySelection);
  const [focusedIndex, setFocusedIndex] = React.useState(-1);
  const [menuGroup, setMenuGroup] = React.useState<ThreadGroup | null>(null);
  const tagDefinitions = React.useMemo(loadTagDefinitions, []);
  const parentRef = React.useRef<HTMLDivElement>(null);

  const groups = React.useMemo(() => groupByThread(emails), [emails]);
  const orderedIds = React.useMemo(() => groups.map((group) => group.newest.id), [groups]);

  React.useEffect(() => {
    setSelection(emptySelection);
  }, [filterKey]);

  const refetch = query.refetch;
  React.useEffect(() => {
    if (!swipe) return;
    const element = parentRef.current;
    if (!element) return;
    let startY = 0;
    let pulling = false;
    const onTouchStart = (event: TouchEvent) => {
      pulling = element.scrollTop <= 0;
      startY = event.touches[0]?.clientY ?? 0;
    };
    const onTouchEnd = (event: TouchEvent) => {
      if (!pulling) return;
      const endY = event.changedTouches[0]?.clientY ?? 0;
      if (endY - startY > 80 && element.scrollTop <= 0) void refetch();
    };
    element.addEventListener("touchstart", onTouchStart, { passive: true });
    element.addEventListener("touchend", onTouchEnd, { passive: true });
    return () => {
      element.removeEventListener("touchstart", onTouchStart);
      element.removeEventListener("touchend", onTouchEnd);
    };
  }, [swipe, refetch]);

  const virtualizer = useVirtualizer({
    count: groups.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => (density === "cozy" ? 84 : 52),
    overscan: 5,
    getItemKey: (index) => groups[index]!.threadId,
  });

  const items = virtualizer.getVirtualItems();
  React.useEffect(() => {
    const last = items[items.length - 1];
    if (!last) return;
    if (last.index >= groups.length - 5 && query.hasNextPage && !query.isFetchingNextPage) {
      void query.fetchNextPage();
    }
  }, [items, groups.length, query]);

  const selectedGroups = groups.filter((group) => selection.selected.has(group.newest.id));
  const selectedEmailIds = selectedGroups.flatMap((group) => group.emailIds);
  const anyUnread = selectedGroups.some((group) => group.hasUnread);

  const act = (ids: string[], patch: Record<string, unknown>) => {
    mutation.mutate({ update: updateMap(ids, patch) });
    setSelection(emptySelection);
  };
  const moveTo = (ids: string[], role: string) => {
    const target = byRole[role];
    if (target) act(ids, movePatch(target.id));
  };

  const focused = focusedIndex >= 0 ? groups[focusedIndex] : undefined;
  const actionTargets = () =>
    selection.selected.size > 0 ? selectedGroups.flatMap((group) => group.emailIds) : (focused?.emailIds ?? []);
  const moveFocus = (delta: number) => {
    setFocusedIndex((current) => {
      const next = Math.min(groups.length - 1, Math.max(0, current + delta));
      virtualizer.scrollToIndex(next);
      return next;
    });
  };

  useShortcuts({
    j: () => moveFocus(1),
    ArrowDown: () => moveFocus(1),
    k: () => moveFocus(-1),
    ArrowUp: () => moveFocus(-1),
    o: () => focused && navigate(openPath(focused)),
    Enter: () => focused && navigate(openPath(focused)),
    x: () =>
      focused &&
      setSelection((current) =>
        applySelectionClick(current, orderedIds, focused.newest.id, { toggle: true, range: false }),
      ),
    "mod+a": () => setSelection({ selected: new Set(orderedIds), anchor: orderedIds[0] ?? null }),
    e: () => {
      const ids = actionTargets();
      if (ids.length) moveTo(ids, "archive");
    },
    "#": () => {
      const ids = actionTargets();
      if (ids.length) moveTo(ids, "trash");
    },
    u: () => {
      const ids = actionTargets();
      const unread = selection.selected.size > 0 ? anyUnread : (focused?.hasUnread ?? false);
      if (ids.length) act(ids, seenPatch(unread));
    },
    s: () => focused && act([focused.newest.id], flaggedPatch(!focused.hasFlagged)),
    Escape: () => setSelection(emptySelection),
  });

  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        {leading}
        <h1 className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</h1>
        {online ? null : (
          <Badge variant="muted" role="status" className="px-1.5 py-0 text-[10px] font-normal">
            Offline
          </Badge>
        )}
        <span className="font-mono text-[11px] tabular-nums text-muted-foreground">{total}</span>
        {trailing}
      </header>

      {selection.selected.size > 0 ? (
        <div className="flex shrink-0 items-center gap-1 border-b bg-accent/60 px-2 py-1.5">
          <span className="px-1 font-mono text-[11px] tabular-nums">{selection.selected.size} selected</span>
          <div className="flex-1" />
          <Button variant="ghost" size="sm" onClick={() => moveTo(selectedEmailIds, "archive")}>
            <Archive className="size-3.5" /> Archive
          </Button>
          <Button variant="ghost" size="sm" onClick={() => moveTo(selectedEmailIds, "trash")}>
            <Trash2 className="size-3.5" /> Delete
          </Button>
          <Button variant="ghost" size="sm" onClick={() => act(selectedEmailIds, seenPatch(anyUnread))}>
            {anyUnread ? <MailOpen className="size-3.5" /> : <Mail className="size-3.5" />}
            {anyUnread ? "Read" : "Unread"}
          </Button>
          <Button variant="ghost" size="icon" aria-label="Clear selection" onClick={() => setSelection(emptySelection)}>
            <X className="size-3.5" />
          </Button>
        </div>
      ) : null}

      <div ref={parentRef} className="min-h-0 flex-1 overflow-y-auto px-1.5 py-1">
        {query.isLoading ? (
          <div className="space-y-1.5 p-1.5">
            {Array.from({ length: 8 }).map((_, index) => (
              <Skeleton key={index} className={cn(density === "cozy" ? "h-20" : "h-12", "w-full")} />
            ))}
          </div>
        ) : groups.length === 0 ? (
          <EmptyState icon={Inbox} title="Nothing here" description="This folder has no mail." />
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {items.map((item) => {
              const group = groups[item.index]!;
              const id = group.newest.id;
              return (
                <div
                  key={item.key}
                  data-index={item.index}
                  ref={virtualizer.measureElement}
                  style={{ position: "absolute", top: 0, left: 0, width: "100%", transform: `translateY(${item.start}px)` }}
                >
                  <MessageRow
                    group={group}
                    density={density}
                    selected={selection.selected.has(id)}
                    focused={item.index === focusedIndex}
                    selectionActive={selection.selected.size > 0}
                    tagDefinitions={tagDefinitions}
                    onOpen={() => navigate(openPath(group))}
                    onSelectToggle={(range) =>
                      setSelection((current) =>
                        applySelectionClick(current, orderedIds, id, { toggle: !range, range }),
                      )
                    }
                    onArchive={() => moveTo(group.emailIds, "archive")}
                    onDelete={() => moveTo(group.emailIds, "trash")}
                    onToggleRead={() => act(group.emailIds, seenPatch(group.hasUnread))}
                    onToggleFlag={() => act([id], flaggedPatch(!group.hasFlagged))}
                    swipeEnabled={swipe}
                    onSwipeMenu={() => setMenuGroup(group)}
                  />
                </div>
              );
            })}
          </div>
        )}
        {query.isFetchingNextPage ? <Skeleton className="m-1.5 h-10" /> : null}
      </div>

      <Sheet open={menuGroup != null} onOpenChange={(open) => !open && setMenuGroup(null)}>
        <SheetContent side="bottom" className="pb-[max(env(safe-area-inset-bottom),12px)]">
          <SheetTitle className="truncate px-1 text-sm">{menuGroup?.newest.subject || "(no subject)"}</SheetTitle>
          {menuGroup ? (
            <div className="mt-2 grid gap-1">
              <Button
                variant="ghost"
                className="justify-start"
                onClick={() => {
                  moveTo(menuGroup.emailIds, "trash");
                  setMenuGroup(null);
                }}
              >
                <Trash2 className="size-4" /> Delete
              </Button>
              <Button
                variant="ghost"
                className="justify-start"
                onClick={() => {
                  act(menuGroup.emailIds, seenPatch(menuGroup.hasUnread));
                  setMenuGroup(null);
                }}
              >
                {menuGroup.hasUnread ? <MailOpen className="size-4" /> : <Mail className="size-4" />}
                Mark {menuGroup.hasUnread ? "read" : "unread"}
              </Button>
              <Button
                variant="ghost"
                className="justify-start"
                onClick={() => {
                  act([menuGroup.newest.id], flaggedPatch(!menuGroup.hasFlagged));
                  setMenuGroup(null);
                }}
              >
                <Star className="size-4" /> {menuGroup.hasFlagged ? "Remove star" : "Star"}
              </Button>
            </div>
          ) : null}
        </SheetContent>
      </Sheet>
    </div>
  );
}
