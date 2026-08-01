import { describe, expect, it } from "vitest";

import {
  composedFullName,
  contactToForm,
  emptyContactForm,
  formToPayload,
  validateContactForm,
  type ContactFormValues,
} from "./contact-form-mapping";
import type { ContactCard } from "./types";

const filled: ContactCard = {
  id: "c1",
  addressBookId: "book-1",
  uid: "uid-1",
  kind: "individual",
  name: { prefix: "Dr", given: "Ada", additional: "Byron", surname: "Lovelace", suffix: "PhD" },
  fullName: "Ada Lovelace",
  nickname: "Ada",
  emails: [
    { value: "ada@example.com", label: "work" },
    { value: "ada@home.example", label: null },
  ],
  phones: [{ value: "+44 20 7946 0000", label: "home" }],
  organization: "Analytical Engines",
  jobTitle: "Mathematician",
  addresses: [
    {
      street: "12 Engine Row",
      city: "London",
      region: "Greater London",
      postcode: "NW1 4RY",
      country: "UK",
      label: "work",
    },
    { street: "Second", city: null, region: null, postcode: null, country: null, label: "home" },
  ],
  birthday: "1815-12-10",
  note: "Wrote the first program",
  members: [],
  photo: { mediaType: "image/jpeg", data: "AAAA" },
};

function form(patch: Partial<ContactFormValues> = {}): ContactFormValues {
  return { ...emptyContactForm("book-1"), ...patch };
}

describe("emptyContactForm", () => {
  it("keeps the address book and blanks every string", () => {
    const values = emptyContactForm("book-9");
    expect(values.addressBookId).toBe("book-9");
    expect(values.given).toBe("");
    expect(values.fullName).toBe("");
    expect(values.note).toBe("");
    expect(values.photo).toBeNull();
  });

  it("seeds one blank email row and one blank phone row labelled home", () => {
    const values = emptyContactForm("book-1");
    expect(values.emails).toEqual([{ value: "", label: "home" }]);
    expect(values.phones).toEqual([{ value: "", label: "home" }]);
  });

  it("seeds a blank address labelled home", () => {
    expect(emptyContactForm("book-1").address).toEqual({
      street: "",
      city: "",
      region: "",
      postcode: "",
      country: "",
      label: "home",
    });
  });
});

describe("contactToForm", () => {
  it("maps every populated field", () => {
    const values = contactToForm(filled);
    expect(values).toMatchObject({
      addressBookId: "book-1",
      prefix: "Dr",
      given: "Ada",
      additional: "Byron",
      surname: "Lovelace",
      suffix: "PhD",
      fullName: "Ada Lovelace",
      nickname: "Ada",
      organization: "Analytical Engines",
      jobTitle: "Mathematician",
      birthday: "1815-12-10",
      note: "Wrote the first program",
      photo: { mediaType: "image/jpeg", data: "AAAA" },
    });
  });

  it("turns a missing card body into blanks", () => {
    const values = contactToForm({ id: "c2", addressBookId: "book-2" });
    expect(values).toMatchObject({
      addressBookId: "book-2",
      given: "",
      surname: "",
      fullName: "",
      nickname: "",
      organization: "",
      jobTitle: "",
      birthday: "",
      note: "",
      photo: null,
    });
    expect(values.address.street).toBe("");
  });

  it("seeds a blank row when the arrays are null", () => {
    const values = contactToForm({ id: "c3", addressBookId: "b", emails: null, phones: null });
    expect(values.emails).toEqual([{ value: "", label: "home" }]);
    expect(values.phones).toEqual([{ value: "", label: "home" }]);
  });

  it("seeds a blank row when the arrays are empty", () => {
    const values = contactToForm({ id: "c4", addressBookId: "b", emails: [], phones: [] });
    expect(values.emails).toEqual([{ value: "", label: "home" }]);
    expect(values.phones).toEqual([{ value: "", label: "home" }]);
  });

  it("normalises a null label to other", () => {
    const values = contactToForm(filled);
    expect(values.emails).toEqual([
      { value: "ada@example.com", label: "work" },
      { value: "ada@home.example", label: "other" },
    ]);
  });

  it("takes the first address only", () => {
    expect(contactToForm(filled).address).toEqual({
      street: "12 Engine Row",
      city: "London",
      region: "Greater London",
      postcode: "NW1 4RY",
      country: "UK",
      label: "work",
    });
  });

  it("blanks null address components", () => {
    const values = contactToForm({
      id: "c5",
      addressBookId: "b",
      addresses: [
        { street: "Main", city: null, region: null, postcode: null, country: null, label: null },
      ],
    });
    expect(values.address).toEqual({
      street: "Main",
      city: "",
      region: "",
      postcode: "",
      country: "",
      label: "other",
    });
  });
});

