import type { EmailAddress } from "@/lib/mail-types";

export function formatListDate(value?: string | null): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  if (date.toDateString() === now.toDateString()) {
    return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(date);
  }
  const sameYear = date.getFullYear() === now.getFullYear();
  return new Intl.DateTimeFormat(
    undefined,
    sameYear ? { month: "short", day: "numeric" } : { year: "numeric", month: "short", day: "numeric" },
  ).format(date);
}

export function formatDateTime(value?: string | number | null): string {
  if (value === null || value === undefined) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(date);
}

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  const unit = units[exponent] ?? "B";
  return `${value.toFixed(exponent === 0 || value >= 10 ? 0 : 1)} ${unit}`;
}

export function senderName(addresses?: EmailAddress[] | null): string {
  const first = addresses?.[0];
  if (!first) return "Unknown";
  return first.name || first.email;
}

export function addressList(addresses?: EmailAddress[] | null): string {
  if (!addresses || addresses.length === 0) return "";
  return addresses.map((entry) => entry.name || entry.email).join(", ");
}
