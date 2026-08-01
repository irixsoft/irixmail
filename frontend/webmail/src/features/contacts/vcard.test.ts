import { describe, expect, it } from "vitest";
import { emptyName, type ContactCard } from "./types";
import {
  cardToParsed,
  generateVcf,
  parsedToPayload,
  parseVcf,
  type ParsedCard,
} from "./vcard";

const octets = (s: string) => new TextEncoder().encode(s).length;

const wrap = (...lines: string[]) =>
  ["BEGIN:VCARD", "VERSION:3.0", ...lines, "END:VCARD"].join("\r\n");

const first = (text: string): ParsedCard => {
  const [parsed] = parseVcf(text);
  if (!parsed) throw new Error("expected at least one card");
  return parsed;
};

const one = (...lines: string[]) => first(wrap(...lines));

const card = (over: Partial<ParsedCard> = {}): ParsedCard => ({
  uid: null,
  name: { ...emptyName },
  fullName: "",
  nickname: null,
  emails: [],
  phones: [],
  organization: null,
  jobTitle: null,
  addresses: [],
  birthday: null,
  note: null,
  photo: null,
  ...over,
});

const propNames = (vcf: string) =>
  vcf
    .split("\r\n")
    .filter((l) => l.length > 0 && !l.startsWith(" "))
    .map((l) => l.split(/[;:]/)[0]);

describe("parseVcf unfolding", () => {
  it("joins a continuation line starting with a space", () => {
    expect(one("FN:Ada Love", " lace").fullName).toBe("Ada Lovelace");
  });

  it("joins a continuation line starting with a tab", () => {
    expect(one("FN:Ada Love", "\tlace").fullName).toBe("Ada Lovelace");
  });

  it("strips only the first whitespace char of a continuation", () => {
    expect(one("FN:Ada", "  Lovelace").fullName).toBe("Ada Lovelace");
  });

  it("unfolds text using bare LF line endings", () => {
    const text = "BEGIN:VCARD\nVERSION:3.0\nFN:Ada Love\n lace\nEND:VCARD\n";
    expect(first(text).fullName).toBe("Ada Lovelace");
  });
});

describe("parseVcf structure", () => {
  it("returns one entry per vcard block", () => {
    const text = `${wrap("FN:Ada Lovelace")}\r\n${wrap("FN:Alan Turing")}\r\n`;
    expect(parseVcf(text).map((c) => c.fullName)).toEqual([
      "Ada Lovelace",
      "Alan Turing",
    ]);
  });

  it("ignores content outside vcard blocks", () => {
    const text = `junk line\r\n${wrap("FN:Ada Lovelace")}\r\ntrailing junk\r\n`;
    expect(parseVcf(text)).toHaveLength(1);
  });

  it("skips a card with no recognisable content", () => {
    const text = `${wrap("NOTE:orphan")}\r\n${wrap("FN:Ada Lovelace")}\r\n`;
    expect(parseVcf(text).map((c) => c.fullName)).toEqual(["Ada Lovelace"]);
  });

  it("keeps a card that only has an email", () => {
    expect(parseVcf(wrap("EMAIL:ada@example.com"))).toHaveLength(1);
  });

  it("parses a vcard 3.0 sample", () => {
    const parsed = one(
      "N:Lovelace;Ada;Byron;Ms.;PhD",
      "FN:Ada Lovelace",
      "ORG:Analytical Engines",
      "TITLE:Mathematician",
      "EMAIL;TYPE=INTERNET;TYPE=WORK:ada@example.com",
      "TEL;TYPE=CELL;TYPE=HOME:+15550001",
    );
    expect(parsed.name).toEqual({
      prefix: "Ms.",
      given: "Ada",
      additional: "Byron",
      surname: "Lovelace",
      suffix: "PhD",
    });
    expect(parsed.organization).toBe("Analytical Engines");
    expect(parsed.jobTitle).toBe("Mathematician");
    expect(parsed.emails).toEqual([{ value: "ada@example.com", label: "work" }]);
    expect(parsed.phones).toEqual([{ value: "+15550001", label: "home" }]);
  });

  it("parses a vcard 4.0 sample", () => {
    const text = [
      "BEGIN:VCARD",
      "VERSION:4.0",
      "UID:urn:uuid:9a1b-2c3d",
      "FN:Alan Turing",
      "N:Turing;Alan;Mathison;;",
      "EMAIL;TYPE=work:alan@example.com",
      "TEL;VALUE=uri;TYPE=\"voice,home\":tel:+15550002",
      "END:VCARD",
    ].join("\r\n");
    const parsed = first(text);
    expect(parsed.uid).toBe("9a1b-2c3d");
    expect(parsed.name.additional).toBe("Mathison");
    expect(parsed.phones).toEqual([{ value: "tel:+15550002", label: "home" }]);
  });

  it("drops a group prefix from a property name", () => {
    expect(one("item1.EMAIL;TYPE=work:ada@example.com").emails).toEqual([
      { value: "ada@example.com", label: "work" },
    ]);
  });

  it("ignores unknown properties", () => {
    const parsed = one("FN:Ada Lovelace", "X-SOCIALPROFILE;TYPE=x:whatever");
    expect(parsed.fullName).toBe("Ada Lovelace");
    expect(parsed.emails).toEqual([]);
  });

  it("skips empty email values", () => {
    expect(one("FN:Ada Lovelace", "EMAIL;TYPE=work:").emails).toEqual([]);
  });
});

