import type { JmapClient, JmapSession } from "@irixmail/shared";

import {
  applicationServerKey,
  deviceClientIdFor,
  subscriptionCreateArgs,
  urlBase64ToUint8Array,
} from "./push";
import { clearPending, listPending, removePending } from "./pending-verifications";

const SUB_PREFIX = "irixmail.webmail.push.sub.";
const SUB_KEY = (accountId: string) => `${SUB_PREFIX}${accountId}`;

export class PushVerifyRejected extends Error {}

export interface ServerSubscription {
  id: string;
  deviceClientId: string;
  verified?: boolean;
  expires?: string | null;
}

export interface PushStatus {
  supported: boolean;
  keyAvailable: boolean;
  permission: NotificationPermission | "unsupported";
  enabled: boolean;
  verified: boolean;
}

export function pushSupported(): boolean {
  return "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
}

export async function pushStatus(jmap: JmapClient, session: JmapSession, accountId: string): Promise<PushStatus> {
  if (!pushSupported()) {
    return { supported: false, keyAvailable: false, permission: "unsupported", enabled: false, verified: false };
  }
  const keyAvailable = applicationServerKey(session) != null;
  const permission = Notification.permission;
  const storedId = localStorage.getItem(SUB_KEY(accountId));
  let enabled = false;
  let verified = false;
  if (storedId && permission === "granted") {
    const registration = await navigator.serviceWorker.getRegistration();
    const browserSub = await registration?.pushManager.getSubscription();
    if (browserSub) {
      const response = await jmap.call<{ list: ServerSubscription[] }>("PushSubscription/get", { ids: null });
      const server = response.list.find((entry) => entry.id === storedId);
      enabled = Boolean(server);
      verified = Boolean(server?.verified);
    }
  }
  return { supported: true, keyAvailable, permission, enabled, verified };
}

export async function enableWebPush(jmap: JmapClient, session: JmapSession, accountId: string): Promise<void> {
  const key = applicationServerKey(session);
  if (!key) throw new Error("push is not available on this server");
  const permission = await Notification.requestPermission();
  if (permission !== "granted") throw new Error("notification permission was not granted");

  const registration = await navigator.serviceWorker.ready;
  const existing = await registration.pushManager.getSubscription();
  const browserSub =
    existing ??
    (await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(key).buffer as ArrayBuffer,
    }));
  const json = browserSub.toJSON();
  if (!json.endpoint || !json.keys?.["p256dh"] || !json.keys?.["auth"]) {
    throw new Error("browser subscription is incomplete");
  }

  const deviceClientId = deviceClientIdFor(accountId);
  const current = await jmap.call<{ list: ServerSubscription[] }>("PushSubscription/get", { ids: null });
  const stale = current.list.filter((entry) => entry.deviceClientId === deviceClientId);
  if (stale.length > 0) {
    await jmap.call("PushSubscription/set", { destroy: stale.map((entry) => entry.id) });
  }

  const created = await jmap.call<{ created?: Record<string, { id: string }> }>("PushSubscription/set", {
    create: {
      sub: subscriptionCreateArgs(deviceClientId, {
        endpoint: json.endpoint,
        keys: { p256dh: json.keys["p256dh"], auth: json.keys["auth"] },
      }),
    },
  });
  const id = created.created?.["sub"]?.id;
  if (!id) throw new Error("the server rejected the subscription");
  localStorage.setItem(SUB_KEY(accountId), id);
}

export async function disableWebPush(jmap: JmapClient, accountId: string): Promise<void> {
  return teardownPush(jmap, accountId);
}

export async function teardownPush(
  jmap: JmapClient | null,
  accountId: string | null,
  factory: IDBFactory = indexedDB,
): Promise<void> {
  if (jmap && accountId) {
    const storedId = localStorage.getItem(SUB_KEY(accountId));
    if (storedId) {
      await jmap.call("PushSubscription/set", { destroy: [storedId] }).catch(() => undefined);
    }
  }
  for (const key of Object.keys(localStorage)) {
    if (key.startsWith(SUB_PREFIX)) localStorage.removeItem(key);
  }
  await clearPending(factory).catch(() => undefined);
  try {
    if ("serviceWorker" in navigator) {
      const registration = await navigator.serviceWorker.getRegistration();
      const browserSub = await registration?.pushManager.getSubscription();
      if (browserSub) await browserSub.unsubscribe();
    }
  } catch {
    // browser subscription cleanup is best-effort
  }
}

export async function verifySubscription(jmap: JmapClient, subscriptionId: string, code: string): Promise<void> {
  const response = await jmap.call<{ notUpdated?: Record<string, { type?: string; description?: string }> }>(
    "PushSubscription/set",
    { update: { [subscriptionId]: { verificationCode: code } } },
  );
  const rejection = response?.notUpdated?.[subscriptionId];
  if (rejection) {
    throw new PushVerifyRejected(
      `verification rejected: ${rejection.type ?? "unknown"} ${rejection.description ?? ""}`.trimEnd(),
    );
  }
}

export async function drainPendingVerifications(jmap: JmapClient, factory: IDBFactory = indexedDB): Promise<number> {
  const entries = await listPending(factory);
  let verified = 0;
  for (const entry of entries) {
    try {
      await verifySubscription(jmap, entry.subscriptionId, entry.code);
      await removePending(entry.subscriptionId, factory);
      verified += 1;
    } catch (error) {
      if (error instanceof PushVerifyRejected) {
        await removePending(entry.subscriptionId, factory).catch(() => undefined);
      }
      // network errors keep the entry so the next launch retries
    }
  }
  return verified;
}
