import type { JmapSession } from "@irixmail/shared";

export const WEBPUSH_CAPABILITY = "urn:irixmail:webpush";

export function applicationServerKey(session: JmapSession): string | null {
  const capability = session.capabilities[WEBPUSH_CAPABILITY];
  if (!capability || typeof capability !== "object") return null;
  const key = (capability as { applicationServerKey?: unknown }).applicationServerKey;
  return typeof key === "string" && key.length > 0 ? key : null;
}

export function urlBase64ToUint8Array(base64: string): Uint8Array {
  const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
  const normalized = padded.replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(normalized);
  const bytes = new Uint8Array(raw.length);
  for (let index = 0; index < raw.length; index += 1) bytes[index] = raw.charCodeAt(index);
  return bytes;
}

export function deviceClientIdFor(accountId: string): string {
  const key = `irixmail.webmail.push.device.${accountId}`;
  const existing = localStorage.getItem(key);
  if (existing) return existing;
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  const id = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  localStorage.setItem(key, id);
  return id;
}

export interface BrowserSubscription {
  endpoint: string;
  keys: { p256dh: string; auth: string };
}

export function subscriptionCreateArgs(deviceClientId: string, subscription: BrowserSubscription) {
  return {
    deviceClientId,
    url: subscription.endpoint,
    keys: subscription.keys,
  };
}

export function requiresVisibleNotification(userAgent: string): boolean {
  if (/iPad|iPhone|iPod/.test(userAgent)) return true;
  return /Macintosh/.test(userAgent) && /AppleWebKit/.test(userAgent) && !/Chrome|Chromium|Edg\//.test(userAgent);
}

export interface PushNotice {
  title: string;
  body: string;
}

export function stateChangeNotice(
  changed: Record<string, Record<string, string>>,
  visible: boolean,
  userAgent: string,
): PushNotice | null {
  const mailChanged = Object.values(changed).some(
    (types) => types && Object.prototype.hasOwnProperty.call(types, "Email"),
  );
  const mustShow = requiresVisibleNotification(userAgent);
  if (mailChanged && (!visible || mustShow)) {
    return { title: "New mail", body: "You have new mail." };
  }
  if (mustShow) {
    return { title: "Mailbox updated", body: "Your mailbox changed on another device." };
  }
  return null;
}

export type PushPayload =
  | { kind: "verification"; subscriptionId: string; code: string }
  | { kind: "stateChange"; changed: Record<string, Record<string, string>> };

export function classifyPushPayload(payload: unknown): PushPayload | null {
  if (!payload || typeof payload !== "object") return null;
  const data = payload as Record<string, unknown>;
  if (data["@type"] === "PushVerification") {
    const subscriptionId = data["pushSubscriptionId"];
    const code = data["verificationCode"];
    if (typeof subscriptionId === "string" && typeof code === "string") {
      return { kind: "verification", subscriptionId, code };
    }
    return null;
  }
  if (data["@type"] === "StateChange" && data["changed"] && typeof data["changed"] === "object") {
    return { kind: "stateChange", changed: data["changed"] as Record<string, Record<string, string>> };
  }
  return null;
}