describe("parseVcf parameters", () => {
  it("treats bare params as type values", () => {
    expect(one("FN:Ada", "TEL;HOME;VOICE:+15550001").phones).toEqual([
      { value: "+15550001", label: "home" },
    ]);
  });

  it("splits a quoted comma separated type list", () => {
    expect(one("FN:Ada", "TEL;TYPE=\"voice,work\":+15550001").phones).toEqual([
      { value: "+15550001", label: "work" },
    ]);
  });

  it("does not end the parameter section at a colon inside a quoted value", () => {
    expect(
      one("FN:Ada", "EMAIL;X-ABLabel=\"a:b\";TYPE=work:ada@example.com").emails,
    ).toEqual([{ value: "ada@example.com", label: "work" }]);
  });

  it("matches param and property names case insensitively", () => {
    expect(one("fn:Ada", "email;type=WORK:ada@example.com").emails).toEqual([
      { value: "ada@example.com", label: "work" },
    ]);
  });

  it("falls back to a null label when no home work or other type is present", () => {
    expect(one("FN:Ada", "TEL;TYPE=CELL;TYPE=PREF:+15550001").phones).toEqual([
      { value: "+15550001", label: null },
    ]);
  });
});

describe("parseVcf value unescaping", () => {
  it("unescapes newlines commas semicolons and backslashes", () => {
    const parsed = one("FN:Ada", "NOTE:one\\ntwo\\, three\\; four \\\\ five");
    expect(parsed.note).toBe("one\ntwo, three; four \\ five");
  });

  it("unescapes an uppercase newline escape", () => {
    expect(one("FN:Ada", "NOTE:one\\Ntwo").note).toBe("one\ntwo");
  });

  it("treats an escaped backslash before n as a backslash and an n", () => {
    expect(one("FN:Ada", "NOTE:a\\\\nb").note).toBe("a\\nb");
  });

  it("unescapes each structured name component separately", () => {
    expect(one("N:Love\\;lace;Ada;;;").name.surname).toBe("Love;lace");
  });

  it("does not split a name on an escaped semicolon", () => {
    expect(one("N:Love\\;lace;Ada;;;").name.given).toBe("Ada");
  });
});

describe("parseVcf name and org", () => {
  it("fills missing name components with null", () => {
    expect(one("N:Turing;Alan").name).toEqual({
      prefix: null,
      given: "Alan",
      additional: null,
      surname: "Turing",
      suffix: null,
    });
  });

  it("joins org components with a space", () => {
    expect(one("FN:Ada", "ORG:Acme Inc.;R&D;Team").organization).toBe(
      "Acme Inc. R&D Team",
    );
  });

  it("maps an empty org to null", () => {
    expect(one("FN:Ada", "ORG:;;").organization).toBeNull();
  });

  it("keeps a comma separated nickname as a raw string", () => {
    expect(one("FN:Ada", "NICKNAME:Addy\\,Countess").nickname).toBe(
      "Addy,Countess",
    );
  });
});

