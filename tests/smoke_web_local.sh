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

# Fake local Tailscale netmap and ping binary: CI never probes a live network.
cat > "$tmp/tailscale-ro" <<'EOF'
#!/bin/sh
cat <<'JSON'
{"BackendState":"Running","Self":{"HostName":"test-server","TailscaleIPs":["100.80.198.69"]},"Peer":{"test-peer":{"HostName":"lab-peer","Online":true,"TailscaleIPs":["100.92.11.114"]}}}
JSON
EOF
chmod 0755 "$tmp/tailscale-ro"

# Fake restricted native helper input/output.
cat > "$tmp/tailscale-native" <<'EOF'
#!/bin/sh
IFS= read -r ip || exit 1
[ "$ip" = "100.92.11.114" ] || exit 64
echo 'pong from lab-peer (100.92.11.114) via DERP(ams) in 148ms'
echo 'pong from lab-peer (100.92.11.114) via DERP(ams) in 217ms'
echo 'direct connection not established'
EOF
chmod 0755 "$tmp/tailscale-native"

# Fixed nonprivileged localhost port on isolated GitHub runner.
base="http://127.0.0.1:18790"
env \
  VPN_UI_TAILSCALE_COMMAND="$tmp/tailscale-ro" \
  VPN_UI_PING_COMMAND=/usr/bin/false \
  VPN_UI_TS_PING_COMMAND="$tmp/tailscale-native" \
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

# Without administrator credentials the invited-users API must remain disabled
# and must not attempt external Tailscale API requests.
curl --silent --show-error --fail --max-time 5 \
  "$base/api/tailscale/shares" -o "$tmp/ts-shares.json"
python3 - "$tmp/ts-shares.json" <<'PY'
import json
from pathlib import Path
import sys

data = json.loads(Path(sys.argv[1]).read_text())
assert data["configured"] is False, data
assert data["users"] == [], data
assert data["invites"] == [], data
print("TAILSCALE INVITES DISABLED-BY-DEFAULT PASS")
PY

# Ping is read-only and MUST be restricted to the current Tailscale netmap.
curl --silent --show-error --fail --max-time 12 \
  -H 'Content-Type: application/json' \
  --data '{"ip":"100.92.11.114"}' \
  "$base/api/tailscale/ping" -o "$tmp/ts-ping-known.json"
curl --silent --show-error --fail --max-time 12 \
  -H 'Content-Type: application/json' \
  --data '{"ip":"100.92.11.115"}' \
  "$base/api/tailscale/ping" -o "$tmp/ts-ping-unknown.json"
curl --silent --show-error --fail --max-time 12 \
  -H 'Content-Type: application/json' \
  --data '{"ip":"127.0.0.1"}' \
  "$base/api/tailscale/ping" -o "$tmp/ts-ping-loopback.json"
python3 - "$tmp/ts-ping-known.json" "$tmp/ts-ping-unknown.json" "$tmp/ts-ping-loopback.json" <<'PY'
import json
import sys
from pathlib import Path

known, unknown, loopback = [json.loads(Path(p).read_text()) for p in sys.argv[1:]]
# Our deliberately non-functional ICMP command is reached for known peers.
assert known["ip"] == "100.92.11.114", known
assert known["ok"] is False and known["error"] == "timeout", known
# Unrecognized peers and arbitrary IPs never reach the ping command.
for rejected in (unknown, loopback):
    assert rejected["ok"] is False, rejected
    assert rejected["ip"] == "", rejected
    assert rejected["error"] == "Tailscale peer not found", rejected
print("TAILSCALE PING WHITELIST PASS")
PY

# Native ping is distinct from normal ICMP; it never probes arbitrary IPs.
curl --silent --show-error --fail --max-time 12 \
  -H 'Content-Type: application/json' \
  --data '{"ip":"100.92.11.114"}' \
  "$base/api/tailscale/overlay-ping" -o "$tmp/ts-native-known.json"
curl --silent --show-error --fail --max-time 12 \
  -H 'Content-Type: application/json' \
  --data '{"ip":"127.0.0.1"}' \
  "$base/api/tailscale/overlay-ping" -o "$tmp/ts-native-denied.json"
python3 - "$tmp/ts-native-known.json" "$tmp/ts-native-denied.json" <<'PY'
import json,sys
a,b=(json.load(open(x)) for x in sys.argv[1:])
assert a["ok"] is True, a
assert a["path"] == "DERP" and a["relay"] == "ams", a
assert a["received"] == 2, a
assert abs(a["avg_ms"]-182.5)<0.01, a
assert b["ok"] is False and b["ip"] == "", b
print("TAILSCALE NATIVE DERP PING PASS")
PY
curl --silent --show-error --fail --max-time 10 "$base/tailscale" -o "$tmp/ts-page.html"
grep -F '<th>Tailscale ping</th>' "$tmp/ts-page.html" >/dev/null
grep -F '<th>ICMP</th>' "$tmp/ts-page.html" >/dev/null
grep -F 'Проверить все TS' "$tmp/ts-page.html" >/dev/null
echo "TAILSCALE DUAL PING UI PASS"

# No history helper or database on a disposable runner: respond gracefully.
curl --silent --show-error --fail --max-time 8   "$base/api/history?hours=168" -o "$tmp/history.json"
python3 - "$tmp/history.json" <<'PY'
import json,sys
a=json.load(open(sys.argv[1]))
assert a["configured"] is False, a
assert a["points"] == [], a
print("HISTORY NOT INSTALLED PASS")
PY
curl --silent --show-error --fail --max-time 8 "$base/history"   -o "$tmp/history.html"
grep -F 'id="hist-graph"' "$tmp/history.html" >/dev/null
grep -F '<details class="mobile-nav">' "$tmp/history.html" >/dev/null
grep -F 'aria-label="Меню разделов VPN UI"' "$tmp/history.html" >/dev/null
grep -F 'class="nav-links-desktop"' "$tmp/history.html" >/dev/null
grep -F 'class="history-filters"' "$tmp/history.html" >/dev/null
grep -F 'max-width: 900px' "$tmp/history.html" >/dev/null
echo 'MOBILE MENU AND HISTORY CONTROLS PASS'

echo 'LOCALHOST SMOKE PASS (no root, no VPN tunnels, no external access)'
