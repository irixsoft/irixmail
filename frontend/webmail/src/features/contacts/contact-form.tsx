import * as React from "react";
import { motion } from "motion/react";
import { ChevronDown, ChevronLeft, ImageUp, Plus, Trash2 } from "lucide-react";
import {
  Avatar, AvatarFallback, AvatarImage, Button, Input, Label, Select, SelectContent, SelectItem,
  SelectTrigger, SelectValue, Separator, Spinner, Textarea, cn, toast,
} from "@irixmail/shared";

import { usePrefersReducedMotion } from "@/features/compose/compose-entrance";
import { composedFullName, type AddressRow, type ContactFormValues, type LabeledRow } from "./contact-form-mapping";
import { fileToPhoto, photoSrc } from "./photo";
import { useContactForm } from "./use-contact-form";
import { CONTACT_LABELS, type AddressBook, type ContactPayload } from "./types";

export interface ContactFormProps {
  initial: ContactFormValues;
  books: AddressBook[];
  editing: boolean;
  pending: boolean;
  onSubmit: (payload: ContactPayload) => void;
  onCancel: () => void;
}

type TextKey = "prefix" | "given" | "additional" | "surname" | "suffix" | "fullName" | "nickname"
  | "organization" | "jobTitle" | "birthday";

type AddressKey = "street" | "city" | "region" | "postcode" | "country";

type TextFieldProps = { id: string; label: string; containerClassName?: string } & React.ComponentProps<typeof Input>;

const MONO = "font-mono text-sm";

const ROW_SECTIONS = [
  { key: "emails", noun: "Email", type: "email", inputMode: "email", placeholder: "name@example.com" },
  { key: "phones", noun: "Phone", type: "tel", inputMode: "tel", placeholder: "+1 555 0100" },
] as const;

function initialsOf(name: string): string {
  const parts = name.split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  const first = parts[0]?.[0] ?? "";
  const last = parts.length > 1 ? (parts[parts.length - 1]?.[0] ?? "") : "";
  return (first + last).toUpperCase();
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{children}</h2>;
}

function TextField({ id, label, containerClassName, ...props }: TextFieldProps) {
  return (
    <div className={cn("space-y-1.5", containerClassName)}>
      <Label htmlFor={id} className="text-xs text-muted-foreground">
        {label}
      </Label>
      <Input id={id} {...props} />
    </div>
  );
}

