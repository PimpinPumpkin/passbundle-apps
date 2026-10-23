#!/usr/bin/env bash
# Measures a built app on Linux or macOS: the bundle's size (as is, and
# compressed, about what a download weighs), time to the first frame, unlock
# time (the full KDF), and memory once the vault is open, plus a screenshot.
# CI runs it after each build; on Linux it needs a display (DISPLAY, such as
# Xvfb's).
#
# It needs a build with test hooks, as CI's are: the core built with its
# test-hooks feature (cargo build --release --features ffi,test-hooks) and
# the app with --dart-define=PB_TEST_HOOKS=true. Those take PB_MEASURE,
# PB_DATA_DIR, and PB_SECRETS=memory from the environment; a release
# ignores them, and would open on the computer's own PassBundle folder.
#
#   tools/measure.sh <executable> <bundle> <screenshot.png>
set -euo pipefail

exe=$1
bundle=$2
shot=$3
now_us() { python3 -c 'import time; print(int(time.time() * 1e6))'; }

# The sample vault (the core's test vectors) kept as a device keeps one, so
# the app opens locked and unlocks it by itself, offline.
data=$(mktemp -d)
cp "$(dirname "$0")/../app/test/sample/settings.json" "$(dirname "$0")/../app/test/sample/sync.json" "$data/"
lines=$(mktemp)
started=$(now_us)
PB_MEASURE=1 PB_MEASURE_FILE="$lines" PB_MEASURE_PASSWORD='correct horse battery staple 1' \
  PB_DATA_DIR="$data" PB_SECRETS=memory "$exe" > /dev/null 2>&1 &
pid=$!
for _ in $(seq 1 120); do
  grep -q '^ready' "$lines" 2> /dev/null && break
  kill -0 "$pid" 2> /dev/null || { echo "The app quit before it was ready:" >&2; cat "$lines" >&2; exit 1; }
  sleep 0.5
done
grep -q '^ready' "$lines" || { echo "The app wasn't ready within a minute:" >&2; cat "$lines" >&2; kill "$pid"; exit 1; }

first=$(awk '/^first-frame/ { print $2 }' "$lines")
unlock=$(awk '/^unlocked/ { print $2 }' "$lines")
if [ "$(uname)" = Linux ]; then
  rss_kb=$(awk '/^VmRSS/ { print $2 }' "/proc/$pid/status")
  pss_kb=$(awk '/^Pss:/ { print $2 }' "/proc/$pid/smaps_rollup")
  import -window root "$shot" 2> /dev/null || true
else
  rss_kb=$(ps -o rss= -p "$pid" | tr -d ' ')
  # What Activity Monitor calls Memory.
  pss_kb=$(footprint -p "$pid" 2> /dev/null | awk '/phys_footprint:/ { v = $2; u = $3; if (u ~ /^G/) v *= 1048576; else if (u ~ /^M/) v *= 1024; print int(v); exit }' || true)
  screencapture -x "$shot" 2> /dev/null || true
fi
kill "$pid"
wait "$pid" 2> /dev/null || true

size_kb=$(du -sk "$bundle" | cut -f1)
packed_kb=$(( $(tar -czf - -C "$(dirname "$bundle")" "$(basename "$bundle")" | wc -c) / 1024 ))
mb() { awk -v k="$1" 'BEGIN { printf "%.1f MB", k / 1024 }'; }

cat << EOF
| | $(uname -s) |
|---|---|
| App on disk | $(mb "$size_kb") |
| Compressed | $(mb "$packed_kb") |
| First frame after launch | $(( (first - started) / 1000 )) ms |
| Unlock (PBKDF2, 600,000 iterations) | ${unlock} ms |
| Memory, vault open (RSS) | $(mb "$rss_kb") |
| Memory, vault open ($( [ "$(uname)" = Linux ] && echo PSS || echo footprint )) | $( [ -n "${pss_kb:-}" ] && mb "$pss_kb" || echo "n/a") |
EOF
