#!/usr/bin/env bash
# Starts a throwaway PassBundle server for the core's server tests
# (core/tests/server.rs), on plain HTTP at 127.0.0.1, which the core allows
# for this computer only. Prints its URL once it answers; stop it with
# `kill $(cat <data folder>/server.pid)`.
#
#   tools/test-server.sh <passbundle-server binary> <port> <data folder>
set -euo pipefail

bin=$1
port=$2
data=$3
mkdir -p "$data"

DATA_FOLDER="$data" \
  ROCKET_ADDRESS=127.0.0.1 \
  ROCKET_PORT="$port" \
  DOMAIN="http://127.0.0.1:$port" \
  WEB_VAULT_ENABLED=false \
  SIGNUPS_ALLOWED=true \
  SIGNUPS_VERIFY=false \
  DISABLE_ICON_DOWNLOAD=true \
  LOGIN_RATELIMIT_SECONDS=1 \
  LOGIN_RATELIMIT_MAX_BURST=1000 \
  LOGIN_RATELIMIT_ACCOUNT_SECONDS=1 \
  LOGIN_RATELIMIT_ACCOUNT_MAX_BURST=1000 \
  LOGIN_RATELIMIT_ACCOUNT_ANYWHERE_SECONDS=1 \
  LOGIN_RATELIMIT_ACCOUNT_ANYWHERE_MAX_BURST=1000 \
  UNAUTHENTICATED_RATELIMIT_SECONDS=1 \
  UNAUTHENTICATED_RATELIMIT_MAX_BURST=1000 \
  LOG_LEVEL=warn \
  LOG_FILE="$data/server.log" \
  "$bin" > /dev/null 2>&1 &
echo $! > "$data/server.pid"

for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:$port/alive" > /dev/null 2>&1; then
    echo "http://127.0.0.1:$port"
    exit 0
  fi
  sleep 0.5
done
echo "The server didn't start; its log:" >&2
cat "$data/server.log" >&2 || true
exit 1