function LabelSelect({
  value,
  ariaLabel,
  onChange,
}: {
  value: string;
  ariaLabel: string;
  onChange: (value: string) => void;
}) {
  const known = (CONTACT_LABELS as readonly string[]).includes(value);
  return (
    <Select value={known ? value : "other"} onValueChange={onChange}>
      <SelectTrigger aria-label={ariaLabel} className="w-28 shrink-0 capitalize">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {CONTACT_LABELS.map((label) => (
          <SelectItem key={label} value={label} className="capitalize">{label}</SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

interface ValueRowsProps {
  noun: string;
  idPrefix: string;
  rows: LabeledRow[];
  type: string;
  inputMode: "email" | "tel";
  placeholder: string;
  onPatch: (index: number, patch: Partial<LabeledRow>) => void;
  onRemove: (index: number) => void;
  onAdd: () => void;
}

function ValueRows(props: ValueRowsProps) {
  const { noun, idPrefix, rows, onPatch, onRemove, onAdd } = props;
  return (
    <div className="space-y-2">
      {rows.map((row, index) => (
        <div key={`${idPrefix}-${index}`} className="flex items-end gap-2">
          <TextField
            id={`${idPrefix}-${index}`}
            label={`${noun} ${index + 1}`}
            containerClassName="min-w-0 flex-1"
            type={props.type}
            inputMode={props.inputMode}
            placeholder={props.placeholder}
            value={row.value}
            className={MONO}
            onChange={(event) => onPatch(index, { value: event.target.value })}
          />
          <LabelSelect
            value={row.label}
            ariaLabel={`${noun} ${index + 1} label`}
            onChange={(label) => onPatch(index, { label })}
          />
          <Button
            type="button"
            variant="ghost"
            size="icon"
            aria-label={`Remove ${noun.toLowerCase()} ${index + 1}`}
            onClick={() => onRemove(index)}
          >
            <Trash2 className="size-4" />
          </Button>
        </div>
      ))}
      <Button type="button" variant="ghost" size="sm" className="text-muted-foreground" onClick={onAdd}>
        <Plus className="size-4" />
        Add {noun.toLowerCase()}
      </Button>
    </div>
  );
}

export function ContactForm({ initial, books, editing, pending, onSubmit, onCancel }: ContactFormProps) {
  const form = useContactForm(initial);
  const { values, set } = form;
  const uid = React.useId();
  const reduced = usePrefersReducedMotion();
  const fileRef = React.useRef<HTMLInputElement>(null);
  const [more, setMore] = React.useState(false);
  const [uploading, setUploading] = React.useState(false);

  const display = values.fullName.trim() || composedFullName(values);

  const field = (key: TextKey, label: string, extra?: Omit<TextFieldProps, "id" | "label">) => (
    <TextField
      id={`${uid}-${key}`} label={label} value={values[key]}
      onChange={(event) => set(key, event.target.value)} {...extra}
    />
  );

  const addressField = (key: AddressKey, label: string, className?: string) => (
    <TextField
      id={`${uid}-${key}`} label={label} value={values.address[key]} className={className}
      onChange={(event) => form.setAddress({ [key]: event.target.value } as Partial<AddressRow>)}
    />
  );

  const readPhoto = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    setUploading(true);
    try {
      set("photo", await fileToPhoto(file));
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "That image could not be read");
    } finally {
      setUploading(false);
    }
  };

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    form.markSubmitted();
    if (form.error) return;
    onSubmit(form.payload());
  };

  return (
    <form onSubmit={submit} className="flex h-full min-h-0 flex-col bg-background">
      <header className="sticky top-0 z-10 flex items-center gap-2 border-b bg-background/95 px-3 py-2 backdrop-blur md:px-6">
        <Button type="button" variant="ghost" size="icon" aria-label="Cancel" className="md:hidden" onClick={onCancel}>
          <ChevronLeft className="size-4" />
        </Button>
        <h1 className="text-sm font-semibold">{editing ? "Edit contact" : "New contact"}</h1>
        <div className="flex-1" />
        <Button type="button" variant="ghost" className="hidden md:inline-flex" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={pending} className="bg-gradient-to-br from-primary to-primary/80 shadow-sm">
          {pending ? <Spinner label="Saving" className="text-primary-foreground" /> : null}
          Save
        </Button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4 md:px-6">
        <div className="mx-auto max-w-2xl space-y-6">
          {form.showError ? <p role="alert" className="text-sm text-destructive">{form.showError}</p> : null}

          <section className="flex flex-col gap-4 sm:flex-row sm:items-start sm:gap-6">
            <div className="flex flex-col items-center gap-2">
              <Avatar className="size-20">
                {values.photo ? <AvatarImage src={photoSrc(values.photo)} alt="" /> : null}
                <AvatarFallback className="bg-primary/12 text-lg font-semibold text-primary">
                  {initialsOf(display)}
                </AvatarFallback>
              </Avatar>
              <input ref={fileRef} type="file" accept="image/*" className="hidden" onChange={readPhoto} />
              <div className="flex items-center gap-1">
                <Button
                  type="button" variant="ghost" size="sm"
                  disabled={uploading} onClick={() => fileRef.current?.click()}
                >
                  {uploading ? <Spinner label="Resizing" /> : <ImageUp className="size-4" />}
                  {values.photo ? "Replace" : "Upload photo"}
                </Button>
                {values.photo ? (
                  <Button
                    type="button" variant="ghost" size="sm"
                    className="text-muted-foreground" onClick={() => set("photo", null)}
                  >
                    Remove
                  </Button>
                ) : null}
              </div>
            </div>

            <div className="min-w-0 flex-1 space-y-3">
              <div className="grid gap-3 sm:grid-cols-2">
                {field("given", "Given name", { autoComplete: "given-name" })}
                {field("surname", "Surname", { autoComplete: "family-name" })}
              </div>

              <button
                type="button"
                aria-expanded={more}
                onClick={() => setMore((open) => !open)}
                className="inline-flex items-center gap-1.5 rounded-md text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              >
                More name fields
                {reduced ? (
                  <ChevronDown className={cn("size-3.5", more && "rotate-180")} />
                ) : (
                  <motion.span className="inline-flex" animate={{ rotate: more ? 180 : 0 }} transition={{ duration: 0.18 }}>
                    <ChevronDown className="size-3.5" />
                  </motion.span>
                )}
              </button>

              {more ? (
                <div className="grid gap-3 sm:grid-cols-2">
                  {field("prefix", "Prefix")}
                  {field("additional", "Middle name")}
                  {field("suffix", "Suffix")}
                  {field("nickname", "Nickname")}
                  {field("fullName", "Display name", { containerClassName: "sm:col-span-2" })}
                </div>
              ) : null}
            </div>
          </section>

          <Separator />

          <section className="grid gap-3 sm:grid-cols-2">
            {field("organization", "Organization", { autoComplete: "organization" })}
            {field("jobTitle", "Job title", { autoComplete: "organization-title" })}
          </section>

          {ROW_SECTIONS.map((section) => (
            <React.Fragment key={section.key}>
              <Separator />
              <section className="space-y-3">
                <SectionTitle>{section.noun}</SectionTitle>
                <ValueRows
                  noun={section.noun}
                  idPrefix={`${uid}-${section.key}`}
                  rows={values[section.key]}
                  type={section.type}
                  inputMode={section.inputMode}
                  placeholder={section.placeholder}
                  onPatch={(index, patch) => form.setRow(section.key, index, patch)}
                  onRemove={(index) => form.removeRow(section.key, index)}
                  onAdd={() => form.addRow(section.key)}
                />
              </section>
            </React.Fragment>
          ))}

          <Separator />

          <section className="space-y-3">
            <SectionTitle>Address</SectionTitle>
            {addressField("street", "Street")}
            <div className="grid gap-3 sm:grid-cols-2">
              {addressField("city", "City")}
              {addressField("region", "Region")}
              {addressField("postcode", "Postcode", MONO)}
              {addressField("country", "Country")}
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs text-muted-foreground">Address label</Label>
              <LabelSelect
                value={values.address.label}
                ariaLabel="Address label"
                onChange={(label) => form.setAddress({ label })}
              />
            </div>
          </section>

          <Separator />

          <section className="space-y-3">
            <SectionTitle>Details</SectionTitle>
            <div className="grid gap-3 sm:grid-cols-2">
              {field("birthday", "Birthday", { type: "date", className: MONO })}
              <div className="space-y-1.5">
                <Label htmlFor={`${uid}-book`} className="text-xs text-muted-foreground">
                  Address book
                </Label>
                <Select value={values.addressBookId} onValueChange={(value) => set("addressBookId", value)}>
                  <SelectTrigger id={`${uid}-book`} className="w-full">
                    <SelectValue placeholder="Pick an address book" />
                  </SelectTrigger>
                  <SelectContent>
                    {books.map((book) => (
                      <SelectItem key={book.id} value={book.id}>{book.name}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor={`${uid}-note`} className="text-xs text-muted-foreground">
                Note
              </Label>
              <Textarea id={`${uid}-note`} rows={4} value={values.note} onChange={(event) => set("note", event.target.value)} />
            </div>
          </section>
        </div>
      </div>
    </form>
  );
}
