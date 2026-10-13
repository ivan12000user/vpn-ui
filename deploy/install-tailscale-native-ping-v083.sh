#!/usr/bin/env bash
# Guarded v0.8.2 -> v0.8.3 migration. Only vpn-ui.service is restarted.
set -Eeuo pipefail
umask 077
BASE=http://127.0.0.1:8090
ROOT=/opt/vpn-ui-ts-lab
TARGET=/usr/local/bin/vpn-ui
HELPER=/usr/local/libexec/vpn-ui-tailscale-ping
WRAPPER=/usr/local/bin/vpn-ui-tailscale-ping
SUDOERS=/etc/sudoers.d/vpn-ui-tailscale-ping
BACKUP=""
TOUCHED=0
SUCCEEDED=0

finish() {
  local code=$?
  trap - EXIT
  if [[ "$TOUCHED" == 1 && "$SUCCEEDED" == 0 ]]; then
    echo '===== FAILURE: ROLLBACK v0.8.2 ====='
    set +e
    if [[ -n "$BACKUP" && -f "$BACKUP/vpn-ui.0.8.2" ]]; then
      install -m 0755 "$BACKUP/vpn-ui.0.8.2" "${TARGET}.rollback"
      mv -f "${TARGET}.rollback" "$TARGET"
      rm -f "$SUDOERS" "$WRAPPER" "$HELPER"
      systemctl restart vpn-ui.service
      curl --noproxy '*' -fsS --max-time 10 "$BASE/healthz"
      echo
    fi
    set -e
  fi
  exit "$code"
}
trap finish EXIT

echo '===== PREFLIGHT ====='
[[ $(id -u) == 0 ]]
[[ -x "$TARGET" && -s "$ROOT/vpn-ui.upload" ]]
for name in vpn-ui-tailscale-ping.helper vpn-ui-tailscale-ping.wrapper vpn-ui-tailscale-ping.sudoers; do
  [[ -s "$ROOT/$name" ]]
done
for name in "$HELPER" "$WRAPPER" "$SUDOERS"; do
  if [[ -e "$name" ]]; then
    echo "Refusing to overwrite existing $name" >&2
    exit 1
  fi
done
for svc in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled; do
  systemctl is-active --quiet "$svc"
done
old=$(curl --noproxy '*' -fsS --max-time 8 "$BASE/healthz" |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])')
[[ "$old" == "0.8.2" ]] || { echo "Expected 0.8.2, got $old" >&2; exit 1; }
[[ "$(systemctl show vpn-ui.service -p NoNewPrivileges --value)" == no ]] || {
  echo 'Cannot use narrow sudo helper when NoNewPrivileges=yes' >&2
  exit 1
}
sudo --version >/dev/null
visudo -cf "$ROOT/vpn-ui-tailscale-ping.sudoers"
python3 -m py_compile "$ROOT/vpn-ui-tailscale-ping.helper"
bash -n "$ROOT/vpn-ui-tailscale-ping.wrapper"

echo '===== BACKUP ====='
install -d -m 0700 /root/vpn-ui-backups
BACKUP=$(mktemp -d /root/vpn-ui-backups/ts-native-v083-XXXXXXXX)
chmod 0700 "$BACKUP"
cp -a "$TARGET" "$BACKUP/vpn-ui.0.8.2"
sha256sum "$TARGET" "$ROOT/vpn-ui.upload" > "$BACKUP/sha256.txt"
systemctl cat vpn-ui.service > "$BACKUP/vpn-ui.service.before"
echo "Backup: $BACKUP"

echo '===== INSTALL NARROW HELPER ====='
TOUCHED=1
install -m 0755 "$ROOT/vpn-ui-tailscale-ping.helper" "$HELPER"
install -m 0755 "$ROOT/vpn-ui-tailscale-ping.wrapper" "$WRAPPER"
install -m 0440 "$ROOT/vpn-ui-tailscale-ping.sudoers" "$SUDOERS"
visudo -cf "$SUDOERS"
# Verify vpn-ui can execute ONLY the fixed no-argument helper.
# An intentionally rejected loopback address returns helper exit code 64;
# sudo permission errors return a different status.
set +e
printf '%s\n' '127.0.0.1' | sudo -n -u vpn-ui "$WRAPPER" >/dev/null 2>&1
permission_rc=$?
set -e
if [[ "$permission_rc" != 64 ]]; then
  echo "Restricted helper privilege test failed (rc=$permission_rc)" >&2
  exit 1
fi
echo 'Fixed-helper sudo permission: PASS'

echo '===== UPGRADE BINARY ====='
install -m 0755 "$ROOT/vpn-ui.upload" "${TARGET}.v083"
mv -f "${TARGET}.v083" "$TARGET"
systemctl restart vpn-ui.service

echo '===== HEALTH ====='
READY=0
for i in $(seq 1 30); do
  if curl --noproxy '*' -fsS --max-time 3 "$BASE/healthz" > "$BACKUP/health.json" 2>/dev/null; then
    if python3 - "$BACKUP/health.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))["version"]=="0.8.3"
PY
    then READY=1; break; fi
  fi
  sleep 1
done
[[ "$READY" == 1 ]]
cat "$BACKUP/health.json"
echo

echo '===== SECURE TS PING AND ICMP API ====='
python3 - <<'PY'
import json,urllib.request
base="http://127.0.0.1:8090"
def get(path):
  with urllib.request.urlopen(base+path,timeout=12) as response:
    return json.load(response)
def ping(route, ip):
  req=urllib.request.Request(base+route,
    data=json.dumps({"ip":ip}).encode(),
    headers={"Content-Type":"application/json"},method="POST")
  with urllib.request.urlopen(req,timeout=28) as response:
    return json.load(response)
peers=get("/api/tailscale/status")
assert not peers.get("error"), "Tailscale status failure"
ips=[p.get("ip") for p in (peers.get("peers") or []) if p.get("ip")]
assert ips,"No peers in Tailscale netmap"
bad=ping("/api/tailscale/overlay-ping","127.0.0.1")
assert bad["ok"] is False and bad["ip"]==""
print("SSRF protection: PASS")
good=ping("/api/tailscale/overlay-ping",ips[0])
assert good["ip"]==ips[0]
print("Tailscale ping:",good.get("avg_ms"),"ms",
      good.get("path"),good.get("relay"),"error:",good.get("error"))
assert good.get("error") != "Tailscale ping helper unavailable"
icmp=ping("/api/tailscale/ping",ips[0])
assert icmp.get("ip")==ips[0]
print("ICMP:",icmp.get("avg_ms"),"ms",icmp.get("error"))
PY

echo '===== PAGE CONTROLS ====='
curl --noproxy '*' -fsS --max-time 8 "$BASE/tailscale" -o "$BACKUP/tailscale.html"
grep -F '<th>Tailscale ping</th>' "$BACKUP/tailscale.html" >/dev/null
grep -F '<th>ICMP</th>' "$BACKUP/tailscale.html" >/dev/null
grep -F 'id="ts-ping-all"' "$BACKUP/tailscale.html" >/dev/null
rm -f "$BACKUP/tailscale.html"
echo 'TS ping primary; ICMP secondary: PASS'

echo '===== SERVICES ====='
for svc in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled; do
  printf '%-25s ' "$svc"
  systemctl is-active "$svc"
done
SUCCEEDED=1
echo '===== VPN UI 0.8.3 SUCCESS ====='
echo "Rollback binary: $BACKUP/vpn-ui.0.8.2"
