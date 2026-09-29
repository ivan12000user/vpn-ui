#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
EXPECTED_UI_SHA='c392c0efd818115409fe553ee2221ef6fe361a3bf7599d9d4263b0281f1c6682'
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf '%s\n' '===== GLOBAL ACCESS REPAIR PRECHECK ====='
[[ "$(git branch --show-current)" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]
git diff --check
cargo fmt --check
cargo check
cargo test

REMOTE_UI_SHA="$(ssh "$REMOTE" "sha256sum /usr/local/bin/vpn-ui | cut -d' ' -f1")"
echo "production UI SHA = $REMOTE_UI_SHA"
[[ "$REMOTE_UI_SHA" == "$EXPECTED_UI_SHA" ]]

ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx
nginx -t

[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" == 200 ]]
[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/favicon.svg)" == 200 ]]

WG_HTML="$(curl -fsS http://127.0.0.1:8090/wireguard)"
[[ "$WG_HTML" == *'/favicon.svg'* ]]
[[ "$WG_HTML" != *'🔐 Управление'* ]]
REMOTE

capture_state() {
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
count_inventory() {
    local helper="$1"
    printf '%s\n' '{"op":"list"}' |
    "$helper" |
    python3 -c '
import json,sys
d=json.load(sys.stdin)
peers=d.get("peers", [])
print(d.get("count", len(peers)))
'
}

printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
printf 'WG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/wireguard/server.json | cut -d' ' -f1)"
printf 'AWG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/amneziawg/server.json | cut -d' ' -f1)"
printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'WG_COUNTS=%s/%s/%s\n' \
  "$(count_inventory /usr/local/bin/vpn-ui-manage-wg)" \
  "$(grep -c '^\[Peer\]' /etc/wireguard/wg0.conf)" \
  "$(wg show wg0 peers | wc -l)"
printf 'AWG_COUNTS=%s/%s/%s\n' \
  "$(count_inventory /usr/local/bin/vpn-ui-manage-awg)" \
  "$(grep -c '^\[Peer\]' /etc/amnezia/amneziawg/awg0.conf)" \
  "$(awg show awg0 peers | wc -l)"
REMOTE
}

printf '\n%s\n' '===== CAPTURE VPN BASELINE ====='
BEFORE="$(capture_state)"
printf '%s\n' "$BEFORE"

printf '\n%s\n' '===== COPY NGINX CONFIG ====='
scp deploy/nginx/vpn-ui-global.conf "$REMOTE:/root/vpn-ui-global.conf.repair"

printf '\n%s\n' '===== APPLY NGINX GLOBAL ACCESS ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

SITE=/etc/nginx/sites-available/vpn-ui-5005.conf
NEW=/root/vpn-ui-global.conf.repair
CERT=/etc/nginx/ssl/vpn-ui.crt
KEY=/etc/nginx/ssl/vpn-ui.key
AUTH=/etc/nginx/vpn-ui-admin.htpasswd
STAMP="$(date +%Y%m%d_%H%M%S)"
BACKDIR="/root/vpn-ui-global-repair-backup-$STAMP"

[[ -f "$SITE" ]]
[[ -s "$NEW" ]]
[[ -s "$AUTH" ]]

mkdir -p "$BACKDIR"
chmod 700 "$BACKDIR"
cp -a "$SITE" "$BACKDIR/vpn-ui-5005.conf.before"
[[ -e "$CERT" ]] && cp -a "$CERT" "$BACKDIR/vpn-ui.crt.before" || true
[[ -e "$KEY" ]] && cp -a "$KEY" "$BACKDIR/vpn-ui.key.before" || true

rollback() {
    rc=$?
    trap - ERR
    echo "NGINX REPAIR FAILED rc=$rc — ROLLBACK" >&2
    cp -a "$BACKDIR/vpn-ui-5005.conf.before" "$SITE" || true

    if [[ -f "$BACKDIR/vpn-ui.crt.before" ]]; then
        cp -a "$BACKDIR/vpn-ui.crt.before" "$CERT" || true
    else
        rm -f "$CERT"
    fi

    if [[ -f "$BACKDIR/vpn-ui.key.before" ]]; then
        cp -a "$BACKDIR/vpn-ui.key.before" "$KEY" || true
    else
        rm -f "$KEY"
    fi

    nginx -t >/dev/null 2>&1 && systemctl restart nginx || true
    exit "$rc"
}
trap rollback ERR

if [[ ! -s "$CERT" || ! -s "$KEY" ]]; then
    TMPDIR="$(mktemp -d)"
    cat >"$TMPDIR/openssl.cnf" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = v3
prompt = no

[dn]
CN = VPN UI

[v3]
subjectAltName = @alt
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,keyEncipherment
extendedKeyUsage = serverAuth

[alt]
IP.1 = 10.66.66.1
IP.2 = 10.77.77.1
IP.3 = 46.17.106.244
IP.4 = 100.80.198.69
IP.5 = 2a0a:9300:1:104::1
IP.6 = fd7a:115c:a1e0::2730:c646
DNS.1 = vds3081540.my-ihor.ru
EOF

    openssl req -x509 -nodes -newkey rsa:2048 -sha256 -days 3650 \
        -config "$TMPDIR/openssl.cnf" \
        -keyout "$TMPDIR/vpn-ui.key" \
        -out "$TMPDIR/vpn-ui.crt" >/dev/null 2>&1

    install -o root -g root -m 0600 "$TMPDIR/vpn-ui.key" "$KEY"
    install -o root -g root -m 0644 "$TMPDIR/vpn-ui.crt" "$CERT"
    rm -rf "$TMPDIR"
fi

install -o root -g root -m 0644 "$NEW" "$SITE"
nginx -t
systemctl restart nginx
sleep 1
systemctl is-active --quiet nginx

HTTP="$(curl --connect-timeout 3 -sS -o /dev/null -w '%{http_code}' http://127.0.0.1/)"
HTTPS="$(curl --connect-timeout 3 -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1/)"
OLD_HTTP="$(curl --connect-timeout 3 -sS -o /dev/null -w '%{http_code}' http://10.77.77.1:5005/)"

echo "http :80           = $HTTP"
echo "https:443          = $HTTPS"
echo "old http :5005     = $OLD_HTTP"

[[ "$HTTP" == 308 ]]
[[ "$HTTPS" == 401 ]]
[[ "$OLD_HTTP" == 308 ]]

printf '%s\n' '--- listeners ---'
ss -ltnp | grep -E ':(80|443|5005|8090)[[:space:]]' || true

rm -f "$NEW"
trap - ERR

echo "config backup = $BACKDIR"
REMOTE

printf '\n%s\n' '===== VERIFY FROM OPi3B ====='
for url in \
  'http://10.77.77.1/' \
  'https://10.77.77.1/' \
  'http://10.77.77.1:5005/'
do
    code="$(curl --connect-timeout 5 -k -sS -o /dev/null -w '%{http_code}' "$url")"
    echo "$url = HTTP $code"
done

[[ "$(curl --connect-timeout 5 -sS -o /dev/null -w '%{http_code}' 'http://10.77.77.1/')" == 308 ]]
[[ "$(curl --connect-timeout 5 -k -sS -o /dev/null -w '%{http_code}' 'https://10.77.77.1/')" == 401 ]]
[[ "$(curl --connect-timeout 5 -sS -o /dev/null -w '%{http_code}' 'http://10.77.77.1:5005/')" == 308 ]]

printf '\n%s\n' '===== VERIFY VPN UNCHANGED ====='
AFTER="$(capture_state)"
printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
    echo 'ERROR: VPN state changed' >&2
    diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
    exit 1
fi

printf '\n%s\n' '========================================'
printf '%s\n' 'GLOBAL ACCESS REPAIR = PASS'
printf '%s\n' 'AUTH = WHOLE UI ON HTTPS ENTRY'
printf '%s\n' 'HTTP :80 = REDIRECT TO HTTPS'
printf '%s\n' 'HTTPS :443 = ALL INTERFACES'
printf '%s\n' 'OLD HTTP :5005 = REDIRECT'
printf '%s\n' 'DEDICATED VPN-UI CERT = ENABLED'
printf '%s\n' 'FAVICON = ENABLED'
printf '%s\n' 'WG0 / AWG0 NOT RESTARTED'
printf '%s\n' 'VPN STATE UNCHANGED'
printf '%s\n' '========================================'
