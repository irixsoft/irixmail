import { useNavigate, useParams, useSearchParams } from "react-router-dom";
import { Button, EmptyState, Spinner, toast } from "@irixmail/shared";
import { UserRound } from "lucide-react";

import { ContactForm } from "./contact-form";
import { contactToForm, emptyContactForm } from "./contact-form-mapping";
import { useAddressBooks, useContactMutation, useContacts } from "./use-contacts";
import type { ContactPayload } from "./types";

export function ContactEdit() {
  const { contactId } = useParams<{ contactId: string }>();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const { byId, query } = useContacts();
  const { list: books, defaultId, query: booksQuery } = useAddressBooks();
  const mutation = useContactMutation();

  const editing = Boolean(contactId);
  const card = contactId ? byId[contactId] : undefined;

  if (booksQuery.isPending || (editing && query.isPending)) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner className="size-5 text-muted-foreground" />
      </div>
    );
  }

  if (editing && !card) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <EmptyState
          icon={UserRound}
          title="Contact not found"
          description="It may have been deleted."
          action={
            <Button size="sm" variant="secondary" onClick={() => navigate("/contacts")}>
              Back to contacts
            </Button>
          }
        />
      </div>
    );
  }

  const initial = card ? contactToForm(card) : emptyContactForm(params.get("book") || defaultId);

  const submit = (payload: ContactPayload) => {
    if (card) {
      mutation.mutate(
        { update: { [card.id]: payload } },
        {
          onSuccess: () => {
            toast.success("Contact saved");
            navigate(`/contacts/${card.id}`);
          },
          onError: (error) => toast.error(error.message),
        },
      );
      return;
    }
    mutation.mutate(
      { create: { c1: payload } },
      {
        onSuccess: (result) => {
          const created = (result["created"] as Record<string, { id?: string }> | undefined)?.["c1"];
          toast.success("Contact created");
          navigate(created?.id ? `/contacts/${created.id}` : "/contacts");
        },
        onError: (error) => toast.error(error.message),
      },
    );
  };

  return (
    <ContactForm
      key={card?.id ?? "new"}
      initial={initial}
      books={books}
      editing={Boolean(card)}
      pending={mutation.isPending}
      onSubmit={submit}
      onCancel={() => navigate(card ? `/contacts/${card.id}` : "/contacts")}
    />
  );
}
