import {
  emptyName,
  type ContactAddress,
  type ContactCard,
  type ContactName,
  type ContactPayload,
  type ContactPhoto,
  type LabeledValue,
} from "./types";

export interface ParsedCard {
  uid: string | null;
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
  photo: ContactPhoto | null;
}

interface Prop {
  name: string;
  params: Map<string, string[]>;
  types: string[];
  value: string;
}

const LABELS = ["home", "work", "other"];
const MAX_OCTETS = 75;
const encoder = new TextEncoder();
const isText = (v: string | null | undefined): v is string => !!v;

function unfold(text: string): string[] {
  const out: string[] = [];
  for (const line of text.split(/\r\n|\n|\r/)) {
    if (out.length > 0 && /^[ \t]/.test(line)) out[out.length - 1] += line.slice(1);
    else out.push(line);
  }
  return out;
}

function split(input: string, sep: string, mode: "quote" | "escape"): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  for (let i = 0; i < input.length; i += 1) {
    const ch = input.charAt(i);
    if (mode === "escape" && ch === "\\" && i + 1 < input.length) {
      cur += ch + input.charAt(i + 1);
      i += 1;
    } else if (mode === "quote" && ch === '"') {
      quoted = !quoted;
      cur += ch;
    } else if (ch === sep && !quoted) {
      out.push(cur);
      cur = "";
    } else cur += ch;
  }
  out.push(cur);
  return out;
}

const unquote = (input: string): string =>
  input.trim().replace(/^"+/, "").replace(/"+$/, "");

function unescapeValue(input: string): string {
  let out = "";
  for (let i = 0; i < input.length; i += 1) {
    const ch = input.charAt(i);
    if (ch !== "\\") {
      out += ch;
      continue;
    }
    if (i + 1 >= input.length) return `${out}\\`;
    const next = input.charAt(i + 1);
    i += 1;
    out += next === "n" || next === "N" ? "\n" : next;
  }
  return out;
}

const component = (parts: string[], index: number): string | null =>
  unescapeValue(parts[index] ?? "").trim() || null;

const labelOf = (types: string[]): string | null =>
  types.find((t) => LABELS.includes(t)) ?? null;

function parseLine(line: string): Prop | null {
  let quoted = false;
  let colon = -1;
  for (let i = 0; i < line.length && colon < 0; i += 1) {
    const ch = line.charAt(i);
    if (ch === '"') quoted = !quoted;
    else if (ch === ":" && !quoted) colon = i;
  }
  if (colon < 0) return null;

  const segments = split(line.slice(0, colon), ";", "quote");
  const raw = segments[0] ?? "";
  const name = raw.slice(raw.indexOf(".") + 1).trim().toUpperCase();
  if (!name) return null;

  const params = new Map<string, string[]>();
  const types: string[] = [];
  for (const segment of segments.slice(1)) {
    const eq = segment.indexOf("=");
    if (eq < 0) {
      const bare = unquote(segment).toLowerCase();
      if (bare) types.push(bare);
      continue;
    }
    const key = segment.slice(0, eq).trim().toUpperCase();
    const values = segment
      .slice(eq + 1)
      .split(",")
      .map(unquote)
      .filter((v) => v.length > 0);
    params.set(key, [...(params.get(key) ?? []), ...values]);
    if (key === "TYPE") types.push(...values.map((v) => v.toLowerCase()));
  }
  return { name, params, types, value: line.slice(colon + 1) };
}

function normalizeDate(raw: string): string | null {
  const value = raw.trim();
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(value) ?? /^(\d{4})(\d{2})(\d{2})/.exec(value);
  return match ? `${match[1]}-${match[2]}-${match[3]}` : null;
}

