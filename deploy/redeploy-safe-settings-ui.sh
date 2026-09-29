#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
EXPECTED_CURRENT_UI_SHA='ed437bdcf636b6f3c758e83898f2ab5d9265e50cfbc7b84ed821097305364dad'
EXPECTED_WRAPPER_SHA='f255db92d2e45e66543e66951cc394182c1f5691c04b7ac1b6ce97b538a18dea'
EXPECTED_CORE_SHA='c82febd069990da039611932517c7f501496953fe8249c065b141a8c99bb792a'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf '%s\n' '===== SETTINGS UI CLEANUP PRECHECK ====='

BRANCH="$(git branch --show-current)"
echo "branch = $BRANCH"
[[ "$BRANCH" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]

git diff --check
cargo fmt --check
cargo check
cargo test

REMOTE_UI_SHA="$(ssh "$REMOTE" "sha256sum /usr/local/bin/vpn-ui | cut -d' ' -f1")"
REMOTE_WRAPPER_SHA="$(ssh "$REMOTE" "sha256sum /usr/local/libexec/vpn-ui-manage | cut -d' ' -f1")"
REMOTE_CORE_SHA="$(ssh "$REMOTE" "sha256sum /usr/local/libexec/vpn-ui-manage-core-v070 | cut -d' ' -f1")"

echo "production UI SHA = $REMOTE_UI_SHA"
echo "wrapper SHA       = $REMOTE_WRAPPER_SHA"
echo "core SHA          = $REMOTE_CORE_SHA"

[[ "$REMOTE_UI_SHA" == "$EXPECTED_CURRENT_UI_SHA" ]]
[[ "$REMOTE_WRAPPER_SHA" == "$EXPECTED_WRAPPER_SHA" ]]
[[ "$REMOTE_CORE_SHA" == "$EXPECTED_CORE_SHA" ]]

printf '\n%s\n' '===== CAPTURE PRODUCTION BASELINE ====='

BEFORE="$(ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set -Eeuo pipefail

printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
printf 'WG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/wireguard/server.json | cut -d' ' -f1)"
printf 'AWG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/amneziawg/server.json | cut -d' ' -f1)"
printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'WG_COUNT=%s\n' "$(wg show wg0 peers | wc -l)"
printf 'AWG_COUNT=%s\n' "$(awg show awg0 peers | wc -l)"
REMOTE_SCRIPT
)"

printf '%s\n' "$BEFORE"

printf '\n%s\n' '===== VERIFY SAVED SAFE SETTINGS ====='

ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set -Eeuo pipefail

printf '%s\n' '{"op":"settings_get"}' |
/usr/local/bin/vpn-ui-manage-wg |
python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d["ok"] is True
s=d["settings"]
print("WG DNS =", ", ".join(s["dns_servers"]))
print("WG Client AllowedIPs =", ", ".join(s["client_allowed_ips"]))
'

printf '%s\n' '{"op":"settings_get"}' |
/usr/local/bin/vpn-ui-manage-awg |
python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d["ok"] is True
s=d["settings"]
print("AWG DNS =", ", ".join(s["dns_servers"]))
print("AWG Client AllowedIPs =", ", ".join(s["client_allowed_ips"]))
'
REMOTE_SCRIPT

printf '\n%s\n' '===== BUILD CLEANED UI ====='

cargo zigbuild --release --target x86_64-unknown-linux-musl

BIN='target/x86_64-unknown-linux-musl/release/vpn-ui'
NEW_UI_SHA="$(sha256sum "$BIN" | cut -d' ' -f1)"

echo "old UI SHA = $EXPECTED_CURRENT_UI_SHA"
echo "new UI SHA = $NEW_UI_SHA"
[[ "$NEW_UI_SHA" != "$EXPECTED_CURRENT_UI_SHA" ]]

printf '\n%s\n' '===== COPY ====='
scp "$BIN" "$REMOTE:/root/vpn-ui-settings-cleanup.new"

printf '\n%s\n' '===== DEPLOY VPN-UI ONLY ====='