describe("composedFullName", () => {
  it("joins the five name parts in order", () => {
    expect(
      composedFullName(
        form({ prefix: "Dr", given: "Ada", additional: "B", surname: "Lovelace", suffix: "PhD" }),
      ),
    ).toBe("Dr Ada B Lovelace PhD");
  });

  it("skips empty and whitespace-only parts", () => {
    expect(composedFullName(form({ given: "  Ada  ", additional: "   ", surname: "Lovelace" }))).toBe(
      "Ada Lovelace",
    );
  });

  it("returns an empty string when no part is set", () => {
    expect(composedFullName(form())).toBe("");
  });
});

describe("formToPayload", () => {
  it("trims values and drops blank rows", () => {
    const payload = formToPayload(
      form({
        given: " Ada ",
        surname: " Lovelace ",
        emails: [
          { value: "  ada@example.com ", label: "work" },
          { value: "   ", label: "home" },
        ],
        phones: [{ value: "", label: "home" }],
      }),
    );
    expect(payload.name).toMatchObject({ given: "Ada", surname: "Lovelace" });
    expect(payload.emails).toEqual([{ value: "ada@example.com", label: "work" }]);
    expect(payload.phones).toEqual([]);
  });

  it("nulls a label that is not home, work or other", () => {
    const payload = formToPayload(
      form({
        emails: [{ value: "a@b.co", label: "mobile" }],
        phones: [{ value: "123", label: "" }],
      }),
    );
    expect(payload.emails[0]?.label).toBeNull();
    expect(payload.phones[0]?.label).toBeNull();
  });

  it("nulls empty optional strings", () => {
    const payload = formToPayload(form({ given: "Ada" }));
    expect(payload.nickname).toBeNull();
    expect(payload.organization).toBeNull();
    expect(payload.jobTitle).toBeNull();
    expect(payload.birthday).toBeNull();
    expect(payload.note).toBeNull();
    expect(payload.name.prefix).toBeNull();
  });

  it("drops a blank address", () => {
    expect(formToPayload(form({ given: "Ada" })).addresses).toEqual([]);
  });

  it("keeps a partly filled address with null components", () => {
    const payload = formToPayload(
      form({
        given: "Ada",
        address: {
          street: " 12 Engine Row ",
          city: "",
          region: "  ",
          postcode: "",
          country: "UK",
          label: "work",
        },
      }),
    );
    expect(payload.addresses).toEqual([
      {
        street: "12 Engine Row",
        city: null,
        region: null,
        postcode: null,
        country: "UK",
        label: "work",
      },
    ]);
  });

  it("uses the typed display name when it is set", () => {
    expect(formToPayload(form({ fullName: " Ada L. ", given: "Ada" })).fullName).toBe("Ada L.");
  });

  it("falls back to the composed name", () => {
    expect(formToPayload(form({ given: "Ada", surname: "Lovelace" })).fullName).toBe("Ada Lovelace");
  });

  it("falls back to the first non-empty email", () => {
    expect(
      formToPayload(
        form({
          emails: [
            { value: "", label: "home" },
            { value: " ada@example.com ", label: "work" },
          ],
        }),
      ).fullName,
    ).toBe("ada@example.com");
  });

  it("leaves the full name empty when nothing is available", () => {
    expect(formToPayload(form()).fullName).toBe("");
  });

  it("always sends an individual with no members and copies the photo", () => {
    const photo = { mediaType: "image/png", data: "BBBB" };
    const payload = formToPayload(form({ given: "Ada", photo }));
    expect(payload.kind).toBe("individual");
    expect(payload.members).toEqual([]);
    expect(payload.photo).toBe(photo);
  });
});

describe("validateContactForm", () => {
  it("requires an address book first", () => {
    expect(validateContactForm(form({ addressBookId: "" }))).toBe("Pick an address book");
  });

  it("requires a name or an email address", () => {
    expect(validateContactForm(form())).toBe("Add a name or an email address");
  });

  it("accepts a form carrying only an email address", () => {
    expect(validateContactForm(form({ emails: [{ value: "ada@example.com", label: "home" }] }))).toBeNull();
  });

  it("accepts a form carrying only a display name", () => {
    expect(validateContactForm(form({ fullName: "Ada" }))).toBeNull();
  });

  it("rejects the first malformed email address", () => {
    expect(
      validateContactForm(
        form({
          given: "Ada",
          emails: [
            { value: "ada@example.com", label: "home" },
            { value: "nope", label: "work" },
          ],
        }),
      ),
    ).toBe("nope is not a valid email address");
  });

  it("rejects a birthday that is not YYYY-MM-DD", () => {
    expect(validateContactForm(form({ given: "Ada", birthday: "10/12/1815" }))).toBe(
      "Enter the birthday as YYYY-MM-DD",
    );
  });

  it("accepts a well formed birthday", () => {
    expect(validateContactForm(form({ given: "Ada", birthday: "1815-12-10" }))).toBeNull();
  });
});
