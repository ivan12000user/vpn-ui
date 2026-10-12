#!/usr/bin/env bash
# Read-only, temporary VPN UI preview on VPS loopback port 18792.
# Opens NO public port; NEVER restarts vpn-ui, WG, AWG, Tailscale or nginx.
set -euo pipefail
umask 077

UNIT=vpn-ui-ts-preview.service
BIN=/opt/vpn-ui-ts-lab/vpn-ui
PORT=18792
ID=/etc/vpn-ui/ts-oauth-id
SECRET=/etc/vpn-ui/ts-oauth-secret
BASE="http://127.0.0.1:${PORT}"

echo "===== PRECHECK ====="
test "$(id -u)" -eq 0
test -x "$BIN"
test -s "$ID"
test -s "$SECRET"

for svc in vpn-ui wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 nginx; do
    systemctl is-active --quiet "$svc"
done

if systemctl is-active --quiet "$UNIT"; then
    echo "Preview already active; leave existing instance untouched" >&2
    exit 1
fi

if ss -H -lnt "( sport = :${PORT} )" | grep -q .; then
    echo "Preview port ${PORT} is already in use; no changes made" >&2
    exit 1
fi

TS_ID=$(tr -d '\r\n' < "$ID")
test -n "$TS_ID"

echo "===== START TEMPORARY PREVIEW (max 15 min) ====="
systemd-run --unit=vpn-ui-ts-preview --collect \
    --property=Type=exec \
    --property=DynamicUser=yes \
    --property=NoNewPrivileges=yes \
    --property=ProtectSystem=strict \
    --property=ProtectHome=yes \
    --property=PrivateTmp=yes \
    --property=PrivateDevices=yes \
    --property=CapabilityBoundingSet= \
    --property=MemoryMax=64M \
    --property=TasksMax=32 \
    --property=RuntimeMaxSec=900s \
    --property=RuntimeDirectory=vpn-ui-ts-preview \
    --property=LoadCredential=ts_oauth_secret:"$SECRET" \
    --setenv=VPN_UI_LISTEN=127.0.0.1:${PORT} \
    --setenv=VPN_UI_TS_TAILNET=- \
    --setenv=VPN_UI_TS_OAUTH_CLIENT_ID="$TS_ID" \
    --setenv=VPN_UI_TS_OAUTH_SECRET_FILE=/run/credentials/vpn-ui-ts-preview.service/ts_oauth_secret \
    --setenv=VPN_UI_GEOIP_CACHE=/run/vpn-ui-ts-preview/geoip.json \
    --setenv=VPN_UI_WG_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG31_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_WG_METADATA_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG_METADATA_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG31_METADATA_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_WG_MANAGE_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG_MANAGE_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG31_MANAGE_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_WG_SETTINGS_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG_SETTINGS_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG31_SETTINGS_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_WG_CLIENT_CONFIG_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG_CLIENT_CONFIG_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_AWG31_CLIENT_CONFIG_COMMAND=/usr/bin/false \
    --setenv=VPN_UI_PING_COMMAND=/usr/bin/false \
    "$BIN"
unset TS_ID

echo "===== VERIFY PREVIEW ====="
ready=0
for _ in $(seq 1 30); do
    if curl -fsS --max-time 2 "$BASE/healthz" >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 1
done
if [ "$ready" -ne 1 ]; then
    echo "Preview failed; stopping temporary unit" >&2
    systemctl stop "$UNIT" >/dev/null 2>&1 || true
    exit 1
fi

# Only check HTML marker; do not print user identities, secret invite URLs,
# or invite responses.
curl -fsS --max-time 5 "$BASE/tailscale" |
    grep -F 'ts-directory-refresh' >/dev/null || {
        echo "Unexpected Tailscale page; stopping temporary unit" >&2
        systemctl stop "$UNIT" >/dev/null 2>&1 || true
        exit 1
    }

curl -fsS --max-time 90 "$BASE/api/tailscale/shares" |
python3 -c '
import json,sys
d=json.load(sys.stdin)
print("configured:",d.get("configured"))
print("users:",len(d.get("users") or []))
print("shared_users:",sum(u.get("kind")=="shared" for u in (d.get("users") or [])))
print("open_invites_received:",len(d.get("invites") or []))
print("invite_checks:",d.get("invite_checks_ok"),"/",d.get("invite_checks"))
print("warnings:",len(d.get("warnings") or []))
'

echo "===== PRODUCTION STILL ACTIVE ====="
curl -fsS --max-time 5 http://127.0.0.1:8090/healthz
echo

echo "===== ACCESS VIA SSH LOOPBACK FORWARDING ONLY ====="
echo "On Z270 WSL run: ssh -N -L 18792:127.0.0.1:18792 new-vps"
echo "Then open on Z270: http://127.0.0.1:18792/tailscale"
echo "Stop preview: ssh new-vps 'systemctl stop vpn-ui-ts-preview.service'"
