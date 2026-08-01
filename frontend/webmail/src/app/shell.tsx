import * as React from "react";
import { Outlet, useMatch, useNavigate } from "react-router-dom";
import { FolderPane, FolderRail } from "@/features/mail/folder-pane";
import { InstallPrompt } from "@/pwa/install-prompt";
import { CommandPalette } from "@/features/shortcuts/command-palette";
import { ShortcutHelpModal } from "@/features/shortcuts/help-modal";
import { useShortcuts } from "@/features/shortcuts/use-shortcuts";
import { PANE_LIMITS, clampPane, getLayout, saveLayout, setLayout, useLayout } from "./layout-store";
import { Rail } from "./rail";
import { ResizeHandle } from "./resize-handle";
import { TabBar } from "./tab-bar";
import { useIsMobile } from "./use-is-mobile";

export function Shell() {
  const layout = useLayout();
  const [paletteOpen, setPaletteOpen] = React.useState(false);
  const [helpOpen, setHelpOpen] = React.useState(false);
  const navigate = useNavigate();

  useShortcuts({
    c: () => navigate("/compose"),
    "/": () => {
      const input = document.querySelector<HTMLInputElement>("[data-search-input]");
      if (input) input.focus();
      else navigate("/search");
    },
    "?": () => setHelpOpen(true),
    "mod+k": () => setPaletteOpen((current) => !current),
  });

  const isMobile = useIsMobile();
  const inConversation = Boolean(useMatch("/:mailboxId/:emailId"));
  const inCompose = Boolean(useMatch("/compose"));
  const inContactDetail = Boolean(useMatch("/contacts/:contactId") || useMatch("/contacts/:contactId/edit"));
  const showTabs = !inConversation && !inCompose && !inContactDetail;

  if (isMobile) {
    return (
      <div className="flex h-dvh flex-col overflow-hidden bg-background text-foreground">
        <main className="min-h-0 flex-1 overflow-hidden">
          <Outlet />
        </main>
        {showTabs ? <TabBar /> : null}
        <InstallPrompt />
        <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
        <ShortcutHelpModal open={helpOpen} onOpenChange={setHelpOpen} />
      </div>
    );
  }

  return (
    <div className="flex h-dvh overflow-hidden bg-background text-foreground">
      <Rail />
      {layout.foldersCollapsed ? (
        <FolderRail onExpand={() => saveLayout({ ...getLayout(), foldersCollapsed: false })} />
      ) : (
        <>
          <div style={{ width: layout.folders }} className="shrink-0 border-r border-sidebar-border">
            <FolderPane onCollapse={() => saveLayout({ ...getLayout(), foldersCollapsed: true })} />
          </div>
          <ResizeHandle
            label="Resize folder pane"
            value={layout.folders}
            min={PANE_LIMITS.folders.min}
            max={PANE_LIMITS.folders.max}
            onChange={(width) => setLayout({ ...getLayout(), folders: clampPane("folders", width) })}
            onCommit={(width) => saveLayout({ ...getLayout(), folders: clampPane("folders", width) })}
            onReset={() => saveLayout({ ...getLayout(), folders: PANE_LIMITS.folders.initial })}
          />
        </>
      )}
      <main className="min-w-0 flex-1 overflow-hidden">
        <Outlet />
      </main>
      <InstallPrompt />
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
      <ShortcutHelpModal open={helpOpen} onOpenChange={setHelpOpen} />
    </div>
  );
}
