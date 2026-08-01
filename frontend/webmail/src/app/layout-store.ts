import * as React from "react";

export type PaneName = "folders" | "list" | "listHeight";

export type ReadingPane = "right" | "bottom" | "off";

export const READING_PANES: ReadingPane[] = ["right", "bottom", "off"];

export interface PaneLimit {
  min: number;
  initial: number;
  max: number;
}

export const PANE_LIMITS: Record<PaneName, PaneLimit> = {
  folders: { min: 180, initial: 256, max: 400 },
  list: { min: 260, initial: 384, max: 600 },
  listHeight: { min: 160, initial: 320, max: 800 },
};

export interface Layout {
  folders: number;
  list: number;
  foldersCollapsed: boolean;
  readingPane: ReadingPane;
  listHeight: number;
}

const STORAGE_KEY = "irixmail.webmail.layout";

export function clampPane(pane: PaneName, width: number): number {
  const limit = PANE_LIMITS[pane];
  return Math.min(limit.max, Math.max(limit.min, width));
}

export function loadLayout(): Layout {
  const fallback: Layout = {
    folders: PANE_LIMITS.folders.initial,
    list: PANE_LIMITS.list.initial,
    foldersCollapsed: false,
    readingPane: "right",
    listHeight: PANE_LIMITS.listHeight.initial,
  };
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return fallback;
  try {
    const parsed = JSON.parse(raw) as Partial<Layout>;
    return {
      folders: clampPane("folders", typeof parsed.folders === "number" ? parsed.folders : fallback.folders),
      list: clampPane("list", typeof parsed.list === "number" ? parsed.list : fallback.list),
      foldersCollapsed: parsed.foldersCollapsed === true,
      readingPane: READING_PANES.includes(parsed.readingPane as ReadingPane)
        ? (parsed.readingPane as ReadingPane)
        : fallback.readingPane,
      listHeight: clampPane(
        "listHeight",
        typeof parsed.listHeight === "number" ? parsed.listHeight : fallback.listHeight,
      ),
    };
  } catch {
    return fallback;
  }
}

let current: Layout | null = null;
const listeners = new Set<() => void>();

export function getLayout(): Layout {
  if (!current) current = loadLayout();
  return current;
}

export function subscribeLayout(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function setLayout(layout: Layout) {
  current = layout;
  for (const listener of [...listeners]) listener();
}

export function saveLayout(layout: Layout) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(layout));
  setLayout(layout);
}

export function useLayout(): Layout {
  return React.useSyncExternalStore(subscribeLayout, getLayout, getLayout);
}