describe("parseVcf addresses", () => {
  it("maps adr components", () => {
    expect(
      one("FN:Ada", "ADR;TYPE=work:;;123 Main St;Springfield;IL;62704;USA")
        .addresses,
    ).toEqual([
      {
        street: "123 Main St",
        city: "Springfield",
        region: "IL",
        postcode: "62704",
        country: "USA",
        label: "work",
      },
    ]);
  });

  it("prepends a non-empty pobox and extended address to the street", () => {
    expect(
      one("FN:Ada", "ADR:PO Box 7;Suite 5;123 Main St;Springfield;;;")
        .addresses[0]?.street,
    ).toBe("PO Box 7 Suite 5 123 Main St");
  });

  it("skips an adr whose mapped components are all empty", () => {
    expect(one("FN:Ada", "ADR;TYPE=home:;;;;;;").addresses).toEqual([]);
  });
});

describe("parseVcf birthday", () => {
  it("normalises a basic date", () => {
    expect(one("FN:Ada", "BDAY:19851203").birthday).toBe("1985-12-03");
  });

  it("keeps an extended date", () => {
    expect(one("FN:Ada", "BDAY:1985-12-03").birthday).toBe("1985-12-03");
  });

  it("takes the date part of a timestamp", () => {
    expect(one("FN:Ada", "BDAY:1985-12-03T00:00:00Z").birthday).toBe(
      "1985-12-03",
    );
  });

  it("returns null for a year-less date", () => {
    expect(one("FN:Ada", "BDAY:--1203").birthday).toBeNull();
  });

  it("returns null for an unparseable date", () => {
    expect(one("FN:Ada", "BDAY:sometime").birthday).toBeNull();
  });
});

describe("parseVcf uid and photo", () => {
  it("strips a urn uuid prefix from uid", () => {
    expect(one("FN:Ada", "UID:urn:uuid:abc-123").uid).toBe("abc-123");
  });

  it("keeps a plain uid", () => {
    expect(one("FN:Ada", "UID:abc-123").uid).toBe("abc-123");
  });

  it("reads a 3.0 base64 photo", () => {
    expect(one("FN:Ada", "PHOTO;ENCODING=b;TYPE=JPEG:QUJD").photo).toEqual({
      mediaType: "image/jpeg",
      data: "QUJD",
    });
  });

  it("reads a legacy BASE64 encoding param", () => {
    expect(one("FN:Ada", "PHOTO;ENCODING=BASE64;TYPE=PNG:QUJD").photo).toEqual({
      mediaType: "image/png",
      data: "QUJD",
    });
  });

  it("reads a data uri photo", () => {
    expect(one("FN:Ada", "PHOTO:data:image/png;base64,QUJD").photo).toEqual({
      mediaType: "image/png",
      data: "QUJD",
    });
  });

  it("strips whitespace from folded photo data", () => {
    expect(one("FN:Ada", "PHOTO;ENCODING=b;TYPE=JPEG:QU", "  JD").photo).toEqual(
      { mediaType: "image/jpeg", data: "QUJD" },
    );
  });

  it("ignores a photo holding an http url", () => {
    expect(
      one("FN:Ada", "PHOTO;VALUE=URI:https://example.com/a.jpg").photo,
    ).toBeNull();
  });
});

describe("parseVcf fullName fallback", () => {
  it("uses the first non-empty FN", () => {
    expect(one("FN:", "FN:Ada Lovelace").fullName).toBe("Ada Lovelace");
  });

  it("builds a fullName from name parts when FN is absent", () => {
    expect(one("N:Lovelace;Ada;Byron;Ms.;PhD").fullName).toBe(
      "Ms. Ada Byron Lovelace PhD",
    );
  });

  it("falls back to the first email when FN and name are absent", () => {
    expect(one("EMAIL:ada@example.com").fullName).toBe("ada@example.com");
  });

  it("falls back to an empty string when only a phone is present", () => {
    expect(one("TEL:+15550001").fullName).toBe("");
  });
});

