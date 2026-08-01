#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
assets="$root/crates/http/assets"

cd "$root/frontend"
bun install --frozen-lockfile
bun run --filter @irixmail/admin build
bun run --filter @irixmail/webmail build

rm -rf "$assets/admin" "$assets/webmail"
mkdir -p "$assets/admin" "$assets/webmail"
cp -R "$root/frontend/admin/dist/." "$assets/admin/"
cp -R "$root/frontend/webmail/dist/." "$assets/webmail/"

echo "Embedded admin and webmail assets into $assets"
