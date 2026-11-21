#!/usr/bin/env bash
# IHOR-only binary upgrade: exactly one service restart, with automatic rollback.
# Never copies WG/AWG credentials, peers, inventory, systemd units or Nginx config.
set -Eeuo pipefail
umask 077
export LC_ALL=C

EXPECTED_HOST_IP=100.88.88.1
EXPECTED_SHA256=232bfda6d12508dd8ea07c768766cdef3529de54f544d77eb7f7d7c37ad6b41a
SRC=/root/vpn-ui-v088-console.x86_64
DST=/usr/local/bin/vpn-ui
API=http://127.0.0.1:8090

if (( EUID != 0 )); then echo 'Must run as root' >&2; exit 1; fi
if [[ $(uname -m) != x86_64 ]]; then echo 'Wrong architecture' >&2; exit 1; fi
if [[ ! -s "$SRC" || ! -x "$DST" ]]; then echo 'Required binaries missing' >&2; exit 1; fi
if [[ $(tailscale ip -4) != "$EXPECTED_HOST_IP" ]]; then
  echo 'Wrong target VPS: expected IHOR Tailscale IP 100.88.88.1' >&2
  exit 1
fi
printf '%s  %s\n' "$EXPECTED_SHA256" "$SRC" | sha256sum -c -

exec 9>/run/lock/vpn-ui-ihor-parity.lock
flock -n 9 || { echo 'Another IHOR upgrade is running' >&2; exit 1; }

if ! systemctl is-active --quiet vpn-ui.service; then
  echo 'vpn-ui service not active; inspect before attempting upgrade' >&2
  exit 1
fi
exec_line=$(systemctl show vpn-ui.service -p ExecStart --value)
if [[ "$exec_line" != *'path=/usr/local/bin/vpn-ui'* ]]; then
  echo 'vpn-ui.service uses a different binary; refusing to alter it' >&2
  exit 1
fi

# Do not alter or restart these services. Preserve whatever state exists.
PROTECTED=(wg-quick@wg0.service vpn-ui-awg31.service awg-quick@awg1.service tailscaled.service zerotier-one.service nginx.service vpn-ui-history.timer)
declare -A states
for svc in "${PROTECTED[@]}"; do
  states["$svc"]=$(systemctl is-active "$svc" 2>/dev/null || true)
done

file_hash() {
  if [[ -f "$1" ]]; then sha256sum "$1" | cut -d' ' -f1
  else printf '%s\n' MISSING
  fi
}
peer_hash() {
  if command -v "$1" >/dev/null 2>&1 && "$1" show "$2" peers >/dev/null 2>&1; then
    "$1" show "$2" peers | LC_ALL=C sort | sha256sum | cut -d' ' -f1
  else
    printf '%s\n' UNAVAILABLE
  fi
}

WG_BEFORE=$(file_hash /etc/wireguard/wg0.conf)
AWG_BEFORE=$(file_hash /etc/amnezia/amneziawg/awg1.conf)
WG_PEERS_BEFORE=$(peer_hash wg wg0)
AWG_PEERS_BEFORE=$(peer_hash awg awg1)

BACKUP=$(mktemp -d /root/vpn-ui-ihor-v088-backup-XXXXXXXX)
cp -a "$DST" "$BACKUP/vpn-ui.before"
systemctl show vpn-ui.service -p FragmentPath -p DropInPaths > "$BACKUP/service-metadata.txt"
printf 'Source SHA256: %s\n' "$EXPECTED_SHA256" > "$BACKUP/source-verified.txt"
echo "Previous executable: $BACKUP/vpn-ui.before"

changed=0
success=0
restore() {
  rc=$?
  trap - EXIT
  if (( changed == 1 && success != 1 )); then
    echo '===== UPGRADE FAILED: RESTORE PREVIOUS VPN UI =====' >&2
    install -o root -g root -m 0755 "$BACKUP/vpn-ui.before" "$DST.rollback"
    mv -f "$DST.rollback" "$DST"
    systemctl restart vpn-ui.service || true
    echo "Restored executable from: $BACKUP/vpn-ui.before" >&2
  fi
  exit "$rc"
}
trap restore EXIT

echo '===== INSTALL UI BINARY ONLY ====='
install -o root -g root -m 0755 "$SRC" "$DST.new"
test "$(sha256sum "$DST.new" | cut -d' ' -f1)" = "$EXPECTED_SHA256"
changed=1
mv -f "$DST.new" "$DST"
systemctl restart vpn-ui.service

echo '===== VERIFY UI ROUTES AND PROVIDERS ====='
python3 - "$API" <<'PY'
import json
import sys
import time
import urllib.request

base = sys.argv[1]
client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
def get(path):
    with client.open(base + path, timeout=20) as response:
        return response.read(2 * 1024 * 1024).decode("utf-8")

for _ in range(25):
    try:
        health = json.loads(get("/healthz"))
        if health.get("status") == "ok" and health.get("version") == "0.8.8":
            break
    except Exception:
        pass
    time.sleep(1)
else:
    raise RuntimeError("VPN UI v0.8.8 health check failed")

page = get("/tailscale")
assert "Доступность по TS Ping" in page
assert "Доступность / статус TS" in page
assert "ts-reachable" in page
assert "ZeroTier" in get("/zerotier")
assert "WireGuard" in get("/wireguard")
assert "AmneziaWG 3.1" in get("/amneziawg31")
assert "История" in get("/history")
assert "Настройки" in get("/settings/manage")

ts = json.loads(get("/api/tailscale/status"))
assert not ts.get("error"), "Local Tailscale API returned error"
assert ts.get("self_ip") == "100.88.88.1", "Target Tailscale IP mismatch"
print("Tailscale local:", ts.get("self_hostname"), ts.get("self_ip"))

for provider in ("wireguard", "amneziawg31"):
    data = json.loads(get(f"/api/{provider}/inventory"))
    assert data.get("ok") is True, f"{provider} inventory unavailable"
    print(provider, "inventory:", len(data.get("peers", [])))

zt = json.loads(get("/api/zerotier/status"))
assert isinstance(zt.get("errors"), list), "ZeroTier response invalid"
if zt["errors"]:
    print("ZeroTier diagnostics:", [str(e)[:100] for e in zt["errors"]])
else:
    print("ZeroTier status: read OK")
print("UI route and inventory checks: PASS")
PY

echo '===== VERIFY VPN FILES / PEER SETS UNCHANGED ====='
test "$WG_BEFORE" = "$(file_hash /etc/wireguard/wg0.conf)"
test "$AWG_BEFORE" = "$(file_hash /etc/amnezia/amneziawg/awg1.conf)"
test "$WG_PEERS_BEFORE" = "$(peer_hash wg wg0)"
test "$AWG_PEERS_BEFORE" = "$(peer_hash awg awg1)"

for svc in "${PROTECTED[@]}"; do
  after=$(systemctl is-active "$svc" 2>/dev/null || true)
  if [[ "$after" != "${states[$svc]}" ]]; then
    echo "Protected service changed state unexpectedly: $svc (${states[$svc]} -> $after)" >&2
    exit 1
  fi
done
systemctl is-active --quiet vpn-ui.service

success=1
trap - EXIT
echo '===== IHOR VPN UI DEPLOYMENT SUCCESS ====='
echo "Installed SHA256: $EXPECTED_SHA256"
echo "Backup: $BACKUP/vpn-ui.before"
echo 'VPN interface files and existing peer public keys unchanged.'
