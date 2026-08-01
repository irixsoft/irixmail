import {
  CONTACT_LABELS,
  type ContactAddress,
  type ContactCard,
  type ContactPayload,
  type ContactPhoto,
  type LabeledValue,
} from "./types";

export interface LabeledRow {
  value: string;
  label: string;
}

export interface AddressRow {
  street: string;
  city: string;
  region: string;
  postcode: string;
  country: string;
  label: string;
}

export interface ContactFormValues {
  addressBookId: string;
  prefix: string;
  given: string;
  additional: string;
  surname: string;
  suffix: string;
  fullName: string;
  nickname: string;
  organization: string;
  jobTitle: string;
  emails: LabeledRow[];
  phones: LabeledRow[];
  address: AddressRow;
  birthday: string;
  note: string;
  photo: ContactPhoto | null;
}

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const BIRTHDAY_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

function text(value: string | null | undefined): string {
  return value ?? "";
}

function blankAddress(): AddressRow {
  return { street: "", city: "", region: "", postcode: "", country: "", label: "home" };
}

function blankRow(): LabeledRow {
  return { value: "", label: "home" };
}

function toRows(values: LabeledValue[] | null | undefined): LabeledRow[] {
  const rows = (values ?? []).map((entry) => ({ value: entry.value, label: entry.label ?? "other" }));
  return rows.length > 0 ? rows : [blankRow()];
}

function normalizeLabel(label: string): string | null {
  const trimmed = label.trim().toLowerCase();
  return (CONTACT_LABELS as readonly string[]).includes(trimmed) ? trimmed : null;
}

function toPayloadRows(rows: LabeledRow[]): LabeledValue[] {
  return rows
    .map((row) => ({ value: row.value.trim(), label: normalizeLabel(row.label) }))
    .filter((row) => row.value.length > 0);
}

function orNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function emptyContactForm(addressBookId: string): ContactFormValues {
  return {
    addressBookId,
    prefix: "",
    given: "",
    additional: "",
    surname: "",
    suffix: "",
    fullName: "",
    nickname: "",
    organization: "",
    jobTitle: "",
    emails: [blankRow()],
    phones: [blankRow()],
    address: blankAddress(),
    birthday: "",
    note: "",
    photo: null,
  };
}

export function contactToForm(card: ContactCard): ContactFormValues {
  const name = card.name ?? {};
  const address = card.addresses?.[0];
  return {
    addressBookId: card.addressBookId,
    prefix: text(name.prefix),
    given: text(name.given),
    additional: text(name.additional),
    surname: text(name.surname),
    suffix: text(name.suffix),
    fullName: text(card.fullName),
    nickname: text(card.nickname),
    organization: text(card.organization),
    jobTitle: text(card.jobTitle),
    emails: toRows(card.emails),
    phones: toRows(card.phones),
    address: address
      ? {
          street: text(address.street),
          city: text(address.city),
          region: text(address.region),
          postcode: text(address.postcode),
          country: text(address.country),
          label: address.label ?? "other",
        }
      : blankAddress(),
    birthday: text(card.birthday),
    note: text(card.note),
    photo: card.photo ?? null,
  };
}

export function composedFullName(values: ContactFormValues): string {
  return [values.prefix, values.given, values.additional, values.surname, values.suffix]
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
    .join(" ");
}

function addressPayload(row: AddressRow): ContactAddress[] {
  const street = orNull(row.street);
  const city = orNull(row.city);
  const region = orNull(row.region);
  const postcode = orNull(row.postcode);
  const country = orNull(row.country);
  if (!street && !city && !region && !postcode && !country) return [];
  return [{ street, city, region, postcode, country, label: normalizeLabel(row.label) }];
}

export function formToPayload(values: ContactFormValues): ContactPayload {
  const emails = toPayloadRows(values.emails);
  const fullName = values.fullName.trim() || composedFullName(values) || emails[0]?.value || "";
  return {
    addressBookId: values.addressBookId,
    kind: "individual",
    name: {
      prefix: orNull(values.prefix),
      given: orNull(values.given),
      additional: orNull(values.additional),
      surname: orNull(values.surname),
      suffix: orNull(values.suffix),
    },
    fullName,
    nickname: orNull(values.nickname),
    emails,
    phones: toPayloadRows(values.phones),
    organization: orNull(values.organization),
    jobTitle: orNull(values.jobTitle),
    addresses: addressPayload(values.address),
    birthday: orNull(values.birthday),
    note: orNull(values.note),
    members: [],
    photo: values.photo,
  };
}

export function validateContactForm(values: ContactFormValues): string | null {
  if (!values.addressBookId.trim()) return "Pick an address book";

  const named = Boolean(values.fullName.trim() || composedFullName(values));
  const anyEmail = values.emails.some((row) => row.value.trim().length > 0);
  if (!named && !anyEmail) return "Add a name or an email address";

  for (const row of values.emails) {
    const value = row.value.trim();
    if (value.length > 0 && !EMAIL_PATTERN.test(value)) return `${value} is not a valid email address`;
  }

  const birthday = values.birthday.trim();
  if (birthday.length > 0 && !BIRTHDAY_PATTERN.test(birthday)) return "Enter the birthday as YYYY-MM-DD";

  return null;
}
