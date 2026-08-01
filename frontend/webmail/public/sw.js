// The build rewrites the next line with the real manifest — see src/pwa/precache-manifest.ts.
self.__PRECACHE = [];

const BASE = new URL("./", self.location).pathname;

// Mirrors src/pwa/pending-verifications.ts and src/pwa/push.ts — keep in sync.
const DB_NAME = "irixmail-push";
const STORE = "pending";

const SHELL_PREFIX = "irixmail-shell-";
const SHELL_CACHE = self.__SHELL_CACHE || `${SHELL_PREFIX}dev`;
const BLOB_CACHE = "irixmail-blobs";
const BLOB_LIMIT = 100;
const PRECACHE = self.__PRECACHE || [];
const PRECACHED = new Set(PRECACHE);
const INDEX_URL = `${BASE}index.html`;

self.addEventListener("install", (event) => {
  self.skipWaiting();
  event.waitUntil(
    (async () => {
      if (!PRECACHE.length) return;
      const cache = await caches.open(SHELL_CACHE);
      await cache.addAll(PRECACHE);
    })().catch(() => undefined),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const names = await caches.keys();
      await Promise.all(
        names
          .filter((name) => name.startsWith(SHELL_PREFIX) && name !== SHELL_CACHE)
          .map((name) => caches.delete(name)),
      );
      await self.clients.claim();
    })(),
  );
});

async function trimBlobCache(cache) {
  const keys = await cache.keys();
  for (let index = 0; index < keys.length - BLOB_LIMIT; index += 1) {
    await cache.delete(keys[index]);
  }
}

async function revalidateBlob(request, cache) {
  const response = await fetch(request);
  if (response && response.ok) {
    await cache.put(request, response.clone());
    await trimBlobCache(cache);
  }
  return response;
}

async function blobStaleWhileRevalidate(event) {
  const cache = await caches.open(BLOB_CACHE);
  const cached = await cache.match(event.request);
  if (cached) {
    event.waitUntil(revalidateBlob(event.request, cache).catch(() => undefined));
    return cached;
  }
  return revalidateBlob(event.request, cache);
}

async function navigateOrShell(request) {
  try {
    return await fetch(request);
  } catch (error) {
    const cache = await caches.open(SHELL_CACHE);
    const shell = await cache.match(INDEX_URL);
    if (shell) return shell;
    throw error;
  }
}

async function shellCacheFirst(request) {
  const cache = await caches.open(SHELL_CACHE);
  const cached = await cache.match(request);
  if (cached) return cached;
  const response = await fetch(request);
  if (response && response.ok) await cache.put(request, response.clone());
  return response;
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;

  let url;
  try {
    url = new URL(request.url);
  } catch {
    return;
  }
  if (url.origin !== self.location.origin) return;

  if (url.pathname.startsWith("/jmap") || url.pathname.startsWith("/api")) {
    if (url.pathname.startsWith("/jmap/download")) event.respondWith(blobStaleWhileRevalidate(event));
    return;
  }

  if (request.mode === "navigate") {
    event.respondWith(navigateOrShell(request));
    return;
  }

  if (PRECACHED.has(url.pathname)) event.respondWith(shellCacheFirst(request));
});

function openDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE, { keyPath: "subscriptionId" });
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function persistPending(entry) {
  try {
    const db = await openDb();
    await new Promise((resolve, reject) => {
      const request = db.transaction(STORE, "readwrite").objectStore(STORE).put(entry);
      request.onsuccess = () => resolve(undefined);
      request.onerror = () => reject(request.error);
    });
    db.close();
  } catch {
    // verification still reaches open clients through the broadcast
  }
}

async function broadcast(message) {
  const clients = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  for (const client of clients) client.postMessage(message);
  return clients;
}

function requiresVisibleNotification(userAgent) {
  if (/iPad|iPhone|iPod/.test(userAgent)) return true;
  return /Macintosh/.test(userAgent) && /AppleWebKit/.test(userAgent) && !/Chrome|Chromium|Edg\//.test(userAgent);
}

function stateChangeNotice(payload, visible, userAgent) {
  const rich = payload.notification;
  const mustShow = requiresVisibleNotification(userAgent);
  if (rich && rich.title && (!visible || mustShow)) {
    return { title: rich.title, body: rich.body || "", tag: rich.tag || "irixmail-new-mail", url: rich.navigate };
  }
  if (mustShow) {
    return { title: "Mailbox updated", body: "Your mailbox changed on another device.", tag: "irixmail-sync" };
  }
  return null;
}

function showNotice(notice) {
  return self.registration.showNotification(notice.title, {
    body: notice.body,
    tag: notice.tag || "irixmail-new-mail",
    icon: `${BASE}icons/icon-192.png`,
    badge: `${BASE}icons/maskable-192.png`,
    data: { url: notice.url || BASE },
  });
}

self.addEventListener("push", (event) => {
  let payload = null;
  try {
    payload = event.data ? event.data.json() : null;
  } catch {
    payload = null;
  }
  if (!payload || typeof payload !== "object") return;

  if (payload["@type"] === "PushVerification") {
    const entry = {
      subscriptionId: String(payload.pushSubscriptionId ?? ""),
      code: String(payload.verificationCode ?? ""),
    };
    event.waitUntil(
      (async () => {
        await persistPending(entry);
        await broadcast({ kind: "push-verification", subscriptionId: entry.subscriptionId, code: entry.code });
        await showNotice({
          title: "Notifications enabled",
          body: "IRIXMAIL will notify you about new mail.",
          tag: "irixmail-push-setup",
        });
      })(),
    );
    return;
  }

  if (payload["@type"] === "StateChange") {
    event.waitUntil(
      (async () => {
        const clients = await broadcast({ kind: "state-change", changed: payload.changed ?? {} });
        const visible = clients.some((client) => client.visibilityState === "visible");
        const notice = stateChangeNotice(payload, visible, navigator.userAgent);
        if (notice) await showNotice(notice);
      })(),
    );
  }
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = (event.notification.data && event.notification.data.url) || BASE;
  event.waitUntil(
    (async () => {
      const clients = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      for (const client of clients) {
        if ("focus" in client) {
          await client.focus();
          client.postMessage({ kind: "open-url", url });
          return;
        }
      }
      await self.clients.openWindow(url);
    })(),
  );
});