ssh "$REMOTE" bash -s -- "$NEW_UI_SHA" "$EXPECTED_CURRENT_UI_SHA" <<'REMOTE_SCRIPT'
set -Eeuo pipefail

NEW_UI_SHA="$1"
EXPECTED_CURRENT_UI_SHA="$2"

UI=/usr/local/bin/vpn-ui
NEW=/root/vpn-ui-settings-cleanup.new
STAMP="$(date +%Y%m%d_%H%M%S)"
BACKUP="/usr/local/bin/vpn-ui.bak-before-settings-cleanup-$STAMP"

[[ "$(sha256sum "$UI" | cut -d' ' -f1)" == "$EXPECTED_CURRENT_UI_SHA" ]]
[[ "$(sha256sum "$NEW" | cut -d' ' -f1)" == "$NEW_UI_SHA" ]]

cp -a "$UI" "$BACKUP"

rollback() {
    rc=$?
    trap - ERR
    echo "UI DEPLOY FAILED rc=$rc — ROLLBACK"
    cp -a "$BACKUP" "$UI" || true
    systemctl restart vpn-ui.service || true
    exit "$rc"
}
trap rollback ERR

install -o root -g root -m 0755 "$NEW" "$UI"
systemctl restart vpn-ui.service
sleep 1
systemctl is-active --quiet vpn-ui.service

HEALTH="$(curl -fsS http://127.0.0.1:8090/healthz)"
echo "health = $HEALTH"

rm -f "$NEW"
trap - ERR

echo "UI backup = $BACKUP"
REMOTE_SCRIPT

printf '\n%s\n' '===== VERIFY WEB NAVIGATION ====='

CODE="$(curl -sS -o /dev/null -w '%{http_code}' 'http://10.77.77.1:5005/settings')"
echo "/settings = HTTP $CODE"
[[ "$CODE" == 307 ]]

CODE="$(curl -sS -L -o /dev/null -w '%{http_code}' 'http://10.77.77.1:5005/settings')"
echo "/settings -> /settings/manage = HTTP $CODE"
[[ "$CODE" == 200 ]]

for URL in \
    'http://10.77.77.1:5005/settings/manage' \
    'http://10.77.77.1:5005/settings/interfaces' \
    'http://10.77.77.1:5005/wireguard' \
    'http://10.77.77.1:5005/amneziawg'
do
    CODE="$(curl -sS -o /dev/null -w '%{http_code}' "$URL")"
    echo "$URL = HTTP $CODE"
    [[ "$CODE" == 200 ]]
done

printf '\n%s\n' '===== VERIFY PRODUCTION UNCHANGED ====='

AFTER="$(ssh "$REMOTE" 'bash -s' <<'REMOTE_SCRIPT'
set -Eeuo pipefail

printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
printf 'WG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/wireguard/server.json | cut -d' ' -f1)"
printf 'AWG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/amneziawg/server.json | cut -d' ' -f1)"
printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'WG_COUNT=%s\n' "$(wg show wg0 peers | wc -l)"
printf 'AWG_COUNT=%s\n' "$(awg show awg0 peers | wc -l)"
REMOTE_SCRIPT
)"

printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
    echo 'ERROR: production VPN/settings state changed during UI redeploy' >&2
    diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
    exit 1
fi

printf '\n%s\n' '========================================'
printf '%s\n' 'SETTINGS UI CLEANUP DEPLOY = PASS'
printf '%s\n' "UI SHA = $NEW_UI_SHA"
printf '%s\n' 'SIDEBAR SETTINGS -> EDITABLE DEFAULTS'
printf '%s\n' 'INTERFACE SETTINGS = SEPARATE READ-ONLY PAGE'
printf '%s\n' 'AUTH BUTTON = "Войти для изменений"'
printf '%s\n' 'WG0 / AWG0 NOT RESTARTED'
printf '%s\n' 'VPN CONFIGS AND SAVED DEFAULTS UNCHANGED'
printf '%s\n' '========================================'
