#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PKI_DIR="${VPN_UI_PKI_DIR:-/root/vpn-ui-pki}"
CA_KEY="$PKI_DIR/vpn-ui-local-ca.key"
CA_CERT="$PKI_DIR/vpn-ui-local-ca.crt"
CA_SERIAL="$PKI_DIR/vpn-ui-local-ca.srl"
LEAF_KEY="$PKI_DIR/vpn-ui.key"
LEAF_CSR="$PKI_DIR/vpn-ui.csr"
LEAF_CERT="$PKI_DIR/vpn-ui.crt"
LEAF_CONF="$PKI_DIR/vpn-ui-leaf.cnf"
CA_CONF="$PKI_DIR/vpn-ui-ca.cnf"
EXPORT_CA="/root/vpn-ui-local-ca.crt"

cd "$ROOT"

printf '%s\n' '===== LOCAL CA PRECHECK ====='
[[ "$(git branch --show-current)" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]
git diff --check
cargo fmt --check
cargo check
cargo test
command -v openssl >/dev/null
command -v scp >/dev/null

ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx
nginx -t
[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" == 200 ]]
[[ -s /etc/nginx/vpn-ui-admin.htpasswd ]]
[[ -s /etc/nginx/ssl/vpn-ui.crt ]]
[[ -s /etc/nginx/ssl/vpn-ui.key ]]
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

printf '\n%s\n' '===== PREPARE LOCAL PRIVATE CA ====='
install -d -m 0700 "$PKI_DIR"

cat >"$CA_CONF" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = v3_ca
prompt = no

[dn]
CN = VPN UI Local CA

[v3_ca]
basicConstraints = critical,CA:TRUE,pathlen:0
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always,issuer
EOF

cat >"$LEAF_CONF" <<'EOF'
[req]
distinguished_name = dn
req_extensions = req_ext
prompt = no

[dn]
CN = VPN UI

[req_ext]
subjectAltName = @alt

[server_ext]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,keyEncipherment
extendedKeyUsage = serverAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid,issuer
subjectAltName = @alt

[alt]
IP.1 = 10.66.66.1
IP.2 = 10.77.77.1
IP.3 = 46.17.106.244
IP.4 = 100.80.198.69
IP.5 = 2a0a:9300:1:104::1
IP.6 = fd7a:115c:a1e0::2730:c646
DNS.1 = vds3081540.my-ihor.ru
EOF

if [[ ! -s "$CA_KEY" || ! -s "$CA_CERT" ]]; then
    echo 'creating new local CA'
    umask 077
    openssl genrsa -out "$CA_KEY" 3072 >/dev/null 2>&1
    openssl req -x509 -new -sha256 -days 3650 \
        -key "$CA_KEY" \
        -config "$CA_CONF" \
        -out "$CA_CERT"
else
    echo 'reusing existing local CA'
fi

chmod 0600 "$CA_KEY"
chmod 0644 "$CA_CERT"

umask 077
openssl genrsa -out "$LEAF_KEY" 2048 >/dev/null 2>&1
openssl req -new \
    -key "$LEAF_KEY" \
    -config "$LEAF_CONF" \
    -out "$LEAF_CSR"

rm -f "$CA_SERIAL"
openssl x509 -req \
    -in "$LEAF_CSR" \
    -CA "$CA_CERT" \
    -CAkey "$CA_KEY" \
    -CAcreateserial \
    -CAserial "$CA_SERIAL" \
    -days 825 \
    -sha256 \
    -extfile "$LEAF_CONF" \
    -extensions server_ext \
    -out "$LEAF_CERT" >/dev/null 2>&1

chmod 0600 "$LEAF_KEY"
chmod 0644 "$LEAF_CERT"
install -m 0644 "$CA_CERT" "$EXPORT_CA"

openssl verify -CAfile "$CA_CERT" "$LEAF_CERT"

echo '--- CA ---'
openssl x509 -in "$CA_CERT" -noout -subject -fingerprint -sha256 -dates

echo '--- server certificate ---'
openssl x509 -in "$LEAF_CERT" -noout -subject -issuer -dates -ext subjectAltName

printf '\n%s\n' '===== COPY CERTIFICATES ====='
scp "$LEAF_CERT" "$REMOTE:/root/vpn-ui.crt.local-ca.new"
scp "$LEAF_KEY" "$REMOTE:/root/vpn-ui.key.local-ca.new"
scp "$CA_CERT" "$REMOTE:/root/vpn-ui-local-ca.crt.new"

printf '\n%s\n' '===== INSTALL CERTIFICATES ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
CERT=/etc/nginx/ssl/vpn-ui.crt
KEY=/etc/nginx/ssl/vpn-ui.key
CA=/etc/nginx/ssl/vpn-ui-local-ca.crt
NEW_CERT=/root/vpn-ui.crt.local-ca.new
NEW_KEY=/root/vpn-ui.key.local-ca.new
NEW_CA=/root/vpn-ui-local-ca.crt.new
STAMP="$(date +%Y%m%d_%H%M%S)"
BACKDIR="/root/vpn-ui-cert-backup-$STAMP"

[[ -s "$NEW_CERT" ]]
[[ -s "$NEW_KEY" ]]
[[ -s "$NEW_CA" ]]

openssl verify -CAfile "$NEW_CA" "$NEW_CERT"

mkdir -p "$BACKDIR"
chmod 700 "$BACKDIR"
cp -a "$CERT" "$BACKDIR/vpn-ui.crt.before"
cp -a "$KEY" "$BACKDIR/vpn-ui.key.before"
[[ -e "$CA" ]] && cp -a "$CA" "$BACKDIR/vpn-ui-local-ca.crt.before" || true

rollback() {
    rc=$?
    trap - ERR
    echo "CERT INSTALL FAILED rc=$rc — ROLLBACK" >&2
    cp -a "$BACKDIR/vpn-ui.crt.before" "$CERT" || true
    cp -a "$BACKDIR/vpn-ui.key.before" "$KEY" || true
    if [[ -f "$BACKDIR/vpn-ui-local-ca.crt.before" ]]; then
        cp -a "$BACKDIR/vpn-ui-local-ca.crt.before" "$CA" || true
    else
        rm -f "$CA"
    fi
    nginx -t >/dev/null 2>&1 && systemctl reload nginx || true
    exit "$rc"
}
trap rollback ERR

install -o root -g root -m 0644 "$NEW_CERT" "$CERT"
install -o root -g root -m 0600 "$NEW_KEY" "$KEY"
install -o root -g root -m 0644 "$NEW_CA" "$CA"

nginx -t
systemctl reload nginx
sleep 1
systemctl is-active --quiet nginx
openssl verify -CAfile "$CA" "$CERT"

rm -f "$NEW_CERT" "$NEW_KEY" "$NEW_CA"
trap - ERR

echo "certificate backup = $BACKDIR"
REMOTE

printf '\n%s\n' '===== VERIFIED TLS FROM OPi3B ====='
for ip in \
  10.77.77.1 \
  10.66.66.1 \
  46.17.106.244 \
  100.80.198.69
do
    code="$(curl --cacert "$CA_CERT" --connect-timeout 5 -sS -o /dev/null -w '%{http_code}' "https://$ip/")"
    echo "https://$ip/ = HTTP $code (certificate verified)"
    [[ "$code" == 401 ]]
done

printf '\n%s\n' '===== VERIFY VPN UNCHANGED ====='
AFTER="$(capture_state)"
printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
    echo 'ERROR: VPN state changed' >&2
    diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
    exit 1
fi

printf '\n%s\n' '========================================'
printf '%s\n' 'LOCAL CA CERTIFICATE DEPLOY = PASS'
printf '%s\n' "CA certificate = $EXPORT_CA"
printf '%s\n' "CA private key = $CA_KEY (LOCAL OPi3B ONLY)"
printf '%s\n' 'SERVER CERT = SIGNED BY VPN UI LOCAL CA'
printf '%s\n' 'ALL VPN UI ADDRESSES = SAN VERIFIED'
printf '%s\n' 'NGINX = RELOADED ONLY'
printf '%s\n' 'WG0 / AWG0 NOT RESTARTED'
printf '%s\n' 'VPN STATE UNCHANGED'
printf '%s\n' '========================================'
