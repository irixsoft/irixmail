export interface AddressBook {
  id: string;
  name: string;
  description?: string | null;
  isDefault?: boolean;
}

export type ContactKind = "individual" | "group";

export interface ContactName {
  prefix: string | null;
  given: string | null;
  additional: string | null;
  surname: string | null;
  suffix: string | null;
}

export interface LabeledValue {
  value: string;
  label: string | null;
}

export interface ContactAddress {
  street: string | null;
  city: string | null;
  region: string | null;
  postcode: string | null;
  country: string | null;
  label: string | null;
}

export interface ContactPhoto {
  mediaType: string;
  data: string;
}

export interface ContactCard {
  id: string;
  addressBookId: string;
  uid?: string | null;
  kind?: ContactKind | null;
  name?: Partial<ContactName> | null;
  fullName?: string | null;
  nickname?: string | null;
  emails?: LabeledValue[] | null;
  phones?: LabeledValue[] | null;
  organization?: string | null;
  jobTitle?: string | null;
  addresses?: ContactAddress[] | null;
  birthday?: string | null;
  note?: string | null;
  members?: string[] | null;
  photo?: ContactPhoto | null;
  etag?: string | null;
  created?: string | null;
  updated?: string | null;
}

export interface ContactPayload {
  addressBookId: string;
  kind: ContactKind;
  name: ContactName;
  fullName: string;
  nickname: string | null;
  emails: LabeledValue[];
  phones: LabeledValue[];
  organization: string | null;
  jobTitle: string | null;
  addresses: ContactAddress[];
  birthday: string | null;
  note: string | null;
  members: string[];
  photo: ContactPhoto | null;
}

export const emptyName: ContactName = {
  prefix: null,
  given: null,
  additional: null,
  surname: null,
  suffix: null,
};

export const CONTACT_LABELS = ["home", "work", "other"] as const;
