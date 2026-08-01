#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/THIRD-PARTY-LICENSES.md"

if ! command -v cargo-about >/dev/null 2>&1; then
  echo "cargo-about is required: cargo install cargo-about --locked" >&2
  exit 1
fi

version="$(grep -m1 '^version = ' "$root/Cargo.toml" | cut -d '"' -f2)"

{
  echo "# Third-party licenses"
  echo
  echo "IRIXMAIL $version bundles the dependencies listed below inside its binary."
  echo "IRIXMAIL itself is licensed AGPL-3.0-only; see LICENSE."
  echo
  echo "Regenerate with \`bash scripts/third-party-licenses.sh\`."
  echo
  echo "## Rust dependencies"
  echo
  (cd "$root" && cargo about generate about.hbs)
  echo "## Frontend dependencies"
  echo
  python3 "$root/scripts/npm-licenses.py"
} > "$out"

echo "wrote $out"
