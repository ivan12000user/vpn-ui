#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo '===== LEGACY AMNEZIAWG-WEB CLEANUP ====='
[[ "$(git branch --show-current)" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]
git diff --check

ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

LEGACY_UNIT='amneziawg-web.service'
STAMP="$(date +%Y%m%d_%H%M%S)"
BACKUP="/root/legacy-amneziawg-web-backup-$STAMP.tar.gz"
MANIFEST="/root/legacy-amneziawg-web-backup-$STAMP.manifest.txt"

WG_CONF=/etc/wireguard/wg0.conf
AWG_CONF=/etc/amnezia/amneziawg/awg0.conf

count_inventory() {
    local helper="$1"
    printf '%s\n' '{"op":"list"}' |
      "$helper" |
      python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d.get("ok") is True
peers=d.get("peers", [])
print(d.get("count", len(peers)))
'
}

capture_protected_state() {
    printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum "$WG_CONF" | cut -d' ' -f1)"
    printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum "$AWG_CONF" | cut -d' ' -f1)"
    printf 'WG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/wireguard/server.json | cut -d' ' -f1)"
    printf 'AWG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/amneziawg/server.json | cut -d' ' -f1)"
    printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
    printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
    printf 'WG_COUNTS=%s/%s/%s\n'       "$(count_inventory /usr/local/bin/vpn-ui-manage-wg)"       "$(grep -c '^\[Peer\]' "$WG_CONF")"       "$(wg show wg0 peers | wc -l)"
    printf 'AWG_COUNTS=%s/%s/%s\n'       "$(count_inventory /usr/local/bin/vpn-ui-manage-awg)"       "$(grep -c '^\[Peer\]' "$AWG_CONF")"       "$(awg show awg0 peers | wc -l)"
}

echo
echo '===== PRECHECK ====='
systemctl is-active --quiet wg-quick@wg0.service
systemctl is-active --quiet awg-quick@awg0.service
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx
systemctl is-active --quiet "$LEGACY_UNIT"
systemctl is-enabled --quiet "$LEGACY_UNIT"
nginx -t >/dev/null

