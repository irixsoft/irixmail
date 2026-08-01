import { Avatar, AvatarFallback, AvatarImage, cn } from "@irixmail/shared";
import { Users } from "lucide-react";

import { contactInitials, displayName } from "./contact-display";
import { photoSrc } from "./photo";
import type { ContactCard } from "./types";

const SIZES = {
  sm: { root: "size-8", text: "text-[11px]", icon: "size-3.5" },
  md: { root: "size-9", text: "text-xs", icon: "size-4" },
  lg: { root: "size-20", text: "text-2xl", icon: "size-8" },
} as const;

export function ContactAvatar({
  card,
  size = "sm",
  className,
}: {
  card: ContactCard;
  size?: keyof typeof SIZES;
  className?: string;
}) {
  const scale = SIZES[size];
  const source = card.photo ? photoSrc(card.photo) : null;
  const isGroup = card.kind === "group";

  return (
    <Avatar className={cn(scale.root, "shrink-0", className)}>
      {source ? <AvatarImage src={source} alt="" className="object-cover" /> : null}
      <AvatarFallback
        className={cn(
          "bg-primary/12 font-semibold text-primary ring-1 ring-inset ring-primary/15",
          scale.text,
        )}
      >
        {isGroup ? (
          <Users className={scale.icon} aria-hidden />
        ) : (
          <span aria-hidden>{contactInitials(card)}</span>
        )}
        <span className="sr-only">{displayName(card)}</span>
      </AvatarFallback>
    </Avatar>
  );
}
