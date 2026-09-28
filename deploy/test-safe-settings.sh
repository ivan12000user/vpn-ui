#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
EXPECTED_UI_SHA='4401edae15ff19ee809217e6e414b310f7c127c2cf7728489cebf1f6ee32c462'
EXPECTED_HELPER_SHA='c82febd069990da039611932517c7f501496953fe8249c065b141a8c99bb792a'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf '%s\n' '===== SAFE SETTINGS PROTOTYPE PRECHECK ====='

BRANCH="$(git branch --show-current)"
echo "branch = $BRANCH"

if [[ "$BRANCH" != 'feature/editable-safe-settings' ]]; then
    echo 'ERROR: checkout feature/editable-safe-settings first' >&2
    exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
    echo 'ERROR: worktree is not clean' >&2
    git status --short >&2
    exit 1
fi

git diff --check
cargo fmt --check
cargo check
cargo test

REMOTE_UI_SHA="$(
    ssh "$REMOTE" "sha256sum /usr/local/bin/vpn-ui | cut -d' ' -f1"
)"

REMOTE_HELPER_SHA="$(
    ssh "$REMOTE" "sha256sum /usr/local/libexec/vpn-ui-manage | cut -d' ' -f1"
)"

echo "production UI SHA     = $REMOTE_UI_SHA"
echo "production helper SHA = $REMOTE_HELPER_SHA"

[[ "$REMOTE_UI_SHA" == "$EXPECTED_UI_SHA" ]]
[[ "$REMOTE_HELPER_SHA" == "$EXPECTED_HELPER_SHA" ]]

printf '\n%s\n' '===== BUILD PROTOTYPE ====='

cargo zigbuild --release --target x86_64-unknown-linux-musl

BIN='target/x86_64-unknown-linux-musl/release/vpn-ui'
WRAPPER='deploy/libexec/vpn-ui-manage-settings-wrapper'

NEW_UI_SHA="$(sha256sum "$BIN" | cut -d' ' -f1)"
WRAPPER_SHA="$(sha256sum "$WRAPPER" | cut -d' ' -f1)"

echo "prototype UI SHA = $NEW_UI_SHA"
echo "wrapper SHA      = $WRAPPER_SHA"

[[ "$NEW_UI_SHA" != "$EXPECTED_UI_SHA" ]]

printf '\n%s\n' '===== COPY STAGED FILES ====='

scp "$BIN" "$REMOTE:/root/vpn-ui-safe-settings.new"
scp "$WRAPPER" "$REMOTE:/root/vpn-ui-manage-settings-wrapper.new"

printf '\n%s\n' '===== DEPLOY PROTOTYPE ====='

ssh "$REMOTE" bash -s -- \
    "$NEW_UI_SHA" \
    "$WRAPPER_SHA" \
    "$EXPECTED_UI_SHA" \
    "$EXPECTED_HELPER_SHA" <<'REMOTE_SCRIPT'
set -Eeuo pipefail

NEW_UI_SHA="$1"
WRAPPER_SHA="$2"
EXPECTED_UI_SHA="$3"
EXPECTED_HELPER_SHA="$4"

UI=/usr/local/bin/vpn-ui
HELPER=/usr/local/libexec/vpn-ui-manage
CORE=/usr/local/libexec/vpn-ui-manage-core-v070
NEW_UI=/root/vpn-ui-safe-settings.new
NEW_WRAPPER=/root/vpn-ui-manage-settings-wrapper.new
STAMP="$(date +%Y%m%d_%H%M%S)"
UI_BACKUP="/usr/local/bin/vpn-ui.bak-before-safe-settings-$STAMP"
HELPER_BACKUP="/usr/local/libexec/vpn-ui-manage.bak-before-safe-settings-$STAMP"

[[ "$(sha256sum "$UI" | cut -d' ' -f1)" == "$EXPECTED_UI_SHA" ]]
[[ "$(sha256sum "$HELPER" | cut -d' ' -f1)" == "$EXPECTED_HELPER_SHA" ]]
[[ "$(sha256sum "$NEW_UI" | cut -d' ' -f1)" == "$NEW_UI_SHA" ]]
[[ "$(sha256sum "$NEW_WRAPPER" | cut -d' ' -f1)" == "$WRAPPER_SHA" ]]

WG_TS_BEFORE="$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
AWG_TS_BEFORE="$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
WG_HASH_BEFORE="$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
AWG_HASH_BEFORE="$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
WG_LIVE_BEFORE="$(wg show wg0 peers | wc -l)"
AWG_LIVE_BEFORE="$(awg show awg0 peers | wc -l)"

cp -a "$UI" "$UI_BACKUP"
cp -a "$HELPER" "$HELPER_BACKUP"

rollback() {
    rc=$?
    trap - ERR
    echo "DEPLOY FAILED rc=$rc — ROLLBACK"
    cp -a "$HELPER_BACKUP" "$HELPER" || true
    cp -a "$UI_BACKUP" "$UI" || true
    systemctl restart vpn-ui.service || true
    exit "$rc"
}
trap rollback ERR

if [[ -e "$CORE" ]]; then
    [[ "$(sha256sum "$CORE" | cut -d' ' -f1)" == "$EXPECTED_HELPER_SHA" ]]
else
    install -o root -g root -m 0755 "$HELPER" "$CORE"
fi

install -o root -g root -m 0755 "$NEW_WRAPPER" "$HELPER"
install -o root -g root -m 0755 "$NEW_UI" "$UI"

