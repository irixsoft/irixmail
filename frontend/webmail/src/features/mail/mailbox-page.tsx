import * as React from "react";
import { Outlet, useMatch, useNavigate, useParams } from "react-router-dom";
import { Button, Sheet, SheetContent, SheetTitle } from "@irixmail/shared";
import { PanelLeft, Search } from "lucide-react";

import { PANE_LIMITS, clampPane, getLayout, saveLayout, setLayout, useLayout } from "@/app/layout-store";
import { ResizeHandle } from "@/app/resize-handle";
import { useIsMobile } from "@/app/use-is-mobile";
import { initDensity } from "./density";
import { FolderPane } from "./folder-pane";
import { MessageList } from "./message-list";
import { useMailboxes } from "./use-mailboxes";
import { mailboxLabel } from "@/lib/mail-types";

export function MailboxPage() {
  const { mailboxId } = useParams<{ mailboxId: string }>();
  const navigate = useNavigate();
  const { byId } = useMailboxes();
  const layout = useLayout();
  const [density] = React.useState(initDensity);
  const [foldersOpen, setFoldersOpen] = React.useState(false);

  const mailbox = mailboxId ? byId[mailboxId] : undefined;
  const title = mailbox ? mailboxLabel(mailbox) : "Mail";
  const isMobile = useIsMobile();
  const inConversation = Boolean(useMatch("/:mailboxId/:emailId"));

  if (!mailboxId) return null;

  if (isMobile) {
    if (inConversation) return <Outlet />;
    return (
      <>
        <MessageList
          filter={{ inMailbox: mailboxId }}
          filterKey={mailboxId}
          title={title}
          density="cozy"
          swipe
          openPath={(group) => `/${mailboxId}/${group.newest.id}`}
          leading={
            <Button variant="ghost" size="icon" aria-label="Folders" onClick={() => setFoldersOpen(true)}>
              <PanelLeft className="size-4" />
            </Button>
          }
          trailing={
            <Button variant="ghost" size="icon" aria-label="Search mail" onClick={() => navigate("/search")}>
              <Search className="size-4" />
            </Button>
          }
        />
        <Sheet open={foldersOpen} onOpenChange={setFoldersOpen}>
          <SheetContent side="left" className="w-72 p-0">
            <SheetTitle className="sr-only">Folders</SheetTitle>
            <FolderPane onNavigate={() => setFoldersOpen(false)} />
          </SheetContent>
        </Sheet>
      </>
    );
  }

  const list = (
    <MessageList
      filter={{ inMailbox: mailboxId }}
      filterKey={mailboxId}
      title={title}
      density={density}
      openPath={(group) => `/${mailboxId}/${group.newest.id}`}
    />
  );

  if (layout.readingPane === "off") {
    if (inConversation) return <Outlet />;
    return <div className="h-full">{list}</div>;
  }

  if (layout.readingPane === "bottom") {
    return (
      <div className="flex h-full flex-col">
        <div style={{ height: layout.listHeight }} className="min-h-0 shrink-0 border-b">
          {list}
        </div>
        <ResizeHandle
          axis="y"
          label="Resize message list"
          value={layout.listHeight}
          min={PANE_LIMITS.listHeight.min}
          max={PANE_LIMITS.listHeight.max}
          onChange={(height) => setLayout({ ...getLayout(), listHeight: clampPane("listHeight", height) })}
          onCommit={(height) => saveLayout({ ...getLayout(), listHeight: clampPane("listHeight", height) })}
          onReset={() => saveLayout({ ...getLayout(), listHeight: PANE_LIMITS.listHeight.initial })}
        />
        <div className="min-h-0 flex-1 overflow-hidden">
          <Outlet />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full">
      <div style={{ width: layout.list }} className="shrink-0 border-r">
        {list}
      </div>
      <ResizeHandle
        label="Resize message list"
        value={layout.list}
        min={PANE_LIMITS.list.min}
        max={PANE_LIMITS.list.max}
        onChange={(width) => setLayout({ ...getLayout(), list: clampPane("list", width) })}
        onCommit={(width) => saveLayout({ ...getLayout(), list: clampPane("list", width) })}
        onReset={() => saveLayout({ ...getLayout(), list: PANE_LIMITS.list.initial })}
      />
      <div className="min-w-0 flex-1 overflow-hidden">
        <Outlet />
      </div>
    </div>
  );
}
