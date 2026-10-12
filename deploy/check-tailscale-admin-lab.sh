#!/usr/bin/env bash
# Run as root on the VPS. This script only starts a separate, temporary
# unprivileged read-only UI on localhost; it never touches production vpn-ui.
set -euo pipefail
umask 077

UNIT=vpn-ui-ts-lab.service
BIN=/opt/vpn-ui-ts-lab/vpn-ui
PORT=18792
BASE="http://127.0.0.1:${PORT}"
SECRET=/etc/vpn-ui/ts-oauth-secret
ID=/etc/vpn-ui/ts-oauth-id
STARTED=0

cleanup() {
    if [ "$STARTED" -eq 1 ]; then
        systemctl stop "$UNIT" >/dev/null 2>&1 || true
        echo "===== LAB STOPPED ====="
    fi
}
trap cleanup EXIT

echo "===== PRECHECK ====="
test "$(id -u)" -eq 0
test -x "$BIN"
test -s "$ID"
test -s "$SECRET"
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet wg-quick@wg0.service
systemctl is-active --quiet awg-quick@awg0.service
systemctl is-active --quiet vpn-ui-awg31.service
if systemctl is-active --quiet "$UNIT"; then
    echo "FAIL: another lab is already active" >&2
    exit 1
fi
if ss -H -lnt "( sport = :${PORT} )" | grep -q .; then
    echo "FAIL: lab port already occupied" >&2
    exit 1
fi
TS_ID=$(tr -d '\r\n' < "$ID")
test -n "$TS_ID"

echo "===== START READ-ONLY LAB ====="
systemd-run \
    --unit=vpn-ui-ts-lab --collect \
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
    --property=RuntimeMaxSec=180s \
    --property=RuntimeDirectory=vpn-ui-ts-lab \
    --property=LoadCredential=ts_oauth_secret:"$SECRET" \
    --setenv=VPN_UI_LISTEN=127.0.0.1:${PORT} \
    --setenv=VPN_UI_TS_TAILNET=- \
    --setenv=VPN_UI_TS_OAUTH_CLIENT_ID="$TS_ID" \
    --setenv=VPN_UI_TS_OAUTH_SECRET_FILE=/run/credentials/vpn-ui-ts-lab.service/ts_oauth_secret \
    --setenv=VPN_UI_GEOIP_CACHE=/run/vpn-ui-ts-lab/geoip.json \
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
    --setenv=VPN_UI_TAILSCALE_COMMAND=/usr/bin/false \
    "$BIN"
STARTED=1
unset TS_ID

echo "===== CHECK HEALTH ====="
READY=0
for _ in $(seq 1 30); do
    if curl -fsS --max-time 2 "$BASE/healthz" >/dev/null 2>&1; then
        READY=1
        break
    fi
    sleep 1
done
test "$READY" -eq 1

echo "===== AUTHENTICATED TAILSCALE API COUNTS ====="
curl -fsS --max-time 120 "$BASE/api/tailscale/shares" |
python3 -c '
import json
import re
import sys
d = json.load(sys.stdin)
users = d.get("users") or []
invites = d.get("invites") or []
warnings = d.get("warnings") or []
print("configured:", d.get("configured"))
print("users_total:", len(users))
print("users_member:", sum(u.get("kind") == "member" for u in users))
print("users_shared:", sum(u.get("kind") == "shared" for u in users))
print("users_connected:", sum(u.get("currently_connected") is True for u in users))
print("invites_received:", len(invites))
print("invite_devices_checked:", d.get("invite_checks_ok"), "/", d.get("invite_checks"))
print("warnings_count:", len(warnings))
print("warning_http_statuses:", re.findall(r"HTTP [0-9]{3}", " ".join(warnings)))
'

echo "===== PRODUCTION STILL HEALTHY ====="
curl -fsS --max-time 5 http://127.0.0.1:8090/healthz
echo
for service in vpn-ui wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 nginx; do
    printf "%-24s " "$service"
    systemctl is-active "$service"
done
echo "===== LAB CHECK COMPLETE ====="
