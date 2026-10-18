#!/usr/bin/env bash
# Guarded v0.8.3 -> v0.8.4; never restarts VPN transports.
set -Eeuo pipefail
umask 077
BASE=http://127.0.0.1:8090
STAGE=/opt/vpn-ui-history-stage
BIN=/usr/local/bin/vpn-ui
HELPER=/usr/local/libexec/vpn-ui-history
SVC=/etc/systemd/system/vpn-ui-history.service
TIMER=/etc/systemd/system/vpn-ui-history.timer
DATA=/var/lib/vpn-ui/history
BACKUP=""
CHANGED=0
SUCCESS=0

rollback() {
  local rc=$?
  trap - EXIT
  if [[ "$CHANGED" == 1 && "$SUCCESS" != 1 ]]; then
    echo '===== INSTALL FAILED: RESTORE VPN UI 0.8.3 ====='
    systemctl disable --now vpn-ui-history.timer >/dev/null 2>&1 || true
    rm -f "$TIMER" "$SVC" "$HELPER"
    systemctl daemon-reload
    if [[ -n "$BACKUP" && -s "$BACKUP/vpn-ui.0.8.3" ]]; then
      install -m 0755 "$BACKUP/vpn-ui.0.8.3" "$BIN.recover"
      mv -f "$BIN.recover" "$BIN"
      systemctl restart vpn-ui.service || true
      curl --noproxy '*' -fsS --max-time 10 "$BASE/healthz" || true
      echo
    fi
    echo 'Collected SQLite data (if any) left untouched for recovery.'
  fi
  exit "$rc"
}
trap rollback EXIT

echo '===== PREFLIGHT ====='
[[ $(id -u) == 0 ]]
for f in "$STAGE/vpn-ui.upload" "$STAGE/vpn-ui-history" \
         "$STAGE/vpn-ui-history.service" "$STAGE/vpn-ui-history.timer"; do
  [[ -s "$f" ]] || { echo "Missing staged artifact $f" >&2; exit 1; }
done
for f in "$SVC" "$TIMER" "$HELPER"; do
  [[ ! -e "$f" ]] || { echo "Already exists: $f" >&2; exit 1; }
done
[[ "$(systemctl show vpn-ui -p User --value)" == "vpn-ui" ]] || {
  echo 'Unexpected vpn-ui service account; refusing to install' >&2; exit 1;
}
for s in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled; do
  systemctl is-active --quiet "$s"
done
VERSION=$(curl --noproxy '*' -fsS --max-time 8 "$BASE/healthz" |
          python3 -c 'import json,sys;print(json.load(sys.stdin)["version"])')
[[ "$VERSION" == 0.8.3 ]] || { echo "Expected 0.8.3; got $VERSION" >&2; exit 1; }
python3 -m py_compile "$STAGE/vpn-ui-history"
# Verify data location is not a symlink to any unrelated privileged path.
[[ ! -L /var/lib/vpn-ui && ! -L "$DATA" ]]
[[ -d /var/lib/vpn-ui ]]

echo '===== BACKUP ====='
install -d -m 0700 /root/vpn-ui-backups
BACKUP=$(mktemp -d /root/vpn-ui-backups/history-v084-XXXXXXXX)
chmod 0700 "$BACKUP"
cp -a "$BIN" "$BACKUP/vpn-ui.0.8.3"
sha256sum "$BIN" "$STAGE/vpn-ui.upload" > "$BACKUP/binary-sha256.txt"
systemctl cat vpn-ui.service > "$BACKUP/unit-before.txt"
echo "Backup: $BACKUP"

echo '===== INSTALL SAMPLER AND TIMER ====='
CHANGED=1
install -d -o vpn-ui -g vpn-ui -m 0700 "$DATA"
install -o root -g root -m 0755 "$STAGE/vpn-ui-history" "$HELPER"
install -o root -g root -m 0644 "$STAGE/vpn-ui-history.service" "$SVC"
install -o root -g root -m 0644 "$STAGE/vpn-ui-history.timer" "$TIMER"
systemctl daemon-reload
systemctl enable vpn-ui-history.timer >/dev/null

echo '===== UPGRADE WEB PANEL ====='
install -m 0755 "$STAGE/vpn-ui.upload" "$BIN.v084"
mv -f "$BIN.v084" "$BIN"
systemctl restart vpn-ui.service

echo '===== HTTP HEALTH ====='
READY=0
for i in $(seq 1 30); do
  if curl --noproxy '*' -fsS --max-time 3 "$BASE/healthz" \
      -o "$BACKUP/health.json" 2>/dev/null; then
    if python3 - "$BACKUP/health.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))["version"]=="0.8.4"
PY
    then READY=1; break; fi
  fi
  sleep 1
done
[[ "$READY" == 1 ]]
cat "$BACKUP/health.json"
echo

echo '===== INITIAL SAMPLE (MAX ~190 SEC) ====='
systemctl start vpn-ui-history.service
systemctl start vpn-ui-history.timer
systemctl is-active --quiet vpn-ui-history.timer

echo '===== HISTORICAL DATA ====='
curl --noproxy '*' -fsS --max-time 12 "$BASE/api/history?hours=24" \
  -o "$BACKUP/history.json"
python3 - "$BACKUP/history.json" <<'PY'
import json,sys
a=json.load(open(sys.argv[1]))
assert a["configured"] is True, a
print("Known history entries:",len(a["peers"]))
for p in a["peers"][:12]:
    print(p["provider"],p["label"][:40], "samples",p["samples"],
          "probes",p["probes"], "replies",p["replies"])
PY
curl --noproxy '*' -fsS --max-time 8 "$BASE/history" \
  -o "$BACKUP/history.html"
grep -F 'id="hist-graph"' "$BACKUP/history.html" >/dev/null
rm -f "$BACKUP/history.html"
echo 'History page: PASS'

echo '===== INTACT VPN SERVICES ====='
for s in vpn-ui nginx wg-quick@wg0 awg-quick@awg0 vpn-ui-awg31 tailscaled vpn-ui-history.timer; do
  printf '%-24s ' "$s"
  systemctl is-active "$s"
done
SUCCESS=1
echo '===== SUCCESS: VPN UI 0.8.4 HISTORY ACTIVE ====='
echo "Rollback binary: $BACKUP/vpn-ui.0.8.3"
