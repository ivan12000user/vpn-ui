#!/usr/bin/env bash
# Guarded upgrade: vpn-ui 0.8.1 -> 0.8.2, only vpn-ui.service restarted.
set -Eeuo pipefail
umask 077
TARGET=/usr/local/bin/vpn-ui
CANDIDATE=/opt/vpn-ui-ts-lab/vpn-ui.upload
BASE=http://127.0.0.1:8090
BACKUP=""
REPLACED=0
SUCCESS=0

finish() {
  local previous_exit=$?
  trap - EXIT
  if [[ "$REPLACED" == 1 && "$SUCCESS" != 1 ]]; then
    echo '===== FAILED: AUTOMATIC BINARY ROLLBACK ====='
    if [[ -f "$BACKUP/vpn-ui.0.8.1" ]]; then
      install -m 0755 "$BACKUP/vpn-ui.0.8.1" "$TARGET.recover"
      mv -f "$TARGET.recover" "$TARGET"
      systemctl restart vpn-ui.service || true
      curl --noproxy '*' -fsS --max-time 10 "$BASE/healthz" || true
      echo
    fi
  fi
  exit "$previous_exit"
}
trap finish EXIT

echo '===== PRECHECK ====='
[[ $(id -u) == 0 ]]
[[ -x "$TARGET" && -s "$CANDIDATE" ]]
systemctl is-active --quiet vpn-ui.service
for unit in nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled; do
  systemctl is-active --quiet "$unit"
done
old_version=$(
  curl --noproxy '*' -fsS --max-time 5 "$BASE/healthz" |
  python3 -c 'import json,sys; print(json.load(sys.stdin).get("version",""))'
)
if [[ "$old_version" != "0.8.1" ]]; then
  echo "REFUSING: expected v0.8.1, got $old_version" >&2
  exit 1
fi

echo '===== BACKUP ====='
install -d -m 0700 /root/vpn-ui-backups
BACKUP=$(mktemp -d /root/vpn-ui-backups/ts-ping-v082-XXXXXXXX)
chmod 0700 "$BACKUP"
cp -a "$TARGET" "$BACKUP/vpn-ui.0.8.1"
sha256sum "$TARGET" "$CANDIDATE" > "$BACKUP/sha256-before.txt"
systemctl cat vpn-ui.service > "$BACKUP/vpn-ui-unit-before.txt"
echo "Backup directory: $BACKUP"

echo '===== REPLACE ONLY VPN-UI BINARY ====='
install -m 0755 "$CANDIDATE" "$TARGET.new"
mv -f "$TARGET.new" "$TARGET"
REPLACED=1
systemctl restart vpn-ui.service

echo '===== HEALTH CHECK ====='
ready=0
for _ in $(seq 1 30); do
  if curl --noproxy '*' -fsS --max-time 3 "$BASE/healthz" -o "$BACKUP/health.json" 2>/dev/null; then
    if python3 - "$BACKUP/health.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))["version"] == "0.8.2"
PY
    then
      ready=1
      break
    fi
  fi
  sleep 1
done
[[ "$ready" == 1 ]]
cat "$BACKUP/health.json"
echo

echo '===== TAILSCALE PING ENDPOINT ====='
python3 - <<'PY'
import ipaddress
import json
import urllib.request

base = "http://127.0.0.1:8090"
def get(path):
    with urllib.request.urlopen(base + path, timeout=15) as response:
        return json.load(response)
def ping(ip):
    req = urllib.request.Request(
        base + "/api/tailscale/ping",
        data=json.dumps({"ip":ip}).encode(),
        headers={"Content-Type":"application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=25) as response:
        return json.load(response)

status = get("/api/tailscale/status")
assert not status.get("error"), "Tailscale status unavailable"
network = ipaddress.ip_network("100.64.0.0/10")
ips = [p.get("ip") for p in (status.get("peers") or [])]
valid = []
for ip in ips:
    try:
        if ipaddress.ip_address(ip) in network:
            valid.append(ip)
    except (TypeError, ValueError):
        pass
assert valid, "No Tailscale peer IPs available"

bad = ping("127.0.0.1")
assert bad.get("ok") is False and bad.get("ip") == ""
assert bad.get("error") == "Tailscale peer not found"
print("Arbitrary IP blocked: PASS")
known = ping(valid[0])
assert known.get("ip") == valid[0]
assert isinstance(known.get("ok"), bool)
print("Known peer:", valid[0])
print("ICMP:", "%.1f ms" % known["avg_ms"] if known["ok"] else known.get("error"))
print("ICMP timeouts can occur if firewall/ACL denies echo requests")
PY

echo '===== HTML CONTROLS ====='
curl --noproxy '*' -fsS --max-time 10 "$BASE/tailscale" -o "$BACKUP/tailscale.html"
grep -F '<th>Ping</th>' "$BACKUP/tailscale.html" >/dev/null
grep -F 'id="ts-ping-all"' "$BACKUP/tailscale.html" >/dev/null
rm -f "$BACKUP/tailscale.html"
echo 'Per-node Ping and Ping-all: PASS'

echo '===== SERVICES ====='
for unit in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled; do
  printf '%-23s ' "$unit"
  systemctl is-active "$unit"
done
SUCCESS=1
echo '===== VPN-UI 0.8.2 DEPLOYED SUCCESSFULLY ====='
echo "Rollback binary: $BACKUP/vpn-ui.0.8.1"