function readPhoto(prop: Prop): ContactPhoto | null {
  const value = prop.value.trim();
  if (/^data:/i.test(value)) {
    const match = /^data:([^;,]*);base64,([\s\S]*)$/i.exec(value);
    const data = (match?.[2] ?? "").replace(/\s+/g, "");
    if (!data) return null;
    return { mediaType: (match?.[1] ?? "").toLowerCase() || "image/jpeg", data };
  }
  if (/^https?:/i.test(value)) return null;
  const encoding = (prop.params.get("ENCODING") ?? [])[0]?.toLowerCase();
  if (encoding !== "b" && encoding !== "base64") return null;
  const data = value.replace(/\s+/g, "");
  if (!data) return null;
  const declared = (prop.params.get("MEDIATYPE") ?? prop.params.get("TYPE") ?? [])[0];
  const media = declared ? declared.toLowerCase() : "image/jpeg";
  return { mediaType: media.includes("/") ? media : `image/${media}`, data };
}

function buildCard(props: Prop[]): ParsedCard | null {
  const card: ParsedCard = {
    uid: null, name: { ...emptyName }, fullName: "", nickname: null,
    emails: [], phones: [], organization: null, jobTitle: null,
    addresses: [], birthday: null, note: null, photo: null,
  };
  let fullName = "";
  let hasName = false;

  for (const prop of props) {
    const text = prop.name === "PHOTO" ? "" : unescapeValue(prop.value);
    switch (prop.name) {
      case "FN":
        if (!fullName) fullName = text.trim();
        break;
      case "N": {
        const parts = split(prop.value, ";", "escape");
        card.name = {
          prefix: component(parts, 3), given: component(parts, 1),
          additional: component(parts, 2), surname: component(parts, 0),
          suffix: component(parts, 4),
        };
        hasName = Object.values(card.name).some((v) => v !== null);
        break;
      }
      case "EMAIL":
      case "TEL": {
        const value = text.trim();
        if (!value) break;
        const into = prop.name === "EMAIL" ? card.emails : card.phones;
        into.push({ value, label: labelOf(prop.types) });
        break;
      }
      case "ORG":
        card.organization =
          split(prop.value, ";", "escape")
            .map((p) => unescapeValue(p).trim())
            .filter((p) => p.length > 0).join(" ") || null;
        break;
      case "TITLE":
        card.jobTitle = text.trim() || null;
        break;
      case "NICKNAME":
        card.nickname = text || null;
        break;
      case "NOTE":
        card.note = text || null;
        break;
      case "ADR": {
        const parts = split(prop.value, ";", "escape");
        const street =
          [component(parts, 0), component(parts, 1), component(parts, 2)]
            .filter(isText)
            .join(" ") || null;
        const address: ContactAddress = {
          street, city: component(parts, 3), region: component(parts, 4),
          postcode: component(parts, 5), country: component(parts, 6),
          label: labelOf(prop.types),
        };
        const filled =
          street ?? address.city ?? address.region ?? address.postcode ?? address.country;
        if (filled) card.addresses.push(address);
        break;
      }
      case "BDAY":
        card.birthday = normalizeDate(text);
        break;
      case "UID":
        card.uid = text.trim().replace(/^urn:uuid:/i, "") || null;
        break;
      case "PHOTO":
        if (!card.photo) card.photo = readPhoto(prop);
        break;
      default:
        break;
    }
  }

  const n = card.name;
  const joined = [n.prefix, n.given, n.additional, n.surname, n.suffix]
    .filter(isText)
    .join(" ");
  card.fullName = fullName || joined || card.emails[0]?.value || "";
  const keep = !!fullName || hasName || card.emails.length > 0 || card.phones.length > 0;
  return keep ? card : null;
}

export function parseVcf(text: string): ParsedCard[] {
  const cards: ParsedCard[] = [];
  let open: Prop[] | null = null;
  for (const line of unfold(text)) {
    const trimmed = line.trim();
    if (/^BEGIN:VCARD$/i.test(trimmed)) {
      open = [];
    } else if (/^END:VCARD$/i.test(trimmed)) {
      const card = open ? buildCard(open) : null;
      if (card) cards.push(card);
      open = null;
    } else if (open) {
      const prop = parseLine(line);
      if (prop) open.push(prop);
    }
  }
  return cards;
}