describe("generateVcf", () => {
  it("emits properties in the documented order", () => {
    const vcf = generateVcf([
      card({
        uid: "abc",
        name: { ...emptyName, given: "Ada", surname: "Lovelace" },
        fullName: "Ada Lovelace",
        nickname: "Addy",
        organization: "Acme",
        jobTitle: "Engineer",
        emails: [{ value: "ada@example.com", label: "work" }],
        phones: [{ value: "+15550001", label: "home" }],
        addresses: [
          {
            street: "1 Main St",
            city: "Springfield",
            region: null,
            postcode: null,
            country: null,
            label: "home",
          },
        ],
        birthday: "1985-12-03",
        note: "hello",
        photo: { mediaType: "image/jpeg", data: "QUJD" },
      }),
    ]);
    expect(propNames(vcf)).toEqual([
      "BEGIN",
      "VERSION",
      "UID",
      "FN",
      "N",
      "NICKNAME",
      "ORG",
      "TITLE",
      "EMAIL",
      "TEL",
      "ADR",
      "BDAY",
      "NOTE",
      "PHOTO",
      "END",
    ]);
  });

  it("joins lines with CRLF and ends with a trailing CRLF", () => {
    const vcf = generateVcf([card({ fullName: "Ada" })]);
    expect(vcf.startsWith("BEGIN:VCARD\r\nVERSION:3.0\r\n")).toBe(true);
    expect(vcf.endsWith("END:VCARD\r\n")).toBe(true);
  });

  it("always emits N with empty components for nulls", () => {
    const vcf = generateVcf([
      card({ fullName: "Ada", name: { ...emptyName, given: "Ada" } }),
    ]);
    expect(vcf).toContain("N:;Ada;;;\r\n");
  });

  it("escapes backslashes newlines commas and semicolons", () => {
    const vcf = generateVcf([card({ fullName: "Ada", note: "a\\b\nc,d;e" })]);
    expect(vcf).toContain("NOTE:a\\\\b\\nc\\,d\\;e");
  });

  it("escapes structured components individually", () => {
    const vcf = generateVcf([
      card({
        fullName: "Ada",
        name: { ...emptyName, surname: "Love;lace", given: "Ada" },
      }),
    ]);
    expect(vcf).toContain("N:Love\\;lace;Ada;;;");
  });

  it("emits adr with empty pobox and extended components", () => {
    const vcf = generateVcf([
      card({
        fullName: "Ada",
        addresses: [
          {
            street: "1 Main St",
            city: "Springfield",
            region: "IL",
            postcode: "62704",
            country: "USA",
            label: null,
          },
        ],
      }),
    ]);
    expect(vcf).toContain("ADR:;;1 Main St;Springfield;IL;62704;USA");
  });

  it("emits a lowercase type param only when a label is present", () => {
    const vcf = generateVcf([
      card({
        fullName: "Ada",
        emails: [
          { value: "ada@example.com", label: "work" },
          { value: "ada@home.example", label: null },
        ],
      }),
    ]);
    expect(vcf).toContain("EMAIL;TYPE=work:ada@example.com");
    expect(vcf).toContain("EMAIL:ada@home.example");
  });

  it("emits photo with an uppercased subtype token", () => {
    const vcf = generateVcf([
      card({ fullName: "Ada", photo: { mediaType: "image/png", data: "QUJD" } }),
    ]);
    expect(vcf).toContain("PHOTO;ENCODING=b;TYPE=PNG:QUJD");
  });

  it("does not fold a line of exactly 75 octets", () => {
    const value = "a".repeat(72);
    const vcf = generateVcf([card({ fullName: value })]);
    const line = vcf.split("\r\n").find((l) => l.startsWith("FN:"));
    expect(octets(line ?? "")).toBe(75);
    expect(vcf).toContain(`FN:${value}\r\n`);
  });

  it("folds a line of 76 octets onto a continuation line", () => {
    const value = "a".repeat(73);
    const lines = generateVcf([card({ fullName: value })]).split("\r\n");
    const start = lines.findIndex((l) => l.startsWith("FN:"));
    expect(octets(lines[start] ?? "")).toBe(75);
    expect(lines[start + 1]).toBe(" a");
  });

  it("keeps every physical line within 75 octets", () => {
    const vcf = generateVcf([card({ fullName: "Ada", note: "x".repeat(400) })]);
    for (const line of vcf.split("\r\n")) {
      expect(octets(line)).toBeLessThanOrEqual(75);
    }
  });

  it("never splits a multi-byte utf-8 sequence across lines", () => {
    const vcf = generateVcf([
      card({ fullName: "Ada", note: `${"é".repeat(60)}🎉${"漢".repeat(40)}` }),
    ]);
    for (const line of vcf.split("\r\n")) {
      expect(octets(line)).toBeLessThanOrEqual(75);
      expect(line).not.toContain("�");
    }
  });

  it("prefixes continuation lines with a single space", () => {
    const vcf = generateVcf([card({ fullName: "Ada", note: "y".repeat(200) })]);
    const continuations = vcf
      .split("\r\n")
      .filter((l) => l.startsWith(" "))
      .map((l) => l.slice(1));
    expect(continuations.length).toBeGreaterThan(1);
    expect(continuations.every((l) => !l.startsWith(" "))).toBe(true);
  });
});

