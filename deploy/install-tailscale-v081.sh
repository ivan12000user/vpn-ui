#!/usr/bin/env bash
# Atomic deploy for VPN UI 0.8.1 (Tailscale admin read-only directory).
# Only vpn-ui.service is restarted; wg0, awg0, awg1, nginx and tailscaled stay up.
set -Eeuo pipefail
umask 077
export LC_ALL=C

BIN=/usr/local/bin/vpn-ui
NEW=/opt/vpn-ui-ts-lab/vpn-ui.upload
DIR=/etc/systemd/system/vpn-ui.service.d
DROP="$DIR/tailscale-oauth.conf"
ID=/etc/vpn-ui/ts-oauth-id
SECRET=/etc/vpn-ui/ts-oauth-secret
BACKUP=""
DEPLOYED=0
DONE=0

rollback() {
    local result=$?
    trap - EXIT
    if [ "$DONE" != 1 ] && [ "$DEPLOYED" = 1 ] && [ -f "$BACKUP/vpn-ui.previous" ]; then
        echo "===== DEPLOYMENT FAILED: AUTOMATIC ROLLBACK ====="
        set +e
        install -m 0755 "$BACKUP/vpn-ui.previous" "${BIN}.rollback-new"
        mv -f "${BIN}.rollback-new" "$BIN"
        rm -f "$DROP"
        systemctl daemon-reload
        systemctl restart vpn-ui.service
        echo "===== HEALTH AFTER ROLLBACK ====="
        curl --silent --show-error --max-time 5 http://127.0.0.1:8090/healthz
        echo
        systemctl is-active vpn-ui.service
        set -e
    fi
    exit "$result"
}
trap rollback EXIT

echo "===== PREDEPLOY ====="
test "$(id -u)" -eq 0
test -x "$BIN"
test -s "$NEW"
test -s "$ID"
test -s "$SECRET"
test ! -e "$DROP"
test "$(stat -c %a "$SECRET")" = 600
test "$(stat -c %U "$SECRET")" = root
test "$(stat -c %U "$ID")" = root
test "$(stat -c %a "$ID")" = 600
test "$(systemctl is-active vpn-ui.service)" = active
for service in wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 nginx; do
    systemctl is-active --quiet "$service"
done
nginx -t >/dev/null 2>&1
if ! nginx -T 2>/dev/null | grep -F 'auth_basic "VPN UI";' >/dev/null; then
    echo "REFUSING: VPN UI HTTPS basic-auth configuration not found" >&2
    exit 1
fi
old=$(curl --fail --silent --show-error --max-time 5 http://127.0.0.1:8090/healthz |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])')
test "$old" = 0.8.0 || { echo "REFUSING: expected v0.8.0, got $old"; exit 1; }
id=$(tr -d '\r\n' < "$ID")
[[ "$id" =~ ^[A-Za-z0-9._-]{8,128}$ ]] || {
    echo "REFUSING: unexpected OAuth client ID format" >&2
    exit 1
}

mkdir -p /root/vpn-ui-backups
BACKUP=$(mktemp -d /root/vpn-ui-backups/v081-ts-XXXXXXXX)
chmod 0700 "$BACKUP"
cp -a "$BIN" "$BACKUP/vpn-ui.previous"
sha256sum "$BIN" "$NEW" > "$BACKUP/binary-sha256.txt"
systemctl cat vpn-ui.service > "$BACKUP/vpn-ui.service.before.txt"
echo "Backup: $BACKUP"

echo "===== INSTALL V0.8.1 ====="
install -d -m 0755 "$DIR"
{
    printf '[Service]\n'
    printf 'LoadCredential=ts_oauth_secret:%s\n' "$SECRET"
    printf 'Environment=VPN_UI_TS_TAILNET=-\n'
    printf 'Environment=VPN_UI_TS_OAUTH_CLIENT_ID=%s\n' "$id"
    printf 'Environment=VPN_UI_TS_OAUTH_SECRET_FILE=/run/credentials/vpn-ui.service/ts_oauth_secret\n'
} > "${DROP}.new"
chmod 0600 "${DROP}.new"
mv -f "${DROP}.new" "$DROP"
unset id

install -m 0755 "$NEW" "${BIN}.v081-new"
DEPLOYED=1
mv -f "${BIN}.v081-new" "$BIN"
systemctl daemon-reload
systemctl restart vpn-ui.service

echo "===== VERIFY SERVICE / VERSION ====="
ready=0
for i in $(seq 1 20); do
    if curl --silent --fail --max-time 2 http://127.0.0.1:8090/healthz > "$BACKUP/health-after.json" 2>/dev/null; then
        if python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["version"]=="0.8.1"' "$BACKUP/health-after.json"; then
            ready=1
            break
        fi
    fi
    sleep 1
done
test "$ready" = 1
cat "$BACKUP/health-after.json"
echo

echo "===== VERIFY LIVE TAILSCALE DATA (COUNTS ONLY) ====="
JSON_FILE=$(mktemp /run/vpn-ui-ts-v081-XXXXXXXX.json)
if ! curl --fail --silent --show-error --max-time 120 \
    http://127.0.0.1:8090/api/tailscale/shares -o "$JSON_FILE"; then
    rm -f "$JSON_FILE"
    exit 1
fi
if ! python3 - "$JSON_FILE" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))
users=d.get("users") or []
invites=d.get("invites") or []
warnings=d.get("warnings") or []
checked=d.get("invite_checks_ok",0)
total=d.get("invite_checks",0)
shared=sum(u.get("kind")=="shared" for u in users)
print("configured:",d.get("configured"))
print("users:",len(users),"shared:",shared)
print("open_invites:",len(invites))
print("device_invite_checks:",checked,"/",total)
print("warnings:",len(warnings))
assert d.get("configured") is True
assert len(users)>=2 and shared>=1
assert total>0 and checked==total and not warnings
PY
then
    rm -f "$JSON_FILE"
    exit 1
fi
rm -f "$JSON_FILE"

echo "===== VERIFY VPN SERVICES (NO RESTART) ====="
for service in vpn-ui wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 nginx; do
    printf '%-22s ' "$service"
    systemctl is-active "$service"
done

DONE=1
echo "===== SUCCESS: VPN UI 0.8.1 ACTIVE ====="
echo "Rollback backup: $BACKUP"
