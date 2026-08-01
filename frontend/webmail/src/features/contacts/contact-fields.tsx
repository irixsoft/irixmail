import * as React from "react";
import { Badge, Button, toast } from "@irixmail/shared";
import { Cake, Copy, MapPin, Phone, Send, StickyNote } from "lucide-react";

import { formatBirthday } from "./contact-display";
import type { ContactAddress, ContactCard } from "./types";

function addressLines(address: ContactAddress): string[] {
  const cityLine = [address.city, address.region, address.postcode].filter(Boolean).join(" ");
  return [address.street, cityLine, address.country].filter((line): line is string => Boolean(line));
}

async function copy(value: string) {
  try {
    await navigator.clipboard.writeText(value);
    toast.success("Copied to the clipboard");
  } catch {
    toast.error("Could not copy");
  }
}

export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h3 className="pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">{title}</h3>
      <div className="divide-y rounded-lg border">{children}</div>
    </section>
  );
}

function Row({
  icon: Icon,
  label,
  children,
  actions,
}: {
  icon: React.ElementType;
  label?: string | null;
  children: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-2.5 px-3 py-2">
      <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">{children}</div>
      {label ? (
        <Badge variant="secondary" className="mt-0.5 shrink-0 font-normal capitalize">
          {label}
        </Badge>
      ) : null}
      {actions ? <div className="flex shrink-0 items-center gap-0.5">{actions}</div> : null}
    </div>
  );
}

function CopyButton({ value, label }: { value: string; label: string }) {
  return (
    <Button variant="ghost" size="icon" className="size-7" aria-label={label} onClick={() => void copy(value)}>
      <Copy className="size-3.5" />
    </Button>
  );
}

export function ContactFields({
  card,
  onCompose,
}: {
  card: ContactCard;
  onCompose: (email: string) => void;
}) {
  const emails = card.emails ?? [];
  const phones = card.phones ?? [];
  const addresses = (card.addresses ?? []).filter((address) => addressLines(address).length > 0);

  return (
    <>
      {emails.length > 0 ? (
        <Section title="Email">
          {emails.map((entry, index) => (
            <Row
              key={`${entry.value}-${index}`}
              icon={Send}
              label={entry.label}
              actions={
                <>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Compose to ${entry.value}`}
                    onClick={() => onCompose(entry.value)}
                  >
                    <Send className="size-3.5" />
                  </Button>
                  <CopyButton value={entry.value} label={`Copy ${entry.value}`} />
                </>
              }
            >
              <span className="block truncate font-mono text-sm">{entry.value}</span>
            </Row>
          ))}
        </Section>
      ) : null}

      {phones.length > 0 ? (
        <Section title="Phone">
          {phones.map((entry, index) => (
            <Row
              key={`${entry.value}-${index}`}
              icon={Phone}
              label={entry.label}
              actions={<CopyButton value={entry.value} label={`Copy ${entry.value}`} />}
            >
              <span className="block truncate font-mono text-sm">{entry.value}</span>
            </Row>
          ))}
        </Section>
      ) : null}

      {addresses.length > 0 ? (
        <Section title="Address">
          {addresses.map((address, index) => {
            const lines = addressLines(address);
            return (
              <Row
                key={index}
                icon={MapPin}
                label={address.label}
                actions={<CopyButton value={lines.join("\n")} label="Copy address" />}
              >
                {lines.map((line) => (
                  <span key={line} className="block text-sm">
                    {line}
                  </span>
                ))}
              </Row>
            );
          })}
        </Section>
      ) : null}

      {card.birthday || card.note ? (
        <Section title="Details">
          {card.birthday ? (
            <Row icon={Cake}>
              <span className="block font-mono text-sm">{formatBirthday(card.birthday)}</span>
            </Row>
          ) : null}
          {card.note ? (
            <Row icon={StickyNote}>
              <span className="block whitespace-pre-wrap text-sm">{card.note}</span>
            </Row>
          ) : null}
        </Section>
      ) : null}
    </>
  );
}