systemctl restart vpn-ui.service
sleep 1
systemctl is-active --quiet vpn-ui.service

HEALTH="$(curl -fsS http://127.0.0.1:8090/healthz)"
echo "health = $HEALTH"

printf '%s\n' '{"op":"list"}' | /usr/local/bin/vpn-ui-manage-wg >/tmp/vpn-ui-wg-list.json
printf '%s\n' '{"op":"list"}' | /usr/local/bin/vpn-ui-manage-awg >/tmp/vpn-ui-awg-list.json
printf '%s\n' '{"op":"defaults"}' | /usr/local/bin/vpn-ui-manage-wg >/tmp/vpn-ui-wg-defaults.json
printf '%s\n' '{"op":"defaults"}' | /usr/local/bin/vpn-ui-manage-awg >/tmp/vpn-ui-awg-defaults.json
printf '%s\n' '{"op":"settings_get"}' | /usr/local/bin/vpn-ui-manage-wg >/tmp/vpn-ui-wg-settings.json
printf '%s\n' '{"op":"settings_get"}' | /usr/local/bin/vpn-ui-manage-awg >/tmp/vpn-ui-awg-settings.json

python3 - <<'PY'
import json
from pathlib import Path

checks = {
    'wg-list': '/tmp/vpn-ui-wg-list.json',
    'awg-list': '/tmp/vpn-ui-awg-list.json',
    'wg-defaults': '/tmp/vpn-ui-wg-defaults.json',
    'awg-defaults': '/tmp/vpn-ui-awg-defaults.json',
    'wg-settings': '/tmp/vpn-ui-wg-settings.json',
    'awg-settings': '/tmp/vpn-ui-awg-settings.json',
}

for label, path in checks.items():
    data = json.loads(Path(path).read_text())
    assert data.get('ok') is True, (label, data)
    print(label, '= OK')

wg = json.loads(Path(checks['wg-settings']).read_text())['settings']
awg = json.loads(Path(checks['awg-settings']).read_text())['settings']

assert wg['client_allowed_ips'] == ['10.66.66.0/24']
assert awg['client_allowed_ips'] == ['10.77.77.0/24']
print('safe defaults = OK')
PY

WG_TS_AFTER="$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
AWG_TS_AFTER="$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
WG_HASH_AFTER="$(sha256sum /etc/wireguard/wg0.conf | cut -d' ' -f1)"
AWG_HASH_AFTER="$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d' ' -f1)"
WG_LIVE_AFTER="$(wg show wg0 peers | wc -l)"
AWG_LIVE_AFTER="$(awg show awg0 peers | wc -l)"

[[ "$WG_TS_AFTER" == "$WG_TS_BEFORE" ]]
[[ "$AWG_TS_AFTER" == "$AWG_TS_BEFORE" ]]
[[ "$WG_HASH_AFTER" == "$WG_HASH_BEFORE" ]]
[[ "$AWG_HASH_AFTER" == "$AWG_HASH_BEFORE" ]]
[[ "$WG_LIVE_AFTER" == "$WG_LIVE_BEFORE" ]]
[[ "$AWG_LIVE_AFTER" == "$AWG_LIVE_BEFORE" ]]

rm -f "$NEW_UI" "$NEW_WRAPPER" \
    /tmp/vpn-ui-wg-list.json /tmp/vpn-ui-awg-list.json \
    /tmp/vpn-ui-wg-defaults.json /tmp/vpn-ui-awg-defaults.json \
    /tmp/vpn-ui-wg-settings.json /tmp/vpn-ui-awg-settings.json

trap - ERR

echo "UI backup     = $UI_BACKUP"
echo "helper backup = $HELPER_BACKUP"
echo "core          = $CORE"
echo "WG timestamp  = $WG_TS_AFTER"
echo "AWG timestamp = $AWG_TS_AFTER"
echo "WG config SHA = $WG_HASH_AFTER"
echo "AWG config SHA= $AWG_HASH_AFTER"
REMOTE_SCRIPT

printf '\n%s\n' '===== VERIFY WEB ====='

for URL in \
    'http://10.77.77.1:5005/wireguard' \
    'http://10.77.77.1:5005/amneziawg' \
    'http://10.77.77.1:5005/settings/manage'
do
    CODE="$(curl -sS -o /dev/null -w '%{http_code}' "$URL")"
    echo "$URL = HTTP $CODE"
    [[ "$CODE" == 200 ]]
done

for URL in \
    'http://10.77.77.1:5005/api/admin/wireguard/manage' \
    'http://10.77.77.1:5005/api/admin/amneziawg/manage'
do
    CODE="$(
        curl -sS -o /dev/null -w '%{http_code}' \
            -X POST \
            -H 'Content-Type: application/json' \
            -d '{"op":"settings_get"}' \
            "$URL"
    )"
    echo "$URL unauth = HTTP $CODE"
    [[ "$CODE" == 401 ]]
done

printf '\n%s\n' '========================================'
printf '%s\n' 'SAFE SETTINGS PROTOTYPE DEPLOY = PASS'
printf '%s\n' "UI SHA = $NEW_UI_SHA"
printf '%s\n' 'PAGE = /settings/manage'
printf '%s\n' 'WG0 / AWG0 NOT RESTARTED'
printf '%s\n' 'VPN CONFIGS UNCHANGED'
printf '%s\n' 'NO SETTINGS CHANGED YET'
printf '%s\n' '========================================'
