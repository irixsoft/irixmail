import * as React from "react";
import { cn } from "@irixmail/shared";
import { motion } from "motion/react";
import { AlertCircle, Users, X } from "lucide-react";

import type { EmailAddress } from "@/lib/mail-types";
import { usePrefersReducedMotion } from "./compose-entrance";
import {
  dedupeRecipients,
  hasSeparator,
  isValidEmail,
  parseRecipients,
  recipientInitial,
  recipientLabel,
} from "./recipients";
import { nextHighlight, type Suggestion } from "./suggestions";
import { useContactSuggestions } from "./use-contact-suggestions";

export interface RecipientFieldProps {
  inputId: string;
  label: string;
  value: EmailAddress[];
  onChange: (next: EmailAddress[]) => void;
  placeholder?: string;
  autoFocus?: boolean;
}

interface OptionRowProps {
  option: Suggestion;
  id: string;
  active: boolean;
  onHover: () => void;
  onPick: () => void;
}

function OptionRow({ option, id, active, onHover, onPick }: OptionRowProps) {
  const group = option.kind === "group";
  return (
    <div
      id={id}
      role="option"
      aria-selected={active}
      onMouseMove={onHover}
      onMouseDown={(event) => {
        event.preventDefault();
        onPick();
      }}
      className={cn(
        "flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-left",
        active ? "bg-accent text-accent-foreground" : "text-foreground",
      )}
    >
      <span className="grid size-6 shrink-0 place-items-center rounded-full bg-primary/15 text-[10px] font-semibold text-primary">
        {group ? <Users className="size-3" /> : (option.name[0] ?? "?").toUpperCase()}
      </span>
      <span className="min-w-0 flex-1 truncate text-sm">{option.name}</span>
      <span className="shrink-0 truncate font-mono text-[11px] text-muted-foreground">
        {group ? `${option.memberCount} members` : option.email}
      </span>
    </div>
  );
}

export function RecipientField({
  inputId,
  label,
  value,
  onChange,
  placeholder,
  autoFocus = false,
}: RecipientFieldProps) {
  const [draft, setDraft] = React.useState("");
  const [focused, setFocused] = React.useState(false);
  const [dismissed, setDismissed] = React.useState(false);
  const [highlight, setHighlight] = React.useState(-1);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const reduced = usePrefersReducedMotion();

  const suggestions = useContactSuggestions(
    draft,
    value.map((entry) => entry.email),
  );
  const open = focused && !dismissed && draft.trim() !== "" && suggestions.length > 0;
  const listId = `${inputId}-suggestions`;
  const optionId = (index: number) => `${inputId}-suggestion-${index}`;

  const commit = (raw: string) => {
    const parsed = parseRecipients(raw);
    if (parsed.length === 0) return;
    onChange(dedupeRecipients([...value, ...parsed]));
    setDraft("");
  };

  const remove = (index: number) => {
    onChange(value.filter((_, position) => position !== index));
    inputRef.current?.focus();
  };

  const pick = (option: Suggestion) => {
    onChange(dedupeRecipients([...value, ...option.addresses]));
    setDraft("");
    setHighlight(-1);
    setDismissed(true);
    inputRef.current?.focus();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (open) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setHighlight((current) =>
          nextHighlight(current, event.key === "ArrowDown" ? 1 : -1, suggestions.length),
        );
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissed(true);
        setHighlight(-1);
        return;
      }
      if ((event.key === "Enter" || event.key === "Tab") && highlight >= 0) {
        const option = suggestions[highlight];
        if (option) {
          event.preventDefault();
          pick(option);
          return;
        }
      }
    }
    if (event.key === "Enter" || event.key === "," || event.key === ";") {
      event.preventDefault();
      commit(draft);
      return;
    }
    if (event.key === "Backspace" && draft === "" && value.length > 0) {
      event.preventDefault();
      onChange(value.slice(0, -1));
    }
  };

  const listProps = {
    id: listId,
    role: "listbox" as const,
    "aria-label": `${label} suggestions`,
    className:
      "absolute left-0 top-full z-30 mt-1 max-h-64 w-full max-w-md overflow-hidden overflow-y-auto rounded-lg border bg-popover shadow-lg",
  };

  const options = suggestions.map((option, index) => (
    <OptionRow
      key={option.id}
      id={optionId(index)}
      option={option}
      active={index === highlight}
      onHover={() => setHighlight(index)}
      onPick={() => pick(option)}
    />
  ));

  return (
    <div
      className="relative flex min-h-8 w-full flex-wrap items-center gap-1.5 py-1"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          event.preventDefault();
          inputRef.current?.focus();
        }
      }}
    >
      {value.map((entry, index) => {
        const valid = isValidEmail(entry.email);
        return (
          <span
            key={`${entry.email.toLowerCase()}-${index}`}
            title={entry.email}
            className={cn(
              "group inline-flex max-w-full items-center gap-1.5 rounded-full border py-0.5 pl-1 pr-1.5 text-xs transition-colors",
              valid
                ? "border-border/70 bg-muted/70 text-foreground"
                : "border-destructive/50 bg-destructive/10 text-destructive",
            )}
          >
            {valid ? (
              <span className="grid size-4 shrink-0 place-items-center rounded-full bg-primary/15 text-[9px] font-semibold text-primary">
                {recipientInitial(entry)}
              </span>
            ) : (
              <AlertCircle className="size-3.5 shrink-0" />
            )}
            <span className={cn("truncate", entry.name ? undefined : "font-mono text-[11px]")}>
              {recipientLabel(entry)}
            </span>
            <button
              type="button"
              aria-label={`Remove ${entry.email}`}
              onClick={() => remove(index)}
              className="shrink-0 rounded-full text-muted-foreground/70 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/50"
            >
              <X className="size-3" />
            </button>
          </span>
        );
      })}
      <input
        id={inputId}
        ref={inputRef}
        autoFocus={autoFocus}
        aria-label={label}
        role="combobox"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && highlight >= 0 ? optionId(highlight) : undefined}
        value={draft}
        placeholder={value.length === 0 ? placeholder : undefined}
        onChange={(event) => {
          setDraft(event.target.value);
          setHighlight(-1);
          setDismissed(false);
        }}
        onFocus={() => setFocused(true)}
        onKeyDown={onKeyDown}
        onBlur={() => {
          setFocused(false);
          commit(draft);
        }}
        onPaste={(event) => {
          const text = event.clipboardData.getData("text");
          if (!hasSeparator(text)) return;
          event.preventDefault();
          commit(`${draft}${text}`);
        }}
        className="min-w-[10rem] flex-1 bg-transparent py-0.5 font-mono text-sm outline-none placeholder:font-sans placeholder:text-muted-foreground/70"
      />
      {open ? (
        reduced ? (
          <div {...listProps}>{options}</div>
        ) : (
          <motion.div
            {...listProps}
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ type: "spring", stiffness: 320, damping: 26 }}
          >
            {options}
          </motion.div>
        )
      ) : null}
    </div>
  );
}
