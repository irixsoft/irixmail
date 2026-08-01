import { readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

import { buildPrecacheManifest, injectPrecache } from "./src/pwa/precache-manifest";

const BASE = "/webmail/";

async function listFiles(dir: string, prefix = ""): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const name = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) files.push(...(await listFiles(path.join(dir, entry.name), name)));
    else files.push(name);
  }
  return files;
}

function precacheServiceWorker(): Plugin {
  let outDir = "dist";
  return {
    name: "irixmail-precache-sw",
    apply: "build",
    configResolved(config) {
      outDir = path.resolve(config.root, config.build.outDir);
    },
    async closeBundle() {
      const swPath = path.join(outDir, "sw.js");
      const manifest = buildPrecacheManifest(await listFiles(outDir), BASE);
      const source = await readFile(swPath, "utf8");
      await writeFile(swPath, injectPrecache(source, manifest));
      this.info(`precached ${manifest.urls.length} files as ${manifest.cacheName}`);
    },
  };
}

export default defineConfig({
  base: BASE,
  plugins: [react(), tailwindcss(), precacheServiceWorker()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: 5174,
    proxy: {
      "/api": "http://localhost:8080",
      "/jmap": "http://localhost:8080",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
