export const PRECACHE_PLACEHOLDER = "self.__PRECACHE = [];";

export interface PrecacheManifest {
  cacheName: string;
  urls: string[];
}

function shouldPrecache(file: string): boolean {
  if (file === "sw.js") return false;
  if (file === "index.html") return true;
  if (file.endsWith(".webmanifest")) return true;
  return file.startsWith("assets/") || file.startsWith("icons/");
}

function normaliseBase(base: string): string {
  const trimmed = base.replace(/^\/+/, "").replace(/\/+$/, "");
  return trimmed ? `/${trimmed}/` : "/";
}

function fingerprint(input: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

export function buildPrecacheManifest(files: string[], base: string): PrecacheManifest {
  const prefix = normaliseBase(base);
  const urls = files
    .filter(shouldPrecache)
    .map((file) => `${prefix}${file.replace(/^\/+/, "")}`)
    .sort();
  return { cacheName: `irixmail-shell-${fingerprint(urls.join("\n"))}`, urls };
}

export function injectPrecache(source: string, manifest: PrecacheManifest): string {
  if (!source.includes(PRECACHE_PLACEHOLDER)) {
    throw new Error(`service worker is missing the "${PRECACHE_PLACEHOLDER}" placeholder`);
  }
  const injected = [
    `self.__PRECACHE = ${JSON.stringify(manifest.urls)};`,
    `self.__SHELL_CACHE = ${JSON.stringify(manifest.cacheName)};`,
  ].join("\n");
  return source.replace(PRECACHE_PLACEHOLDER, injected);
}
