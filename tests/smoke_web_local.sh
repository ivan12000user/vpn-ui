#!/usr/bin/env bash
# HTTP smoke check on a disposable CI host. No root, VPN configuration or peers.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
binary="$repo/target/release/vpn-ui"
test -x "$binary" || { echo 'Missing release binary' >&2; exit 1; }

tmp="$(mktemp -d)"
pid=''
cleanup() {
  if [ -n "$pid" ]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

# Fixed nonprivileged localhost port on isolated GitHub runner.
base="http://127.0.0.1:18790"
env \
  VPN_UI_LISTEN=127.0.0.1:18790 \
  VPN_UI_GEOIP_CACHE="$tmp/geoip.json" \
  VPN_UI_WG_COMMAND=/usr/bin/false \
  VPN_UI_AWG_COMMAND=/usr/bin/false \
  VPN_UI_AWG31_COMMAND=/usr/bin/false \
  VPN_UI_WG_METADATA_COMMAND=/usr/bin/false \
  VPN_UI_AWG_METADATA_COMMAND=/usr/bin/false \
  VPN_UI_AWG31_METADATA_COMMAND=/usr/bin/false \
  "$binary" >"$tmp/server.log" 2>&1 &
pid=$!

ready=0
for _ in $(seq 1 40); do
  if curl --silent --show-error --fail --max-time 2 \
       "$base/healthz" -o "$tmp/health.json" 2>/dev/null; then
    ready=1
    break
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo 'vpn-ui unexpectedly exited:' >&2
    tail -30 "$tmp/server.log" >&2
    exit 1
  fi
  sleep 0.25
done
if [ "$ready" -ne 1 ]; then
  echo 'localhost health check timed out' >&2
  tail -30 "$tmp/server.log" >&2
  exit 1
fi

python3 - "$tmp/health.json" <<'PY'
import json
from pathlib import Path
import sys

status = json.loads(Path(sys.argv[1]).read_text())
assert status["status"] == "ok", status
assert status["application"] == "vpn-ui", status
print("HEALTHZ PASS, version =", status["version"])
PY

# Read-only pages should degrade gracefully without installed VPN executables.
for item in 'wireguard:WireGuard' 'amneziawg:AmneziaWG' \
            'amneziawg31:AmneziaWG 3.1' 'tailscale:Tailscale'; do
  route="${item%%:*}"
  label="${item#*:}"
  curl --silent --show-error --fail --max-time 10 \
    "$base/$route" -o "$tmp/page.html"
  if ! grep -Fq "$label" "$tmp/page.html"; then
    echo "Unexpected response for /$route" >&2
    exit 1
  fi
  echo "READ-ONLY PAGE PASS: /$route"
done

echo 'LOCALHOST SMOKE PASS (no root, no VPN tunnels, no external access)'
