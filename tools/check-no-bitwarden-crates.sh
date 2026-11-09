#!/usr/bin/env bash
# The core never links Bitwarden's crates (docs/core.md, "Clean room" in the
# meta repo): fails if any appears in core/Cargo.lock, or in the browsers
# crate's own (browsers/Cargo.lock), which the core links on computers.
set -euo pipefail
cd "$(dirname "$0")/.."
for lock in core/Cargo.lock browsers/Cargo.lock; do
  if grep -nE '^name = "(bitwarden|desktop_|biometric|secure_memory|chromium_importer)' "$lock"; then
    echo "The core must not depend on these crates ($lock)." >&2
    exit 1
  fi
done
echo "No Bitwarden crates."