[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" == 200 ]]
[[ "$(curl -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1/)" == 401 ]]

ss -lntp | grep -q '127\.0\.0\.1:8080'
ss -lntp | grep -q '0\.0\.0\.0:5002'
ss -lntp | grep -q '127\.0\.0\.1:5003'
ss -lntp | grep -q '127\.0\.0\.1:5004'

# Current vpn-ui must not depend on the legacy web stack.
if grep -RIlE   'amneziawg-web|awg-web|127\.0\.0\.1:8080|(^|[^0-9])5002([^0-9]|$)|(^|[^0-9])5003([^0-9]|$)|(^|[^0-9])5004([^0-9]|$)'   /etc/systemd/system/vpn-ui.service   /etc/systemd/system/vpn-ui.service.d   /usr/local/libexec/vpn-ui-manage   /usr/local/libexec/vpn-ui-manage-core-v070   /usr/local/bin/vpn-ui 2>/dev/null |
  grep -q .; then
    echo 'ERROR: current vpn-ui references legacy amneziawg-web components' >&2
    exit 1
fi

BEFORE="$(capture_protected_state)"
printf '%s\n' "$BEFORE"

echo
echo '===== QUIESCE LEGACY SERVICE FOR CONSISTENT BACKUP ====='
systemctl stop "$LEGACY_UNIT"
sleep 1
if systemctl is-active --quiet "$LEGACY_UNIT"; then
    echo 'ERROR: legacy service did not stop' >&2
    exit 1
fi

prebackup_failed() {
    rc=$?
    trap - ERR
    echo "BACKUP PREPARATION FAILED rc=$rc — restarting legacy service" >&2
    systemctl start "$LEGACY_UNIT" || true
    exit "$rc"
}
trap prebackup_failed ERR

echo
echo '===== BACKUP LEGACY STACK ====='
PATHS=(
  /etc/systemd/system/amneziawg-web.service
  /etc/systemd/system/multi-user.target.wants/amneziawg-web.service
  /etc/sudoers.d/amneziawg-web
  /usr/local/bin/amneziawg-web
  /usr/local/libexec/amneziawg-web-privileged
  /etc/amneziawg-web
  /var/lib/amneziawg-web
  /opt/amneziawg-web
  /etc/nginx/sites-available/amneziawg-web
  /etc/nginx/sites-enabled/amneziawg-web
  /etc/nginx/stream-enabled/awg-web-5002.conf
  /etc/nginx/stream-available/awg-web-5002.conf
  /etc/nginx/snippets/awg-status-autorefresh.conf
  /etc/nginx/ssl/awg-web.crt
  /etc/nginx/ssl/awg-web.key
)

REL=()
: >"$MANIFEST"
chmod 600 "$MANIFEST"

for p in "${PATHS[@]}"; do
    if [[ -e "$p" || -L "$p" ]]; then
        printf '%s\n' "$p" >>"$MANIFEST"
        REL+=("${p#/}")
    fi
done

if (( ${#REL[@]} == 0 )); then
    echo 'ERROR: no legacy files found; refusing cleanup' >&2
    exit 1
fi

tar -C / -czf "$BACKUP" "${REL[@]}"
chmod 600 "$BACKUP"

echo "backup   = $BACKUP"
echo "manifest = $MANIFEST"
du -h "$BACKUP"

trap - ERR

rollback() {
    rc=$?
    trap - ERR
    echo "LEGACY CLEANUP FAILED rc=$rc — ROLLBACK" >&2

    tar -C / -xzf "$BACKUP" || true
    systemctl daemon-reload || true
    systemctl enable "$LEGACY_UNIT" >/dev/null 2>&1 || true
    systemctl start "$LEGACY_UNIT" || true
    nginx -t >/dev/null 2>&1 && systemctl reload nginx || true

    echo "rollback backup = $BACKUP" >&2
    exit "$rc"
}
trap rollback ERR

echo
echo '===== DISABLE LEGACY SERVICE ====='
systemctl disable "$LEGACY_UNIT"
sleep 1

if systemctl is-active --quiet "$LEGACY_UNIT"; then
    echo 'ERROR: legacy service still active' >&2
    false
fi

echo
echo '===== REMOVE LEGACY NGINX FRONTEND ====='
rm -f   /etc/nginx/sites-enabled/amneziawg-web   /etc/nginx/sites-available/amneziawg-web   /etc/nginx/stream-enabled/awg-web-5002.conf   /etc/nginx/stream-available/awg-web-5002.conf

# These files belonged to the legacy site. Remove them only when no other
# remaining Nginx configuration references them.
for candidate in   /etc/nginx/snippets/awg-status-autorefresh.conf   /etc/nginx/ssl/awg-web.crt   /etc/nginx/ssl/awg-web.key
do
    [[ -e "$candidate" ]] || continue
    base="$(basename "$candidate")"
    if grep -RIlF "$base" /etc/nginx 2>/dev/null | grep -vFx "$candidate" | grep -q .; then
        echo "keeping referenced file: $candidate"
    else
        rm -f "$candidate"
        echo "removed unreferenced file: $candidate"
    fi
done

nginx -t
systemctl reload nginx
sleep 1
systemctl is-active --quiet nginx

echo
echo '===== REMOVE LEGACY PROGRAM / DATA ====='
rm -f   /etc/sudoers.d/amneziawg-web   /usr/local/bin/amneziawg-web   /usr/local/libexec/amneziawg-web-privileged   /etc/systemd/system/amneziawg-web.service

rm -rf   /etc/amneziawg-web   /var/lib/amneziawg-web   /opt/amneziawg-web

systemctl daemon-reload
systemctl reset-failed "$LEGACY_UNIT" >/dev/null 2>&1 || true

echo
echo '===== VERIFY LEGACY STACK IS GONE ====='
if systemctl cat "$LEGACY_UNIT" >/dev/null 2>&1; then
    echo 'ERROR: legacy systemd unit still exists' >&2
    false
fi

if ss -lntp | grep -Eq ':(5002|5003|5004|8080)[[:space:]]'; then
    echo 'ERROR: legacy TCP listeners remain' >&2
    ss -lntp | grep -E ':(5002|5003|5004|8080)[[:space:]]' >&2 || true
    false
fi

if nginx -T 2>&1 | grep -Eq   'amneziawg-web|awg-web|127\.0\.0\.1:8080|listen .*5002|listen .*5003|listen .*5004'; then
    echo 'ERROR: effective Nginx config still references legacy AWG web stack' >&2
    false
fi

for p in   /etc/sudoers.d/amneziawg-web   /usr/local/bin/amneziawg-web   /usr/local/libexec/amneziawg-web-privileged   /etc/amneziawg-web   /var/lib/amneziawg-web   /opt/amneziawg-web
do
    if [[ -e "$p" || -L "$p" ]]; then
        echo "ERROR: legacy path remains: $p" >&2
        false
    fi
done

echo
echo '===== VERIFY CURRENT PRODUCTION ====='
systemctl is-active --quiet wg-quick@wg0.service
systemctl is-active --quiet awg-quick@awg0.service
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx
nginx -t >/dev/null

[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" == 200 ]]
[[ "$(curl -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1/)" == 401 ]]

ss -lunp | grep -Eq '(^|[[:space:]])0\.0\.0\.0:8443[[:space:]]|\[::\]:8443[[:space:]]'
ss -lntp | grep -q '127\.0\.0\.1:8090'

AFTER="$(capture_protected_state)"
printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
    echo 'ERROR: protected WG/AWG/vpn-ui state changed' >&2
    diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
    false
fi

trap - ERR

echo
echo '===== ACCOUNT ====='
echo 'awg-web user/group intentionally kept for now (rollback-safe cleanup)'
getent passwd awg-web || true
getent group awg-web || true

echo
echo '========================================'
echo 'LEGACY AMNEZIAWG-WEB CLEANUP = PASS'
echo 'LEGACY TCP 5002/5003/5004/8080 = REMOVED'
echo 'LEGACY SERVICE / SUDOERS / BINARY / DB = REMOVED'
echo 'NGINX = RELOADED'
echo 'CURRENT VPN-UI = HEALTHY'
echo 'AWG UDP 8443 = PRESENT'
echo 'WG0 / AWG0 = NOT RESTARTED'
echo 'WG/AWG CONFIGS AND PEER COUNTS = UNCHANGED'
echo "BACKUP = $BACKUP"
echo '========================================'
REMOTE
