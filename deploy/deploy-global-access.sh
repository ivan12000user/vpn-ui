#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
EXPECTED_CURRENT_UI_SHA='ebb19f88c5234a2e54e1b8c380ce2f25ffd331b7af9116fc06a9755b4a695381'
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf '%s\n' '===== GLOBAL ACCESS PRECHECK ====='
[[ "$(git branch --show-current)" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]
git diff --check
cargo fmt --check
cargo check
cargo test

REMOTE_UI_SHA="$(ssh "$REMOTE" "sha256sum /usr/local/bin/vpn-ui | cut -d' ' -f1")"
echo "production UI SHA = $REMOTE_UI_SHA"
[[ "$REMOTE_UI_SHA" == "$EXPECTED_CURRENT_UI_SHA" ]]

BEFORE="$(ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
printf 'WG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/wireguard/server.json | cut -d' ' -f1)"
printf 'AWG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/amneziawg/server.json | cut -d' ' -f1)"
printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'WG_COUNTS=%s/%s/%s\n' \
  "$(printf '%s\n' '{\"op\":\"list\"}' | /usr/local/bin/vpn-ui-manage-wg | python3 -c 'import json,sys;print(json.load(sys.stdin)["count"])')" \
  "$(grep -c '^\[Peer\]' /etc/wireguard/wg0.conf)" \
  "$(wg show wg0 peers | wc -l)"
printf 'AWG_COUNTS=%s/%s/%s\n' \
  "$(printf '%s\n' '{\"op\":\"list\"}' | /usr/local/bin/vpn-ui-manage-awg | python3 -c 'import json,sys;print(json.load(sys.stdin)["count"])')" \
  "$(grep -c '^\[Peer\]' /etc/amnezia/amneziawg/awg0.conf)" \
  "$(awg show awg0 peers | wc -l)"
REMOTE
)"
printf '%s\n' "$BEFORE"

printf '\n%s\n' '===== BUILD ====='
cargo zigbuild --release --target x86_64-unknown-linux-musl
BIN='target/x86_64-unknown-linux-musl/release/vpn-ui'
NEW_UI_SHA="$(sha256sum "$BIN" | cut -d' ' -f1)"
echo "new UI SHA = $NEW_UI_SHA"
[[ "$NEW_UI_SHA" != "$EXPECTED_CURRENT_UI_SHA" ]]

printf '\n%s\n' '===== COPY ====='
scp "$BIN" "$REMOTE:/root/vpn-ui-global-access.new"
scp deploy/nginx/vpn-ui-global.conf "$REMOTE:/root/vpn-ui-global.conf.new"

printf '\n%s\n' '===== DEPLOY ====='
ssh "$REMOTE" bash -s -- "$NEW_UI_SHA" "$EXPECTED_CURRENT_UI_SHA" <<'REMOTE'
set -Eeuo pipefail
NEW_UI_SHA="$1"
EXPECTED_CURRENT_UI_SHA="$2"

UI=/usr/local/bin/vpn-ui
NEW=/root/vpn-ui-global-access.new
SITE=/etc/nginx/sites-available/vpn-ui-5005.conf
NGINX_NEW=/root/vpn-ui-global.conf.new
CERT=/etc/nginx/ssl/vpn-ui.crt
KEY=/etc/nginx/ssl/vpn-ui.key
SOURCE_CERT=/etc/nginx/ssl/awg-web.crt
SOURCE_KEY=/etc/nginx/ssl/awg-web.key
AUTH=/etc/nginx/vpn-ui-admin.htpasswd
STAMP="$(date +%Y%m%d_%H%M%S)"
BACKDIR="/root/vpn-ui-global-access-backup-$STAMP"
UI_BACKUP="/usr/local/bin/vpn-ui.bak-before-global-access-$STAMP"

[[ "$(sha256sum "$UI" | cut -d' ' -f1)" == "$EXPECTED_CURRENT_UI_SHA" ]]
[[ "$(sha256sum "$NEW" | cut -d' ' -f1)" == "$NEW_UI_SHA" ]]
[[ -s "$AUTH" ]]
[[ -s "$SOURCE_CERT" ]]
[[ -s "$SOURCE_KEY" ]]
[[ -f "$SITE" ]]

BUSY="$(ss -H -ltn '( sport = :80 or sport = :443 )' || true)"
[[ -z "$BUSY" ]]

mkdir -p "$BACKDIR"
chmod 700 "$BACKDIR"
cp -a "$SITE" "$BACKDIR/vpn-ui-5005.conf.before"
cp -a "$UI" "$UI_BACKUP"

if [[ -e "$CERT" ]]; then cp -a "$CERT" "$BACKDIR/vpn-ui.crt.before"; fi
if [[ -e "$KEY" ]]; then cp -a "$KEY" "$BACKDIR/vpn-ui.key.before"; fi

rollback() {
  rc=$?
  trap - ERR
  echo "DEPLOY FAILED rc=$rc — ROLLBACK" >&2
  cp -a "$UI_BACKUP" "$UI" || true
  systemctl restart vpn-ui.service || true
  cp -a "$BACKDIR/vpn-ui-5005.conf.before" "$SITE" || true
  if [[ -f "$BACKDIR/vpn-ui.crt.before" ]]; then cp -a "$BACKDIR/vpn-ui.crt.before" "$CERT"; else rm -f "$CERT"; fi
  if [[ -f "$BACKDIR/vpn-ui.key.before" ]]; then cp -a "$BACKDIR/vpn-ui.key.before" "$KEY"; else rm -f "$KEY"; fi
  nginx -t >/dev/null 2>&1 && systemctl reload nginx || true
  exit "$rc"
}
trap rollback ERR

