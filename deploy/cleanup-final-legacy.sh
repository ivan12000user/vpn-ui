#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo '===== FINAL LEGACY CLEANUP ====='
[[ "$(git branch --show-current)" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]
git diff --check

ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

STAMP="$(date +%Y%m%d_%H%M%S)"
BACKUP="/root/vpn-ui-final-legacy-backup-$STAMP.tar.gz"
MANIFEST="/root/vpn-ui-final-legacy-backup-$STAMP.manifest.txt"

WG_CONF=/etc/wireguard/wg0.conf
AWG_CONF=/etc/amnezia/amneziawg/awg0.conf

LEGACY_UNITS=(
  wgui.path
  wgui.service
  wireguard-ui-daemon.service
)

LEGACY_UNIT_FILES=(
  /etc/systemd/system/wgui.path
  /etc/systemd/system/wgui.service
  /etc/systemd/system/wireguard-ui-daemon.service
)

LEGACY_WG_FILES=(
  /etc/wireguard/wireguard-ui
  /etc/wireguard/.env
)

LEGACY_NGINX_FILES=(
  /etc/nginx/sites-available/wireguard-ui-5000.conf
  /etc/nginx/sites-enabled/wireguard-ui-5000.conf
)

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

client_tree_sha() {
    if [[ ! -d /etc/amnezia/amneziawg/clients ]]; then
        printf '%s\n' MISSING
        return
    fi

    find /etc/amnezia/amneziawg/clients -type f -print0 |
      sort -z |
      xargs -0 -r sha256sum |
      sha256sum |
      awk '{print $1}'
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
    printf 'AWG_CLIENT_FILES_SHA=%s\n' "$(client_tree_sha)"
}

echo
echo '===== PRECHECK CURRENT PRODUCTION ====='
systemctl is-active --quiet wg-quick@wg0.service
systemctl is-active --quiet awg-quick@awg0.service
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx
nginx -t >/dev/null

[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" == 200 ]]
[[ "$(curl -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1/)" == 401 ]]

for unit in "${LEGACY_UNITS[@]}"; do
    state="$(systemctl is-active "$unit" 2>/dev/null || true)"
    enabled="$(systemctl is-enabled "$unit" 2>/dev/null || true)"
    echo "$unit active=$state enabled=$enabled"
    [[ "$state" != active ]]
done

# The old daemon must not be listening.
if ss -lntp | grep -Eq ':(5001)[[:space:]]'; then
    echo 'ERROR: legacy WireGuard UI still listens on TCP 5001' >&2
    exit 1
fi

# Only the known legacy Nginx site and legacy units may reference the old WG UI.
python3 - <<'PY'
from pathlib import Path

allowed = {
    "/etc/nginx/sites-available/wireguard-ui-5000.conf",
    "/etc/systemd/system/wgui.path",
    "/etc/systemd/system/wgui.service",
    "/etc/systemd/system/wireguard-ui-daemon.service",
}

roots = (
    Path("/etc/nginx"),
    Path("/etc/systemd/system"),
)

needles = (
    "wireguard-ui",
    "wireguard-ui-daemon",
    "/etc/wireguard/.env",
    "wgui.service",
    "wgui.path",
    "127.0.0.1:5001",
)

bad=[]

for root in roots:
    for path in root.rglob("*"):
        if path.is_symlink() or not path.is_file():
            continue
        try:
            text=path.read_text(errors="ignore")
        except Exception:
            continue
        if any(n in text for n in needles) and str(path) not in allowed:
            bad.append(str(path))

if bad:
    raise SystemExit(
        "unexpected references to legacy WG UI: "
        + ", ".join(sorted(set(bad)))
    )
PY

[[ -f /etc/nginx/sites-available/wireguard-ui-5000.conf ]]
[[ -L /etc/nginx/sites-enabled/wireguard-ui-5000.conf ]]

BEFORE="$(capture_protected_state)"
printf '%s\n' "$BEFORE"

echo
echo '===== VALIDATE AWG-WEB ACCOUNT OWNERSHIP ====='
if getent passwd awg-web >/dev/null; then
    mapfile -t OWNED < <(
      find / -xdev \( -user awg-web -o -group awg-web \) -print 2>/dev/null
    )

    printf 'owned paths = %s\n' "${#OWNED[@]}"

    for p in "${OWNED[@]}"; do
        case "$p" in
          /etc/amnezia/amneziawg/clients|/etc/amnezia/amneziawg/clients/*)
            ;;
          /root/awg0-client-*.conf.before-*)
            ;;
          *)
            echo "ERROR: unexpected awg-web-owned path: $p" >&2
            exit 1
            ;;
        esac
    done
else
    OWNED=()
fi

echo
echo '===== BACKUP FINAL LEGACY STATE ====='
PATHS=(
  "${LEGACY_UNIT_FILES[@]}"
  "${LEGACY_WG_FILES[@]}"
  "${LEGACY_NGINX_FILES[@]}"
  /etc/amnezia/amneziawg/clients
)

for p in "${OWNED[@]:-}"; do
    case "$p" in
      /root/awg0-client-*.conf.before-*) PATHS+=("$p") ;;
    esac
done

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
    echo 'ERROR: nothing found to back up' >&2
    exit 1
fi

tar -C / -czf "$BACKUP" "${REL[@]}"
chmod 600 "$BACKUP"
echo "backup   = $BACKUP"
echo "manifest = $MANIFEST"
du -h "$BACKUP"

PASSWD_LINE="$(getent passwd awg-web || true)"
GROUP_LINE="$(getent group awg-web || true)"
AWG_UID="$(printf '%s\n' "$PASSWD_LINE" | cut -d: -f3)"
AWG_GID="$(printf '%s\n' "$GROUP_LINE" | cut -d: -f3)"

rollback() {
    rc=$?
    trap - ERR
    echo "FINAL LEGACY CLEANUP FAILED rc=$rc — ROLLBACK" >&2

    if [[ -n "$GROUP_LINE" ]] && ! getent group awg-web >/dev/null 2>&1; then
        groupadd -g "$AWG_GID" awg-web || true
    fi

    if [[ -n "$PASSWD_LINE" ]] && ! getent passwd awg-web >/dev/null 2>&1; then
        useradd           --system           --uid "$AWG_UID"           --gid "$AWG_GID"           --home-dir /home/awg-web           --shell /usr/sbin/nologin           --no-create-home           awg-web || true
    fi

    tar -C / -xzf "$BACKUP" || true
    systemctl daemon-reload || true

    # These units were already disabled/inactive before cleanup. Do not start them.
    nginx -t >/dev/null 2>&1 && systemctl reload nginx || true

    echo "rollback backup = $BACKUP" >&2
    exit "$rc"
}
trap rollback ERR

echo
echo '===== REMOVE DISABLED LEGACY WG UNITS ====='
for unit in "${LEGACY_UNITS[@]}"; do
    systemctl disable --now "$unit" >/dev/null 2>&1 || true
done

rm -f   /etc/systemd/system/wgui.path   /etc/systemd/system/wgui.service   /etc/systemd/system/wireguard-ui-daemon.service   /etc/systemd/system/multi-user.target.wants/wgui.path   /etc/systemd/system/multi-user.target.wants/wgui.service   /etc/systemd/system/multi-user.target.wants/wireguard-ui-daemon.service

echo
echo '===== REMOVE LEGACY WG UI FILES ====='
rm -f \
  /etc/wireguard/wireguard-ui \
  /etc/wireguard/.env

echo
echo '===== REMOVE LEGACY WG NGINX FRONTEND ====='
rm -f \
  /etc/nginx/sites-enabled/wireguard-ui-5000.conf \
  /etc/nginx/sites-available/wireguard-ui-5000.conf

nginx -t
systemctl reload nginx
sleep 1
systemctl is-active --quiet nginx

systemctl daemon-reload
systemctl reset-failed wgui.path wgui.service wireguard-ui-daemon.service >/dev/null 2>&1 || true

echo
echo '===== RETIRE AWG-WEB ACCOUNT SAFELY ====='
if getent passwd awg-web >/dev/null; then
    if [[ -d /etc/amnezia/amneziawg/clients ]]; then
        chown -R root:root /etc/amnezia/amneziawg/clients
    fi

    for p in "${OWNED[@]}"; do
        case "$p" in
          /root/awg0-client-*.conf.before-*)
            [[ -e "$p" ]] && chown root:root "$p"
            ;;
        esac
    done

    if find / -xdev \( -user awg-web -o -group awg-web \) -print -quit 2>/dev/null | grep -q .; then
        echo 'ERROR: awg-web still owns files after reassignment' >&2
        find / -xdev \( -user awg-web -o -group awg-web \) -print 2>/dev/null | head -50 >&2
        false
    fi

    userdel awg-web
fi

if getent group awg-web >/dev/null; then
    groupdel awg-web
fi

if find /etc/amnezia/amneziawg/clients -maxdepth 1 \
     \( ! -user root -o ! -group root \) -print -quit 2>/dev/null | grep -q .; then
    echo 'ERROR: AWG client configs are not root-owned after account retirement' >&2
    false
fi

echo
echo '===== VERIFY LEGACY ABSENCE ====='
for unit in "${LEGACY_UNITS[@]}"; do
    load="$(systemctl show "$unit" -p LoadState --value 2>/dev/null || true)"
    if [[ "$load" != not-found ]]; then
        echo "ERROR: legacy unit still loadable: $unit ($load)" >&2
        false
    fi
done

if ss -lntp | grep -Eq ':(5000|5001|5002|5003|5004|8080)[[:space:]]'; then
    echo 'ERROR: legacy UI listener remains' >&2
    ss -lntp | grep -E ':(5000|5001|5002|5003|5004|8080)[[:space:]]' >&2 || true
    false
fi

if getent passwd awg-web >/dev/null || getent group awg-web >/dev/null; then
    echo 'ERROR: awg-web account/group still exists' >&2
    false
fi

for p in "${LEGACY_UNIT_FILES[@]}" "${LEGACY_WG_FILES[@]}" "${LEGACY_NGINX_FILES[@]}"; do
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
ss -lunp | grep -Eq '(^|[[:space:]])0\.0\.0\.0:51820[[:space:]]|\[::\]:51820[[:space:]]'
ss -lntp | grep -q '127\.0\.0\.1:8090'

AFTER="$(capture_protected_state)"
printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
    echo 'ERROR: protected VPN state or AWG client file contents changed' >&2
    diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
    false
fi

trap - ERR

echo
echo '========================================'
echo 'FINAL LEGACY CLEANUP = PASS'
echo 'WGUI PATH/SERVICE = REMOVED'
echo 'WIREGUARD-UI DAEMON = REMOVED'
echo 'WIREGUARD-UI NGINX FRONTEND = REMOVED'
echo 'LEGACY TCP 5000/5001/5002/5003/5004/8080 = ABSENT'
echo 'AWG-WEB ACCOUNT/GROUP = REMOVED'
echo 'AWG CLIENT CONFIG FILES = PRESERVED, ROOT-OWNED'
echo 'WG UDP 51820 = PRESENT'
echo 'AWG UDP 8443 = PRESENT'
echo 'CURRENT VPN-UI = HEALTHY'
echo 'WG0 / AWG0 = NOT RESTARTED'
echo 'WG/AWG CONFIGS, PEER COUNTS AND AWG CLIENT FILE CONTENTS = UNCHANGED'
echo "BACKUP = $BACKUP"
echo '========================================'
REMOTE