describe("round trip", () => {
  it("round trips a full card including a photo and a long note", () => {
    const source = card({
      uid: "abc-123",
      name: {
        prefix: "Ms.",
        given: "Ada",
        additional: "Byron",
        surname: "Lovelace",
        suffix: "PhD",
      },
      fullName: "Ada Lovelace",
      nickname: "Addy",
      organization: "Analytical Engines",
      jobTitle: "Mathematician",
      emails: [
        { value: "ada@example.com", label: "work" },
        { value: "ada@home.example", label: null },
      ],
      phones: [{ value: "+15550001", label: "home" }],
      addresses: [
        {
          street: "123 Main St",
          city: "Springfield",
          region: "IL",
          postcode: "62704",
          country: "USA",
          label: "home",
        },
      ],
      birthday: "1985-12-03",
      note: `${"long note ".repeat(30)}\nwith, punctuation; and \\ slashes`,
      photo: { mediaType: "image/jpeg", data: "QUJDRA".repeat(50) },
    });
    expect(parseVcf(generateVcf([source]))).toEqual([source]);
  });

  it("round trips non-ascii values", () => {
    const source = card({
      name: { ...emptyName, given: "Zoé", surname: "Müller" },
      fullName: "Zoé Müller",
      note: `${"漢字テスト ".repeat(20)}🎉`,
      emails: [],
      phones: [],
    });
    expect(parseVcf(generateVcf([source]))).toEqual([source]);
  });

  it("round trips multiple cards", () => {
    const cards = [
      card({ fullName: "Ada", name: { ...emptyName, given: "Ada" } }),
      card({ fullName: "Alan", name: { ...emptyName, given: "Alan" } }),
    ];
    expect(parseVcf(generateVcf(cards))).toEqual(cards);
  });
});

describe("cardToParsed", () => {
  it("fills missing name keys and arrays", () => {
    const source: ContactCard = {
      id: "1",
      addressBookId: "ab",
      name: { given: "Ada" },
      fullName: "Ada Lovelace",
    };
    expect(cardToParsed(source)).toEqual(
      card({
        name: { ...emptyName, given: "Ada" },
        fullName: "Ada Lovelace",
      }),
    );
  });

  it("falls back to name parts for fullName", () => {
    const source: ContactCard = {
      id: "1",
      addressBookId: "ab",
      name: { given: "Ada", surname: "Lovelace" },
    };
    expect(cardToParsed(source).fullName).toBe("Ada Lovelace");
  });

  it("falls back to the first email for fullName", () => {
    const source: ContactCard = {
      id: "1",
      addressBookId: "ab",
      emails: [{ value: "ada@example.com", label: null }],
    };
    expect(cardToParsed(source).fullName).toBe("ada@example.com");
  });

  it("copies uid photo and scalar fields", () => {
    const source: ContactCard = {
      id: "1",
      addressBookId: "ab",
      uid: "u1",
      fullName: "Ada",
      nickname: "Addy",
      organization: "Acme",
      jobTitle: "Engineer",
      birthday: "1985-12-03",
      note: "hi",
      photo: { mediaType: "image/png", data: "QUJD" },
    };
    const parsed = cardToParsed(source);
    expect(parsed.uid).toBe("u1");
    expect(parsed.photo).toEqual({ mediaType: "image/png", data: "QUJD" });
    expect(parsed.organization).toBe("Acme");
  });
});

describe("parsedToPayload", () => {
  it("returns an individual payload with empty members", () => {
    const source = card({
      fullName: "Ada",
      emails: [{ value: "ada@example.com", label: "work" }],
    });
    expect(parsedToPayload(source, "ab-1")).toEqual({
      addressBookId: "ab-1",
      kind: "individual",
      name: source.name,
      fullName: "Ada",
      nickname: null,
      emails: [{ value: "ada@example.com", label: "work" }],
      phones: [],
      organization: null,
      jobTitle: null,
      addresses: [],
      birthday: null,
      note: null,
      members: [],
      photo: null,
    });
  });
});
