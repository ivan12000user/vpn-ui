#!/usr/bin/env bash
# v0.8.4 -> v0.8.5 graph-only hotfix: only vpn-ui.service restarts.
set -Eeuo pipefail
umask 077
BIN=/usr/local/bin/vpn-ui
UPLOAD=/opt/vpn-ui-history-fix-v085/vpn-ui.upload
BASE=http://127.0.0.1:8090
BACKUP=''
REPLACED=0
SUCCESS=0

finish() {
  local rc=$?
  trap - EXIT
  if [[ $REPLACED == 1 && $SUCCESS != 1 ]]; then
    echo '===== ERROR: RESTORING VPN UI 0.8.4 ====='
    if [[ -s "$BACKUP/vpn-ui.0.8.4" ]]; then
      install -m 0755 "$BACKUP/vpn-ui.0.8.4" "$BIN.recover"
      mv -f "$BIN.recover" "$BIN"
      systemctl restart vpn-ui.service || true
      curl --noproxy '*' -fsS --max-time 10 "$BASE/healthz" || true
      echo
    fi
  fi
  exit "$rc"
}
trap finish EXIT

echo '===== PRECHECK ====='
[[ $(id -u) == 0 && -x "$BIN" && -s "$UPLOAD" ]]
for s in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled vpn-ui-history.timer; do
  systemctl is-active --quiet "$s"
done
version=$(curl --noproxy '*' -fsS --max-time 8 "$BASE/healthz" |
          python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])')
[[ "$version" == 0.8.4 ]] || {
  echo "Refusing to upgrade: expected 0.8.4, got $version" >&2
  exit 1
}
curl --noproxy '*' -fsS --max-time 10 "$BASE/api/history?hours=24" |
  python3 -c 'import json,sys; assert json.load(sys.stdin).get("configured") is True'

echo '===== BACKUP ====='
install -d -m 0700 /root/vpn-ui-backups
BACKUP=$(mktemp -d /root/vpn-ui-backups/history-chart-v085-XXXXXXXX)
cp -a "$BIN" "$BACKUP/vpn-ui.0.8.4"
sha256sum "$BIN" "$UPLOAD" > "$BACKUP/binaries.sha256"
systemctl cat vpn-ui.service > "$BACKUP/service-before.txt"
echo "Backup: $BACKUP"

echo '===== INSTALL UI BINARY ONLY ====='
install -m 0755 "$UPLOAD" "$BIN.v085"
mv -f "$BIN.v085" "$BIN"
REPLACED=1
systemctl restart vpn-ui.service

echo '===== HEALTH ====='
READY=0
for _ in $(seq 1 30); do
  if curl --noproxy '*' -fsS --max-time 3 "$BASE/healthz" \
    -o "$BACKUP/health.json" 2>/dev/null; then
    if python3 - "$BACKUP/health.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))["version"] == "0.8.5"
PY
    then READY=1; break; fi
  fi
  sleep 1
done
[[ $READY == 1 ]]
cat "$BACKUP/health.json"
echo

echo '===== CHART FIX ====='
curl --noproxy '*' -fsS --max-time 8 "$BASE/history" -o "$BACKUP/history.html"
grep -F 'for (const dot of markers) chart.appendChild(dot);' "$BACKUP/history.html" >/dev/null
grep -F 'Ping / записей' "$BACKUP/history.html" >/dev/null
grep -F 'Успешных Ping' "$BACKUP/history.html" >/dev/null
echo 'Single-point SVG markers: PASS'
curl --noproxy '*' -fsS --max-time 8 "$BASE/api/history?hours=24" |
  python3 -c 'import json,sys; a=json.load(sys.stdin); assert a["configured"]; print("History peers:",len(a["peers"]))'
rm -f "$BACKUP/history.html"

echo '===== SERVICES UNCHANGED ====='
for s in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled vpn-ui-history.timer; do
  printf '%-25s ' "$s"
  systemctl is-active "$s"
done
SUCCESS=1
echo '===== SUCCESS: VPN UI 0.8.5 CHART FIX INSTALLED ====='
echo "Rollback binary: $BACKUP/vpn-ui.0.8.4"
