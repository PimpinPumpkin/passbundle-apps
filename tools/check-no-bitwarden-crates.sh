#!/usr/bin/env bash
# The core never links Bitwarden's crates (docs/core.md, "Clean room" in the
# meta repo): fails if any appears in core/Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/../core"
if grep -nE '^name = "(bitwarden|desktop_|biometric|secure_memory)' Cargo.lock; then
  echo "The core must not depend on these crates." >&2
  exit 1
fi
echo "No Bitwarden crates."
