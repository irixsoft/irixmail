import * as React from "react";
import { useNavigate, useParams, useSearchParams } from "react-router-dom";
import { motion } from "motion/react";
import {
  Button,
  ConfirmDialog,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  EmptyState,
  Spinner,
  cn,
  toast,
} from "@irixmail/shared";
import {
  ChevronLeft,
  Download,
  MoreHorizontal,
  Pencil,
  Trash2,
  UserRound,
  Users,
} from "lucide-react";

import { ContactAvatar } from "./contact-avatar";
import { ContactFields, Section } from "./contact-fields";
import { displayName, primaryEmail, sortContacts } from "./contact-display";
import { downloadText, vcardFilename } from "./download";
import { RecentEmails } from "./recent-emails";
import { cardToParsed, generateVcf } from "./vcard";
import { useContactMutation, useContacts } from "./use-contacts";
import type { ContactCard } from "./types";

export function ContactDetail() {
  const { contactId } = useParams<{ contactId: string }>();
  const navigate = useNavigate();
  const [, setParams] = useSearchParams();
  const { byId, list, query } = useContacts();
  const mutation = useContactMutation();
  const [confirming, setConfirming] = React.useState(false);

  const card = contactId ? byId[contactId] : undefined;

  if (query.isPending) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner className="size-5 text-muted-foreground" />
      </div>
    );
  }

  if (!card) {
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

  const isGroup = card.kind === "group";
  const name = displayName(card);
  const memberships = list.filter((entry) => entry.kind === "group" && entry.members?.includes(card.id));
  const members = sortContacts((card.members ?? []).map((id) => byId[id]).filter(Boolean) as ContactCard[]);

  const exportCard = () => downloadText(vcardFilename(name), generateVcf([cardToParsed(card)]));

  const remove = () =>
    mutation.mutate(
      { destroy: [card.id] },
      {
        onSuccess: () => {
          toast.success(`${name} deleted`);
          navigate("/contacts");
        },
        onError: (error) => toast.error(error.message),
      },
    );

  const edit = () => {
    if (isGroup) {
      setParams((current) => {
        current.set("edit-group", card.id);
        return current;
      });
      return;
    }
    navigate(`/contacts/${card.id}/edit`);
  };

  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <header className="flex h-12 shrink-0 items-center gap-1 border-b px-2">
        <Button
          variant="ghost"
          size="icon"
          aria-label="Back to contacts"
          className="md:hidden"
          onClick={() => navigate("/contacts")}
        >
          <ChevronLeft className="size-4" />
        </Button>
        <div className="min-w-0 flex-1 truncate px-1 text-sm font-semibold md:hidden">{name}</div>
        <div className="hidden flex-1 md:block" />
        <Button variant="ghost" size="sm" onClick={edit}>
          <Pencil className="size-3.5" /> Edit
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label="More actions">
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-44">
            <DropdownMenuItem onClick={exportCard}>
              <Download className="size-4" /> Export vCard
            </DropdownMenuItem>
            <DropdownMenuItem variant="destructive" onClick={() => setConfirming(true)}>
              <Trash2 className="size-4" /> Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-5 md:px-6">
        <motion.div
          key={card.id}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ type: "spring", stiffness: 300, damping: 30 }}
          className="mx-auto max-w-2xl space-y-6"
        >
          <div className="flex items-center gap-4">
            <ContactAvatar card={card} size="lg" />
            <div className="min-w-0">
              <h2 className="truncate text-xl font-semibold tracking-tight">{name}</h2>
              {card.nickname ? (
                <p className="truncate text-sm text-muted-foreground">“{card.nickname}”</p>
              ) : null}
              {card.jobTitle || card.organization ? (
                <p className="truncate text-sm text-muted-foreground">
                  {[card.jobTitle, card.organization].filter(Boolean).join(" · ")}
                </p>
              ) : null}
              {isGroup ? (
                <p className="font-mono text-[11px] text-muted-foreground">{members.length} members</p>
              ) : null}
            </div>
          </div>

          {isGroup ? (
            members.length > 0 ? (
              <Section title="Members">
                {members.map((member) => (
                  <button
                    key={member.id}
                    type="button"
                    onClick={() => navigate(`/contacts/${member.id}`)}
                    className="flex w-full items-center gap-2.5 px-3 py-2 text-left transition-colors hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60"
                  >
                    <ContactAvatar card={member} size="sm" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm">{displayName(member)}</span>
                      <span className="block truncate font-mono text-[11px] text-muted-foreground">
                        {primaryEmail(member)}
                      </span>
                    </span>
                  </button>
                ))}
              </Section>
            ) : (
              <EmptyState icon={Users} title="No members yet" description="Add people to this group." />
            )
          ) : null}

          <ContactFields
            card={card}
            onCompose={(value) => navigate(`/compose?to=${encodeURIComponent(value)}`)}
          />

          {memberships.length > 0 ? (
            <section>
              <h3 className="pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                Groups
              </h3>
              <div className="flex flex-wrap gap-1.5">
                {memberships.map((group) => (
                  <button
                    key={group.id}
                    type="button"
                    onClick={() => navigate(`/contacts/${group.id}`)}
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-full border bg-muted/60 px-2.5 py-1 text-xs transition-colors",
                      "hover:bg-accent focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60",
                    )}
                  >
                    <Users className="size-3" />
                    {displayName(group)}
                  </button>
                ))}
              </div>
            </section>
          ) : null}

          {!isGroup && primaryEmail(card) ? <RecentEmails email={primaryEmail(card)} /> : null}
        </motion.div>
      </div>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={`Delete ${name}?`}
        description="This removes the contact from the address book."
        confirmLabel="Delete"
        destructive
        loading={mutation.isPending}
        onConfirm={remove}
      />
    </div>
  );
}
