export interface ShortcutHelp {
  keys: string;
  description: string;
}

export const SHORTCUT_HELP: { group: string; items: ShortcutHelp[] }[] = [
  {
    group: "Navigation",
    items: [
      { keys: "j / k", description: "Next / previous conversation" },
      { keys: "Enter or o", description: "Open focused conversation" },
      { keys: "Esc", description: "Back to the list" },
      { keys: "/", description: "Search" },
      { keys: "⌘K", description: "Command palette" },
      { keys: "?", description: "This help" },
    ],
  },
  {
    group: "Actions",
    items: [
      { keys: "c", description: "Compose" },
      { keys: "r", description: "Reply" },
      { keys: "a", description: "Reply all" },
      { keys: "f", description: "Forward" },
      { keys: "e", description: "Archive" },
      { keys: "# or Del", description: "Delete" },
      { keys: "u", description: "Toggle read" },
      { keys: "s", description: "Toggle star" },
      { keys: "x", description: "Select conversation" },
      { keys: "⌘A", description: "Select all" },
    ],
  },
];
