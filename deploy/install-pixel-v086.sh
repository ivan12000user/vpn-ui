#!/usr/bin/env bash
# Graph/mobile-only v0.8.5 -> v0.8.6 upgrade. No VPN service restarts.
set -Eeuo pipefail
umask 077
BIN=/usr/local/bin/vpn-ui
STAGE=/opt/vpn-ui-pixel-v086
UPLOAD="$STAGE/vpn-ui.upload"
BASE=http://127.0.0.1:8090
BACKUP=''
SWAPPED=0
SUCCESS=0

finish() {
  local code=$?
  trap - EXIT
  if [[ "$SWAPPED" == 1 && "$SUCCESS" != 1 ]]; then
    echo '===== FAILURE: AUTOMATIC BINARY ROLLBACK TO 0.8.5 ====='
    if [[ -s "$BACKUP/vpn-ui.0.8.5" ]]; then
      install -m 0755 "$BACKUP/vpn-ui.0.8.5" "$BIN.rollback"
      mv -f "$BIN.rollback" "$BIN"
      systemctl restart vpn-ui.service || true
      curl --noproxy '*' -fsS --max-time 10 "$BASE/healthz" || true
      echo
    fi
  fi
  exit "$code"
}
trap finish EXIT

echo '===== PREFLIGHT ====='
[[ $(id -u) == 0 && -x "$BIN" && -s "$UPLOAD" ]]
for svc in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled vpn-ui-history.timer; do
  systemctl is-active --quiet "$svc"
done
version=$(curl --noproxy '*' -fsS --max-time 5 "$BASE/healthz" |
  python3 -c 'import json,sys;print(json.load(sys.stdin)["version"])')
[[ "$version" == 0.8.5 ]] || {
  echo "Refusing upgrade: expected VPN UI 0.8.5; got $version" >&2
  exit 1
}
nginx -t
# History DB must be present; this upgrade must never initialize/replace it.
curl --noproxy '*' -fsS --max-time 10 "$BASE/api/history?hours=24" |
  python3 -c 'import json,sys;assert json.load(sys.stdin)["configured"] is True'
test -s /var/lib/vpn-ui/history/metrics.sqlite3

echo '===== BACKUP ====='
install -d -m 0700 /root/vpn-ui-backups
BACKUP=$(mktemp -d /root/vpn-ui-backups/pixel-ui-v086-XXXXXXXX)
cp -a "$BIN" "$BACKUP/vpn-ui.0.8.5"
sha256sum "$BIN" "$UPLOAD" > "$BACKUP/sha256.txt"
systemctl cat vpn-ui.service > "$BACKUP/service-unit-before.txt"
echo "Backup: $BACKUP"

echo '===== UPGRADE ONLY WEB BINARY ====='
install -m 0755 "$UPLOAD" "$BIN.v086"
mv -f "$BIN.v086" "$BIN"
SWAPPED=1
systemctl restart vpn-ui.service

echo '===== HEALTH ====='
READY=0
for _ in $(seq 1 30); do
  if curl --noproxy '*' -fsS --max-time 3 "$BASE/healthz" \
    -o "$BACKUP/health.json" 2>/dev/null; then
    if python3 - "$BACKUP/health.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))["version"]=="0.8.6"
PY
    then READY=1; break; fi
  fi
  sleep 1
done
[[ $READY == 1 ]]
cat "$BACKUP/health.json"
echo

echo '===== MOBILE UI AND HISTORY ====='
curl --noproxy '*' -fsS --max-time 8 "$BASE/history" -o "$BACKUP/history.html"
grep -F '<details class="mobile-nav">' "$BACKUP/history.html" >/dev/null
grep -F '<summary aria-label="Меню разделов VPN UI">' "$BACKUP/history.html" >/dev/null
grep -F '<div class="nav-links-desktop">' "$BACKUP/history.html" >/dev/null
grep -F 'class="history-filters"' "$BACKUP/history.html" >/dev/null
grep -F "width = compact ? 420 : 900" "$BACKUP/history.html" >/dev/null
curl --noproxy '*' -fsS --max-time 10 "$BASE/api/history?hours=24" |
  python3 -c 'import json,sys;a=json.load(sys.stdin);assert a["configured"];print("History peers:",len(a["peers"]))'
rm -f "$BACKUP/history.html"
echo 'Responsive Pixel layout: PASS'

echo '===== HEALTHY SERVICES ====='
for svc in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled vpn-ui-history.timer; do
  printf '%-26s ' "$svc"
  systemctl is-active "$svc"
done
SUCCESS=1
echo '===== SUCCESS: VPN UI 0.8.6 PIXEL UI INSTALLED ====='
echo "Rollback binary: $BACKUP/vpn-ui.0.8.5"
