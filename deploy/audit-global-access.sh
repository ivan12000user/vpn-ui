#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"

printf '%s\n' '===== VPN UI GLOBAL ACCESS AUDIT ====='

echo '===== LOCAL BRANCH ====='
git branch --show-current
git status --short

echo
echo '===== REMOTE ADDRESSES ====='
ssh "$REMOTE" 'ip -br addr'

echo
echo '===== REMOTE ROUTES ====='
ssh "$REMOTE" 'ip route; ip -6 route 2>/dev/null || true'

echo
echo '===== TCP LISTENERS 80 / 443 / 5005 / 8090 ====='
ssh "$REMOTE" "ss -ltnp | awk 'NR==1 || /:80[[:space:]]|:443[[:space:]]|:5005[[:space:]]|:8090[[:space:]]/'"

echo
echo '===== NGINX VERSION / TEST ====='
ssh "$REMOTE" 'nginx -v; nginx -t'

echo
echo '===== VPN UI NGINX FILES ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set -Eeuo pipefail

mapfile -t files < <(
    grep -RIlE '127\.0\.0\.1:8090|10\.77\.77\.1:5005|10\.66\.66\.1:5005|vpn-ui-admin\.htpasswd' \
        /etc/nginx 2>/dev/null || true
)

printf 'matches = %s\n' "${#files[@]}"
printf '%s\n' "${files[@]:-}"

for file in "${files[@]}"; do
    echo
    echo "--- $file ---"
    grep -nE 'listen|server_name|auth_basic|auth_basic_user_file|proxy_pass|ssl_certificate|return [0-9]{3}' "$file" || true
done
REMOTE_SCRIPT

echo
echo '===== EFFECTIVE NGINX RELEVANT LINES ====='
ssh "$REMOTE" 'nginx -T 2>&1' |
grep -E '^# configuration file|listen |server_name|auth_basic|auth_basic_user_file|proxy_pass http://127\.0\.0\.1:8090|ssl_certificate|return 30[1278]' || true

echo
echo '===== AUTH FILE ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set -Eeuo pipefail

AUTH=/etc/nginx/vpn-ui-admin.htpasswd

if [[ -s "$AUTH" ]]; then
    stat -c 'path=%n mode=%a owner=%U:%G size=%s' "$AUTH"
    printf 'users='
    cut -d: -f1 "$AUTH" | paste -sd, -
else
    echo 'AUTH FILE MISSING OR EMPTY'
fi
REMOTE_SCRIPT

echo
echo '===== CERTIFICATES / ACME ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set +e
command -v certbot || true
command -v acme.sh || true
command -v openssl || true

find /etc/letsencrypt /root/.acme.sh /etc/nginx \
    -maxdepth 4 -type f \
    \( -name 'fullchain.pem' -o -name '*.crt' -o -name '*.cer' \) \
    -print 2>/dev/null | head -50
REMOTE_SCRIPT

echo
echo '===== FIREWALL ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set +e

if command -v ufw >/dev/null 2>&1; then
    ufw status verbose
fi

if command -v nft >/dev/null 2>&1; then
    nft list ruleset 2>/dev/null |
        grep -E '(^table|hook input|policy |tcp dport.*(80|443|5005)|dport \{[^}]*(80|443|5005))' |
        head -120
fi
REMOTE_SCRIPT

echo
echo '===== CURRENT HTTP BEHAVIOUR ====='
for URL in \
    'http://10.77.77.1:5005/' \
    'http://10.66.66.1:5005/'
do
    printf '%-36s ' "$URL"
    curl -sS -o /dev/null -w 'HTTP %{http_code} redirect=%{redirect_url}\n' "$URL" || true
done

echo
echo '===== DIRECT APP HEALTH ====='
ssh "$REMOTE" 'curl -fsS http://127.0.0.1:8090/healthz; echo'

echo
echo '===== VPN STATE SAFETY BASELINE ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set -Eeuo pipefail
printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'WG_COUNT=%s\n' "$(wg show wg0 peers | wc -l)"
printf 'AWG_COUNT=%s\n' "$(awg show awg0 peers | wc -l)"
REMOTE_SCRIPT

echo
echo '========================================'
echo 'AUDIT ONLY = PASS'
echo 'NO NGINX CHANGES'
echo 'NO FIREWALL CHANGES'
echo 'NO VPN CHANGES'
echo '========================================'