install -o root -g root -m 0755 "$NEW" "$UI"
systemctl restart vpn-ui.service
sleep 1
systemctl is-active --quiet vpn-ui.service
curl -fsS http://127.0.0.1:8090/healthz

ICON_CODE="$(curl -sS -o /tmp/vpn-ui-icon.$$ -w '%{http_code}' http://127.0.0.1:8090/favicon.svg)"
[[ "$ICON_CODE" == 200 ]]
grep -Fq '<svg' /tmp/vpn-ui-icon.$$
rm -f /tmp/vpn-ui-icon.$$

WG_HTML="$(curl -fsS http://127.0.0.1:8090/wireguard)"
[[ "$WG_HTML" == *'/favicon.svg'* ]]
[[ "$WG_HTML" != *'🔐 Управление'* ]]

install -o root -g root -m 0644 "$SOURCE_CERT" "$CERT"
install -o root -g root -m 0600 "$SOURCE_KEY" "$KEY"
install -o root -g root -m 0644 "$NGINX_NEW" "$SITE"
nginx -t
systemctl reload nginx
sleep 1

HTTP="$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1/)"
HTTPS="$(curl -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1/)"
OLD_HTTP="$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:5005/)"
OLD_HTTPS="$(curl -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1:5005/)"

echo "http :80    = $HTTP"
echo "https:443   = $HTTPS"
echo "http :5005  = $OLD_HTTP"
echo "https:5005  = $OLD_HTTPS"

[[ "$HTTP" == 308 ]]
[[ "$HTTPS" == 401 ]]
[[ "$OLD_HTTP" == 308 ]]
[[ "$OLD_HTTPS" == 308 ]]

printf '%s\n' '--- listeners ---'
ss -ltnp | grep -E ':(80|443|5005|8090)[[:space:]]' || true

rm -f "$NEW" "$NGINX_NEW"
trap - ERR

echo "UI backup     = $UI_BACKUP"
echo "config backup = $BACKDIR"
REMOTE

printf '\n%s\n' '===== VERIFY FROM OPi3B ====='
for url in \
  'http://10.77.77.1/' \
  'https://10.77.77.1/' \
  'http://10.77.77.1:5005/' \
  'https://10.77.77.1:5005/'
do
  code="$(curl --connect-timeout 5 -k -sS -o /dev/null -w '%{http_code}' "$url")"
  echo "$url = HTTP $code"
done

[[ "$(curl --connect-timeout 5 -sS -o /dev/null -w '%{http_code}' 'http://10.77.77.1/')" == 308 ]]
[[ "$(curl --connect-timeout 5 -k -sS -o /dev/null -w '%{http_code}' 'https://10.77.77.1/')" == 401 ]]
[[ "$(curl --connect-timeout 5 -sS -o /dev/null -w '%{http_code}' 'http://10.77.77.1:5005/')" == 308 ]]
[[ "$(curl --connect-timeout 5 -k -sS -o /dev/null -w '%{http_code}' 'https://10.77.77.1:5005/')" == 308 ]]

printf '\n%s\n' '===== VERIFY VPN UNCHANGED ====='
AFTER="$(ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
printf 'WG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/wireguard/server.json | cut -d' ' -f1)"
printf 'AWG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/amneziawg/server.json | cut -d' ' -f1)"
printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'WG_COUNTS=%s/%s/%s\n' \
  "$(printf '%s\n' '{\"op\":\"list\"}' | /usr/local/bin/vpn-ui-manage-wg | python3 -c 'import json,sys;print(json.load(sys.stdin)["count"])')" \
  "$(grep -c '^\[Peer\]' /etc/wireguard/wg0.conf)" \
  "$(wg show wg0 peers | wc -l)"
printf 'AWG_COUNTS=%s/%s/%s\n' \
  "$(printf '%s\n' '{\"op\":\"list\"}' | /usr/local/bin/vpn-ui-manage-awg | python3 -c 'import json,sys;print(json.load(sys.stdin)["count"])')" \
  "$(grep -c '^\[Peer\]' /etc/amnezia/amneziawg/awg0.conf)" \
  "$(awg show awg0 peers | wc -l)"
REMOTE
)"
printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
  echo 'ERROR: VPN state changed' >&2
  diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
  exit 1
fi

printf '\n%s\n' '========================================'
printf '%s\n' 'GLOBAL ACCESS DEPLOY = PASS'
printf '%s\n' "UI SHA = $NEW_UI_SHA"
printf '%s\n' 'AUTH = WHOLE UI ON ENTRY'
printf '%s\n' 'HTTP/HTTPS = CANONICAL HTTPS :443'
printf '%s\n' 'OLD :5005 HTTP/HTTPS = REDIRECT'
printf '%s\n' 'ALL INTERFACES = ENABLED'
printf '%s\n' 'FAVICON = ENABLED'
printf '%s\n' 'WG0 / AWG0 NOT RESTARTED'
printf '%s\n' 'VPN STATE UNCHANGED'
printf '%s\n' '========================================'
