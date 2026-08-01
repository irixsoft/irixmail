import { describe, expect, it } from "vitest";

import { PRECACHE_PLACEHOLDER, buildPrecacheManifest, injectPrecache } from "./precache-manifest";

const FILES = [
  "index.html",
  "sw.js",
  "manifest.webmanifest",
  "assets/index-abc.js",
  "assets/index-abc.css",
  "icons/icon-192.png",
  "stats.json",
];

describe("buildPrecacheManifest", () => {
  it("prefixes every url with the base and sorts them", () => {
    expect(buildPrecacheManifest(FILES, "/webmail/").urls).toEqual([
      "/webmail/assets/index-abc.css",
      "/webmail/assets/index-abc.js",
      "/webmail/icons/icon-192.png",
      "/webmail/index.html",
      "/webmail/manifest.webmanifest",
    ]);
  });

  it("never precaches the service worker or unrelated files", () => {
    const urls = buildPrecacheManifest(FILES, "/webmail/").urls;
    expect(urls).not.toContain("/webmail/sw.js");
    expect(urls).not.toContain("/webmail/stats.json");
  });

  it("normalises a base without surrounding slashes", () => {
    expect(buildPrecacheManifest(["index.html"], "webmail").urls).toEqual(["/webmail/index.html"]);
    expect(buildPrecacheManifest(["index.html"], "/").urls).toEqual(["/index.html"]);
  });

  it("derives a cache name that changes with the file list", () => {
    const one = buildPrecacheManifest(FILES, "/webmail/").cacheName;
    const two = buildPrecacheManifest([...FILES, "assets/late-def.js"], "/webmail/").cacheName;
    expect(one).toMatch(/^irixmail-shell-[0-9a-f]+$/);
    expect(one).not.toBe(two);
  });

  it("is stable across file ordering", () => {
    expect(buildPrecacheManifest(FILES, "/webmail/").cacheName).toBe(
      buildPrecacheManifest([...FILES].reverse(), "/webmail/").cacheName,
    );
  });
});

describe("injectPrecache", () => {
  it("replaces the placeholder with the url list and cache name", () => {
    const source = `const a = 1;\n${PRECACHE_PLACEHOLDER}\nconst b = 2;\n`;
    const result = injectPrecache(source, buildPrecacheManifest(["index.html"], "/webmail/"));
    expect(result).toContain('self.__PRECACHE = ["/webmail/index.html"];');
    expect(result).toMatch(/self\.__SHELL_CACHE = "irixmail-shell-[0-9a-f]+";/);
    expect(result).not.toContain(PRECACHE_PLACEHOLDER);
    expect(result).toContain("const b = 2;");
  });

  it("throws when the placeholder is gone", () => {
    expect(() => injectPrecache("const a = 1;", buildPrecacheManifest([], "/"))).toThrow();
  });
});
