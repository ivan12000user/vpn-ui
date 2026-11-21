#!/usr/bin/env bash
# Read-only, secret-free survey of the IHOR VPS before VPN UI parity upgrade.
set -Eeuo pipefail
export LC_ALL=C

if (( EUID != 0 )); then echo "Run as root on IHOR" >&2; exit 1; fi
echo '===== IHOR VPN UI AUDIT (READ ONLY) ====='
echo 'Expected Tailscale IP: 100.88.88.1'
printf 'Host: '; hostname
printf 'Architecture: '; uname -m
if command -v tailscale >/dev/null 2>&1; then
  printf 'Tailscale IP: '
  tailscale ip -4 2>/dev/null || true
else
  echo 'Tailscale CLI: missing'
fi

echo '===== SERVICE STATE ====='
for unit in vpn-ui.service nginx.service tailscaled.service wg-quick@wg0.service vpn-ui-awg31.service awg-quick@awg1.service zerotier-one.service vpn-ui-history.timer; do
  state=$(systemctl is-active "$unit" 2>/dev/null || true)
  printf '%-30s %s\n' "$unit" "$state"
done

echo '===== UI BINARY ====='
if test -e /usr/local/bin/vpn-ui; then
  ls -lh /usr/local/bin/vpn-ui
  sha256sum /usr/local/bin/vpn-ui
  exec_line=$(systemctl show vpn-ui.service -p ExecStart --value 2>/dev/null || true)
  if [[ "$exec_line" == *'path=/usr/local/bin/vpn-ui'* ]]; then
    echo 'vpn-ui.service ExecStart: expected binary'
  else
    echo 'vpn-ui.service ExecStart: DIFFERENT (inspect locally; not shown)'
  fi
else
  echo '/usr/local/bin/vpn-ui: missing'
fi

echo '===== SAFE LOCAL API ====='
python3 - <<'PY'
import json
import urllib.error
import urllib.request
base = "http://127.0.0.1:8090"
client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
for path in (
    "/healthz", "/wireguard", "/amneziawg31", "/tailscale",
    "/zerotier", "/history", "/settings/manage",
    "/api/wireguard/inventory", "/api/amneziawg31/inventory",
    "/api/tailscale/status", "/api/zerotier/status",
):
    try:
        with client.open(base + path, timeout=5) as response:
            body = response.read(2 * 1024 * 1024)
            status = response.status
        summary = ""
        if path == "/healthz":
            obj = json.loads(body)
            summary = "version=" + str(obj.get("version", "?"))
        elif path in ("/api/wireguard/inventory", "/api/amneziawg31/inventory"):
            obj = json.loads(body)
            if isinstance(obj, dict):
                summary = "ok=" + str(obj.get("ok")) + " peers=" + str(len(obj.get("peers", [])))
        elif path == "/api/tailscale/status":
            obj = json.loads(body)
            summary = "self=" + str(obj.get("self_ip", "?")) + " peers=" + str(len(obj.get("peers", [])))
            if obj.get("error"): summary += " ERROR"
        elif path == "/api/zerotier/status":
            obj = json.loads(body)
            summary = "node=" + str(obj.get("node") is not None) + " errors=" + str(len(obj.get("errors", [])))
        print(f"{path:34} HTTP {status} {summary}")
    except Exception as exc:
        print(f"{path:34} UNAVAILABLE ({type(exc).__name__})")
PY

echo '===== VPN PEER COUNTS ONLY ====='
for entry in 'wg wg0' 'awg awg1'; do
  read -r tool iface <<<"$entry"
  if command -v "$tool" >/dev/null 2>&1; then
    output=$("$tool" show "$iface" peers 2>/dev/null || true)
    if [[ -n "$output" ]]; then
      printf '%s live peers: %s\n' "$iface" "$(printf '%s\n' "$output" | wc -l)"
    else
      echo "$iface live peers: 0 or interface unavailable"
    fi
  fi
done
python3 - <<'PY'
from pathlib import Path
import re
for provider, cfg, inv in (
    ("wireguard", "/etc/wireguard/wg0.conf", "/var/lib/vpn-ui/secure/wireguard"),
    ("amneziawg31", "/etc/amnezia/amneziawg/awg1.conf", "/var/lib/vpn-ui/secure/amneziawg31"),
):
    config = Path(cfg)
    root = Path(inv)
    if config.is_file():
        peers = len(re.findall(r"(?m)^\s*\[Peer\]\s*$", config.read_text(errors="replace")))
    else:
        peers = "MISSING"
    records = len(list((root / "clients").glob("*.json"))) if (root / "clients").is_dir() else "MISSING"
    print(f"{provider}: persistent peers={peers}, inventory records={records}, server.json={(root / 'server.json').is_file()}")
PY

echo '===== REQUIRED UI HELPERS ====='
for name in vpn-ui-wg-meta vpn-ui-wg-settings vpn-ui-manage-wg vpn-ui-wg-client-config vpn-ui-awg31-meta vpn-ui-awg31-status vpn-ui-awg31-settings vpn-ui-manage-awg31 vpn-ui-awg31-client-config vpn-ui-tailscale-ping vpn-ui-zerotier-ro; do
  if test -x "/usr/local/bin/$name"; then result=PRESENT; else result=MISSING; fi
  printf '%-36s %s\n' "$name" "$result"
done
for path in /usr/local/libexec/vpn-ui-manage /usr/local/libexec/vpn-ui-manage-core-v070 /usr/local/libexec/vpn-ui-tailscale-ping /usr/local/libexec/vpn-ui-history; do
  if test -x "$path"; then result=PRESENT; else result=MISSING; fi
  printf '%-52s %s\n' "$path" "$result"
done

echo '===== SUMMARY ====='
echo 'No keys, PSKs, tokens, credentials, or client records were printed.'
echo 'This script did not change VPN interfaces, service settings or files.'
