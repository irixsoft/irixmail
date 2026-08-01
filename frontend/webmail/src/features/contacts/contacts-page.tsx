import * as React from "react";
import { Outlet, useMatch, useNavigate, useSearchParams } from "react-router-dom";
import {
  Button,
  EmptyState,
  Sheet,
  SheetContent,
  SheetTitle,
  toast,
} from "@irixmail/shared";
import { BookUser, PanelLeft } from "lucide-react";

import { useIsMobile } from "@/app/use-is-mobile";
import { AddressBookPane } from "./address-book-pane";
import { ContactList } from "./contact-list";
import { displayName, filterContacts } from "./contact-display";
import { downloadText, vcardFilename } from "./download";
import { GroupDialog } from "./group-dialog";
import { ImportDialog } from "./import-dialog";
import { cardToParsed, generateVcf } from "./vcard";
import { useAddressBookMutation, useAddressBooks, useContactMutation, useContacts } from "./use-contacts";
import type { ContactPayload } from "./types";

export function ContactsPage() {
  const isMobile = useIsMobile();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const [booksOpen, setBooksOpen] = React.useState(false);

  const { list: books, defaultId, query: booksQuery } = useAddressBooks();
  const { list, individuals, groups, byId, query } = useContacts();
  const bookMutation = useAddressBookMutation();
  const contactMutation = useContactMutation();

  const detailMatch = useMatch("/contacts/:contactId");
  const editMatch = useMatch("/contacts/:contactId/edit");
  const routeId = editMatch?.params["contactId"] ?? detailMatch?.params["contactId"] ?? null;
  const hasDetail = Boolean(detailMatch || editMatch);
  const selectedId = routeId === "new" ? null : routeId;

  const search = params.get("q") ?? "";
  const activeBook = params.get("book");
  const activeGroup = params.get("group");
  const editGroup = params.get("edit-group");
  const importing = params.get("import") === "1";

  const patch = (changes: Record<string, string | null>) =>
    setParams(
      (current) => {
        for (const [key, value] of Object.entries(changes)) {
          if (value === null) current.delete(key);
          else current.set(key, value);
        }
        return current;
      },
      { replace: true },
    );

  const counts: Record<string, number> = {};
  for (const card of individuals) counts[card.addressBookId] = (counts[card.addressBookId] ?? 0) + 1;

  const group = activeGroup ? byId[activeGroup] : undefined;
  const scoped = React.useMemo(() => {
    if (group) {
      const members = new Set(group.members ?? []);
      return individuals.filter((card) => members.has(card.id));
    }
    if (activeBook) return individuals.filter((card) => card.addressBookId === activeBook);
    return individuals;
  }, [group, activeBook, individuals]);

  const visible = React.useMemo(() => filterContacts(scoped, search), [scoped, search]);

  const title = group
    ? displayName(group)
    : activeBook
      ? (books.find((book) => book.id === activeBook)?.name ?? "Contacts")
      : "Contacts";

  const newContact = () => navigate(`/contacts/new${activeBook ? `?book=${activeBook}` : ""}`);

  const createBook = (name: string) =>
    bookMutation.mutate(
      { create: { b1: { name } } },
      { onError: (error) => toast.error(error.message) },
    );

  const exportAll = () => {
    const cards = scoped.length > 0 ? scoped : individuals;
    if (cards.length === 0) {
      toast.error("Nothing to export");
      return;
    }
    downloadText(vcardFilename(title), generateVcf(cards.map(cardToParsed)));
  };

  const submitGroup = (payload: ContactPayload, id: string | null) =>
    contactMutation.mutate(
      id ? { update: { [id]: payload } } : { create: { g1: payload } },
      {
        onSuccess: () => {
          toast.success(id ? "Group saved" : "Group created");
          patch({ "edit-group": null });
        },
        onError: (error) => toast.error(error.message),
      },
    );

  const pane = (
    <AddressBookPane
      books={books}
      groups={groups}
      counts={counts}
      total={individuals.length}
      activeBook={activeBook}
      activeGroup={activeGroup}
      loading={booksQuery.isPending}
      onSelectBook={(id) => {
        patch({ book: id, group: null });
        setBooksOpen(false);
      }}
      onSelectGroup={(id) => {
        patch({ group: id, book: null });
        setBooksOpen(false);
      }}
      onCreateBook={createBook}
      onNewContact={() => {
        setBooksOpen(false);
        newContact();
      }}
      onNewGroup={() => {
        setBooksOpen(false);
        patch({ "edit-group": "new" });
      }}
      onImport={() => {
        setBooksOpen(false);
        patch({ import: "1" });
      }}
      onExportAll={() => {
        setBooksOpen(false);
        exportAll();
      }}
    />
  );

  const listPane = (
    <ContactList
      title={title}
      contacts={visible}
      query={search}
      onQuery={(value) => patch({ q: value || null })}
      selectedId={selectedId}
      loading={query.isPending}
      error={query.isError ? (query.error as Error) : null}
      onRetry={() => void query.refetch()}
      onOpen={(card) => navigate(`/contacts/${card.id}`)}
      onNew={newContact}
      leading={
        isMobile ? (
          <Button variant="ghost" size="icon" aria-label="Address books" onClick={() => setBooksOpen(true)}>
            <PanelLeft className="size-4" />
          </Button>
        ) : null
      }
    />
  );

  const dialogs = (
    <>
      {editGroup ? (
        <GroupDialog
          key={editGroup}
          open
          group={editGroup === "new" ? null : (byId[editGroup] ?? null)}
          contacts={individuals}
          books={books}
          defaultBookId={activeBook || defaultId}
          pending={contactMutation.isPending}
          onOpenChange={(open) => {
            if (!open) patch({ "edit-group": null });
          }}
          onSubmit={submitGroup}
        />
      ) : null}
      <ImportDialog
        open={importing}
        books={books}
        defaultBookId={activeBook || defaultId}
        onOpenChange={(open) => patch({ import: open ? "1" : null })}
      />
      <Sheet open={booksOpen} onOpenChange={setBooksOpen}>
        <SheetContent side="left" className="w-72 p-0">
          <SheetTitle className="sr-only">Address books</SheetTitle>
          {pane}
        </SheetContent>
      </Sheet>
    </>
  );

  if (isMobile) {
    return (
      <div className="h-full min-w-0">
        {hasDetail ? <Outlet /> : listPane}
        {dialogs}
      </div>
    );
  }

  return (
    <div className="flex h-full min-w-0">
      <div className="w-[220px] shrink-0 border-r border-sidebar-border">{pane}</div>
      <div className="w-[300px] shrink-0 border-r">{listPane}</div>
      <div className="min-w-0 flex-1 overflow-hidden">
        {hasDetail ? (
          <Outlet />
        ) : (
          <div className="flex h-full items-center justify-center p-6">
            <EmptyState
              icon={BookUser}
              title={list.length === 0 ? "Your address book is empty" : "Select a contact"}
              description={
                list.length === 0
                  ? "Add someone, or import a vCard file to get started."
                  : "Pick a name on the left to see the details here."
              }
              action={
                <Button size="sm" variant="secondary" onClick={newContact}>
                  New contact
                </Button>
              }
            />
          </div>
        )}
      </div>
      {dialogs}
    </div>
  );
}
