#!/usr/bin/env bash
# Run from the reviewed repository root with a locally built release binary.
# Only vpn-ui.service is restarted. No VPN configurations or database writes.
set -Eeuo pipefail
umask 077
export LC_ALL=C
BIN=/usr/local/bin/vpn-ui
WRAPPER=/usr/local/bin/vpn-ui-zerotier-ro
SUDOERS=/etc/sudoers.d/vpn-ui-zerotier
BASE=http://127.0.0.1:8090
BACKUP=''
CHANGED=0
SUCCESS=0
SERVICES=(nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled vpn-ui-history.timer zerotier-one)

snapshot() {
  local svc=$1 destination=$2
  systemctl show "$svc" -p LoadState -p ActiveState -p SubState -p MainPID -p ActiveEnterTimestampMonotonic > "$destination" ||
    grep -Fx 'LoadState=not-found' "$destination" >/dev/null
}

health_version() {
  curl --noproxy '*' -fsS --max-time 5 "$BASE/healthz" |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])'
}
finish() {
  local code=$?
  trap - EXIT
  if [[ $CHANGED == 1 && $SUCCESS != 1 ]]; then
    echo "Deployment failed; restoring VPN UI 0.8.7 from $BACKUP" >&2
    set +e
    install -m 0755 "$BACKUP/vpn-ui.0.8.7" "$BIN.rollback"
    mv -f "$BIN.rollback" "$BIN"
    rm -f "$WRAPPER" "$SUDOERS"
    systemctl restart vpn-ui.service
    restored=0
    for _ in {1..20}; do
      if [[ $(health_version) == 0.8.7 ]]; then restored=1; break; fi
      sleep 1
    done
    [[ $restored == 1 ]] || echo "ROLLBACK HEALTH FAILED: inspect vpn-ui.service; backup: $BACKUP" >&2
    code=1
  fi
  exit "$code"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

[[ $(id -u) == 0 ]]
exec 9>/run/lock/vpn-ui-v088.lock
flock -n 9
[[ $# == 1 && $1 =~ ^[a-f0-9]{64}$ ]] || { echo 'Usage: installer EXPECTED_BINARY_SHA256' >&2; exit 64; }
[[ -x "$BIN" && -s target/release/vpn-ui ]]
[[ ! -e "$WRAPPER" && ! -L "$WRAPPER" && ! -e "$SUDOERS" && ! -L "$SUDOERS" ]]
[[ $(health_version) == 0.8.7 ]]
id vpn-ui >/dev/null
systemctl is-active --quiet vpn-ui.service
visudo -cf deploy/sudoers/vpn-ui-zerotier
# Refuse any race or accidental replacement of the reviewed binary.
[[ $(sha256sum target/release/vpn-ui | cut -d' ' -f1) == "$1" ]]
install -d -m 0700 /root/vpn-ui-backups
BACKUP=$(mktemp -d /root/vpn-ui-backups/zerotier-v088-XXXXXXXX)
cp -a "$BIN" "$BACKUP/vpn-ui.0.8.7"
systemctl cat vpn-ui.service > "$BACKUP/service-before.txt"
for svc in "${SERVICES[@]}"; do
  snapshot "$svc" "$BACKUP/$svc.before"
done
install -m 0755 target/release/vpn-ui "$BIN.v088"
[[ $(sha256sum "$BIN.v088" | cut -d' ' -f1) == "$1" ]]
# From this point every partial installation triggers rollback.
CHANGED=1
install -o root -g root -m 0755 deploy/bin/vpn-ui-zerotier-ro "$WRAPPER"
install -o root -g root -m 0440 deploy/sudoers/vpn-ui-zerotier "$SUDOERS"
visudo -cf /etc/sudoers
mv -f "$BIN.v088" "$BIN"
systemctl restart vpn-ui.service
ready=0
for _ in {1..30}; do
  if [[ $(health_version) == 0.8.8 ]]; then ready=1; break; fi
  sleep 1
done
[[ $ready == 1 ]]
curl --noproxy '*' -fsS --max-time 15 "$BASE/zerotier" > "$BACKUP/zerotier.html"
grep -F 'zt-grid' "$BACKUP/zerotier.html" >/dev/null
! grep -F 'href="/amneziawg"' "$BACKUP/zerotier.html"
curl --noproxy '*' -fsS --max-time 15 "$BASE/api/zerotier/status" > "$BACKUP/zerotier.json"
python3 - "$BACKUP/zerotier.json" <<'PY'
import json,sys
s=json.load(open(sys.argv[1]))
assert set(s)=={'node','networks','peers','errors'}
# Missing ZeroTier is a supported degraded state; errors must be visible.
assert s['node'] is not None or s['errors']
print('ZeroTier reads:', 'available' if not s['errors'] else s['errors'])
PY
for route in wireguard amneziawg31 tailscale history settings; do
  curl --noproxy '*' -fsS --max-time 15 "$BASE/$route" -o /dev/null
done
for svc in "${SERVICES[@]}"; do
  snapshot "$svc" "$BACKUP/$svc.after"
  cmp "$BACKUP/$svc.before" "$BACKUP/$svc.after"
done
SUCCESS=1
echo "Installed VPN UI 0.8.8. Previous binary: $BACKUP/vpn-ui.0.8.7"