const escapeValue = (input: string): string =>
  input.replace(/\r\n?/g, "\n").replace(/\\/g, "\\\\")
    .replace(/\n/g, "\\n").replace(/,/g, "\\,").replace(/;/g, "\\;");

const joinComponents = (values: (string | null)[]): string =>
  values.map((v) => escapeValue(v ?? "")).join(";");

const typeParam = (label: string | null): string =>
  label ? `;TYPE=${label.toLowerCase()}` : "";

function fold(line: string): string[] {
  if (encoder.encode(line).length <= MAX_OCTETS) return [line];
  const out: string[] = [];
  let current = "";
  let bytes = 0;
  for (const ch of line) {
    const width = encoder.encode(ch).length;
    if (bytes + width > MAX_OCTETS) {
      out.push(current);
      current = " ";
      bytes = 1;
    }
    current += ch;
    bytes += width;
  }
  out.push(current);
  return out;
}

function cardLines(card: ParsedCard): string[] {
  const n = card.name;
  const out = ["BEGIN:VCARD", "VERSION:3.0"];
  if (card.uid) out.push(`UID:${escapeValue(card.uid)}`);
  out.push(`FN:${escapeValue(card.fullName)}`);
  out.push(`N:${joinComponents([n.surname, n.given, n.additional, n.prefix, n.suffix])}`);
  if (card.nickname) out.push(`NICKNAME:${escapeValue(card.nickname)}`);
  if (card.organization) out.push(`ORG:${escapeValue(card.organization)}`);
  if (card.jobTitle) out.push(`TITLE:${escapeValue(card.jobTitle)}`);
  for (const e of card.emails) out.push(`EMAIL${typeParam(e.label)}:${escapeValue(e.value)}`);
  for (const p of card.phones) out.push(`TEL${typeParam(p.label)}:${escapeValue(p.value)}`);
  for (const a of card.addresses) {
    const value = joinComponents(["", "", a.street, a.city, a.region, a.postcode, a.country]);
    out.push(`ADR${typeParam(a.label)}:${value}`);
  }
  if (card.birthday) out.push(`BDAY:${card.birthday}`);
  if (card.note) out.push(`NOTE:${escapeValue(card.note)}`);
  if (card.photo) {
    const subtype = (card.photo.mediaType.split("/").pop() ?? "jpeg").toUpperCase();
    out.push(`PHOTO;ENCODING=b;TYPE=${subtype}:${card.photo.data}`);
  }
  out.push("END:VCARD");
  return out;
}

export function generateVcf(cards: ParsedCard[]): string {
  const lines: string[] = [];
  for (const card of cards) {
    for (const logical of cardLines(card)) lines.push(...fold(logical));
  }
  return lines.length > 0 ? `${lines.join("\r\n")}\r\n` : "";
}

export function cardToParsed(card: ContactCard): ParsedCard {
  const src = card.name ?? {};
  const name: ContactName = {
    prefix: src.prefix ?? null, given: src.given ?? null,
    additional: src.additional ?? null, surname: src.surname ?? null,
    suffix: src.suffix ?? null,
  };
  const emails = card.emails ?? [];
  const joined = [name.prefix, name.given, name.additional, name.surname, name.suffix]
    .filter(isText)
    .join(" ");
  return {
    uid: card.uid ?? null, name, emails, phones: card.phones ?? [],
    fullName: (card.fullName ?? "").trim() || joined || emails[0]?.value || "",
    nickname: card.nickname ?? null, organization: card.organization ?? null,
    jobTitle: card.jobTitle ?? null, addresses: card.addresses ?? [],
    birthday: card.birthday ?? null, note: card.note ?? null,
    photo: card.photo ?? null,
  };
}

export function parsedToPayload(card: ParsedCard, addressBookId: string): ContactPayload {
  const { uid, ...rest } = card;
  return { ...rest, addressBookId, kind: "individual", members: [] };
}
