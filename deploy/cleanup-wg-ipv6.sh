#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo '===== WG IPV6 CLEANUP PRECHECK ====='
[[ "$(git branch --show-current)" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]
git diff --check

capture_state() {
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

count_inventory() {
    local helper="$1"
    printf '%s\n' '{"op":"list"}' |
    "$helper" |
    python3 -c '
import json,sys
d=json.load(sys.stdin)
peers=d.get("peers", [])
print(d.get("count", len(peers)))
'
}

printf 'WG_CONFIG_SHA=%s\n' "$(sha256sum /etc/wireguard/wg0.conf | cut -d" " -f1)"
printf 'AWG_CONFIG_SHA=%s\n' "$(sha256sum /etc/amnezia/amneziawg/awg0.conf | cut -d" " -f1)"
printf 'WG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/wireguard/server.json | cut -d" " -f1)"
printf 'AWG_SERVER_SHA=%s\n' "$(sha256sum /var/lib/vpn-ui/secure/amneziawg/server.json | cut -d" " -f1)"
printf 'WG_TS=%s\n' "$(systemctl show wg-quick@wg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'AWG_TS=%s\n' "$(systemctl show awg-quick@awg0.service -p ActiveEnterTimestampMonotonic --value)"
printf 'WG_COUNTS=%s/%s/%s\n'   "$(count_inventory /usr/local/bin/vpn-ui-manage-wg)"   "$(grep -c '^\[Peer\]' /etc/wireguard/wg0.conf)"   "$(wg show wg0 peers | wc -l)"
printf 'AWG_COUNTS=%s/%s/%s\n'   "$(count_inventory /usr/local/bin/vpn-ui-manage-awg)"   "$(grep -c '^\[Peer\]' /etc/amnezia/amneziawg/awg0.conf)"   "$(awg show awg0 peers | wc -l)"
printf 'WG_PEERS_SHA=%s\n' "$(wg show wg0 peers | sha256sum | cut -d" " -f1)"
printf 'AWG_PEERS_SHA=%s\n' "$(awg show awg0 peers | sha256sum | cut -d" " -f1)"
printf 'ENS3_IPV6_SHA=%s\n' "$( { ip -6 -o addr show dev ens3 scope global; ip -6 route show default; } | sha256sum | cut -d" " -f1)"
REMOTE
}

echo
echo '===== CAPTURE BASELINE ====='
BEFORE="$(capture_state)"
printf '%s\n' "$BEFORE"

echo
echo '===== VALIDATE CURRENT DESIGN ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

systemctl is-active --quiet wg-quick@wg0.service
systemctl is-active --quiet awg-quick@awg0.service
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx
nginx -t >/dev/null

# Persistent WG config must not contain the experimental ULA.
# Other IPv6 text (for example an IPv6 Endpoint) is allowed and must be preserved.
if grep -qiF 'fd66:66:66' /etc/wireguard/wg0.conf; then
    echo 'ERROR: persistent wg0.conf contains experimental fd66:66:66 state; refusing automatic cleanup' >&2
    python3 - <<'PY'
from pathlib import Path
for n, raw in enumerate(Path("/etc/wireguard/wg0.conf").read_text().splitlines(), 1):
    if "fd66:66:66" in raw.lower():
        line=raw.strip()
        if line.lower().startswith(("privatekey","publickey","presharedkey")):
            continue
        print(f"line {n}: {line}")
PY
    exit 1
fi

# UI inventory must be IPv4-only before cleanup.
python3 - <<'PY'
from pathlib import Path
import ipaddress, json, sys

root=Path("/var/lib/vpn-ui/secure/wireguard")
bad=[]

server=json.loads((root/"server.json").read_text())
for key in ("default_client_allowed_ips","default_dns_servers"):
    for value in server.get(key) or []:
        try:
            obj=ipaddress.ip_network(value, strict=False) if "/" in value else ipaddress.ip_address(value)
        except ValueError:
            continue
        if obj.version == 6:
            bad.append(f"server.json {key}={value}")

for path in sorted((root/"clients").glob("*.json")):
    d=json.loads(path.read_text())
    for key in ("allocated_ips","client_allowed_ips","server_allowed_ips","extra_allowed_ips"):
        for value in d.get(key) or []:
            try:
                obj=ipaddress.ip_network(value, strict=False) if "/" in value else ipaddress.ip_address(value)
            except ValueError:
                continue
            if obj.version == 6:
                bad.append(f"{path.name} {key}={value}")

if bad:
    print("ERROR: WG inventory contains IPv6; refusing automatic cleanup", file=sys.stderr)
    for item in bad:
        print(item, file=sys.stderr)
    raise SystemExit(1)

print("persistent wg0.conf = IPv4-only")
print("WG inventory       = IPv4-only")
PY

# Live peer state may still contain the experimental ULA even though
# persistent config/inventory are already IPv4-only. Allow only that exact
# experimental network; any unrelated IPv6 AllowedIP is a hard stop.
wg show wg0 allowed-ips |
python3 -c '
import ipaddress, sys

experimental = ipaddress.ip_network("fd66:66:66::/64")
experimental_count = 0
unexpected = []

for raw in sys.stdin:
    fields = raw.split(None, 1)
    if len(fields) < 2:
        continue

    for value in fields[1].split(","):
        value = value.strip()
        if not value:
            continue
        try:
            net = ipaddress.ip_network(value, strict=False)
        except ValueError:
            unexpected.append(value)
            continue

        if net.version != 6:
            continue

        if net.subnet_of(experimental):
            experimental_count += 1
        else:
            unexpected.append(value)

if unexpected:
    print("ERROR: unexpected live WG IPv6 AllowedIPs; refusing automatic cleanup", file=sys.stderr)
    for value in unexpected:
        print(f"unexpected IPv6 AllowedIP = {value}", file=sys.stderr)
    raise SystemExit(1)

if experimental_count:
    print(f"live WG experimental IPv6 AllowedIPs = {experimental_count}")
    print("live WG peer state  = will reconcile from persistent IPv4-only config")
else:
    print("live WG peers       = IPv4-only")
'

echo 'precheck           = PASS'
REMOTE

echo
echo '===== REMOVE EXPERIMENTAL WG IPV6 ONLY ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

ADDR='fd66:66:66::1/64'
NET='fd66:66:66::/64'
STAMP="$(date +%Y%m%d_%H%M%S)"
BACKDIR="/root/wg-ipv6-cleanup-backup-$STAMP"
mkdir -p "$BACKDIR"
chmod 700 "$BACKDIR"

ip -6 addr show dev wg0 >"$BACKDIR/wg0-ipv6.before.txt"
ip -6 addr show dev ens3 scope global >"$BACKDIR/ens3-ipv6.before.txt"
ip -6 route show table all >"$BACKDIR/ip6-routes.before.txt"
nft -a list table ip6 nat >"$BACKDIR/ip6-nat.before.txt" 2>/dev/null || true
wg show wg0 allowed-ips >"$BACKDIR/wg0-allowed-ips.before.txt"
sha256sum /etc/wireguard/wg0.conf >"$BACKDIR/wg0.conf.sha256"
sha256sum /etc/amnezia/amneziawg/awg0.conf >"$BACKDIR/awg0.conf.sha256"

HAD_ADDR=0
if ip -6 -o addr show dev wg0 | grep -Fq "$ADDR"; then
    HAD_ADDR=1
fi

mapfile -t HANDLES < <(
    nft -a list chain ip6 nat POSTROUTING 2>/dev/null |
    awk '
      /oifname "ens3"/ &&
      /ip6 saddr fd66:66:66::\/64/ &&
      /masquerade/ &&
      /# handle [0-9]+/ {
          for (i=1; i<=NF; i++) if ($i=="handle") print $(i+1)
      }
    '
)

if (( ${#HANDLES[@]} > 1 )); then
    echo 'ERROR: more than one matching NAT66 rule found; refusing cleanup' >&2
    exit 1
fi

HAD_NAT=0
NAT_HANDLE=''
if (( ${#HANDLES[@]} == 1 )); then
    HAD_NAT=1
    NAT_HANDLE="${HANDLES[0]}"
fi

HAD_LIVE_IPV6_ALLOWED=0
if python3 - "$BACKDIR/wg0-allowed-ips.before.txt" <<'PY'
import ipaddress, pathlib, sys

experimental=ipaddress.ip_network("fd66:66:66::/64")
for raw in pathlib.Path(sys.argv[1]).read_text().splitlines():
    fields=raw.split(None, 1)
    if len(fields) < 2:
        continue
    for value in fields[1].split(","):
        value=value.strip()
        if not value:
            continue
        try:
            net=ipaddress.ip_network(value, strict=False)
        except ValueError:
            continue
        if net.version == 6 and net.subnet_of(experimental):
            raise SystemExit(0)
raise SystemExit(1)
PY
then
    HAD_LIVE_IPV6_ALLOWED=1
fi

restore_allowed_ips() {
    while IFS=    rc=$?
    trap - ERR
    echo "CLEANUP FAILED rc=$rc — ROLLBACK" >&2

    if (( HAD_LIVE_IPV6_ALLOWED == 1 )); then
        restore_allowed_ips || true
    fi

    if (( HAD_ADDR == 1 )); then
        ip -6 addr show dev wg0 | grep -Fq "$ADDR" ||
            ip -6 addr add "$ADDR" dev wg0 || true
    fi

    if (( HAD_NAT == 1 )); then
        if ! nft -a list chain ip6 nat POSTROUTING 2>/dev/null |
             grep -F 'ip6 saddr fd66:66:66::/64' |
             grep -F 'oifname "ens3"' |
             grep -q 'masquerade'; then
            nft add rule ip6 nat POSTROUTING                 oifname "ens3" ip6 saddr "$NET" masquerade || true
        fi
    fi

    exit "$rc"
}
trap rollback ERR

if (( HAD_LIVE_IPV6_ALLOWED == 1 )); then
    STRIPPED="$(mktemp /run/vpn-ui-wg-cleanup.XXXXXX)"
    trap 'rm -f "$STRIPPED"' RETURN
    wg-quick strip /etc/wireguard/wg0.conf >"$STRIPPED"
    chmod 600 "$STRIPPED"
    wg syncconf wg0 "$STRIPPED"
    rm -f "$STRIPPED"
    trap - RETURN
fi

if (( HAD_ADDR == 1 )); then
    ip -6 addr del "$ADDR" dev wg0
fi

if (( HAD_NAT == 1 )); then
    nft delete rule ip6 nat POSTROUTING handle "$NAT_HANDLE"
fi

# Exact cleanup checks.
if wg show wg0 allowed-ips |
   python3 -c '
import ipaddress, sys
experimental=ipaddress.ip_network("fd66:66:66::/64")
for raw in sys.stdin:
    fields=raw.split(None,1)
    if len(fields)<2:
        continue
    for value in fields[1].split(","):
        value=value.strip()
        try:
            net=ipaddress.ip_network(value, strict=False)
        except ValueError:
            continue
        if net.version == 6 and net.subnet_of(experimental):
            raise SystemExit(0)
raise SystemExit(1)
'; then
    echo 'ERROR: experimental IPv6 AllowedIPs still present in live WG peers' >&2
    false
fi

if ip -6 -o addr show dev wg0 | grep -Fq "$ADDR"; then
    echo 'ERROR: experimental WG IPv6 address still present' >&2
    false
fi

if nft -a list chain ip6 nat POSTROUTING 2>/dev/null |
   grep -F 'ip6 saddr fd66:66:66::/64' |
   grep -F 'oifname "ens3"' |
   grep -q 'masquerade'; then
    echo 'ERROR: experimental NAT66 rule still present' >&2
    false
fi

# Public IPv6 and Tailscale NAT must remain available.
ip -6 -o addr show dev ens3 scope global | grep -Fq '2a0a:9300:1:104::1/48'
ip -6 route show default | grep -Fq 'via 2a0a:9300:1::1'
nft list chain ip6 nat POSTROUTING 2>/dev/null | grep -q 'ts-postrouting'

# VPN services were not restarted.
systemctl is-active --quiet wg-quick@wg0.service
systemctl is-active --quiet awg-quick@awg0.service
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx

trap - ERR

echo "cleanup backup = $BACKDIR"
echo "reconciled live WG peer IPv6  = $HAD_LIVE_IPV6_ALLOWED"
echo "removed live wg0 IPv6 address = $HAD_ADDR"
echo "removed experimental NAT66    = $HAD_NAT"
REMOTE

echo
echo '===== VERIFY APPLICATIONS ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

printf '%s\n' '{"op":"list"}' | /usr/local/bin/vpn-ui-manage-wg |
python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d.get("ok") is True
peers=d.get("peers",[])
assert d.get("count", len(peers)) == len(peers)
assert len(peers) > 0
for p in peers:
    ip=p.get("vpn_ip") or ""
    assert ":" not in ip, p
print("WG inventory API = OK, IPv4-only")
'

printf '%s\n' '{"op":"settings_get"}' | /usr/local/bin/vpn-ui-manage-wg |
python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d.get("ok") is True
s=d.get("settings",{})
assert s.get("client_allowed_ips") == ["10.66.66.0/24"], s
print("WG UI defaults   = 10.66.66.0/24")
'

[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" == 200 ]]
[[ "$(curl -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1/)" == 401 ]]

echo 'vpn-ui health     = OK'
echo 'nginx auth        = OK'
REMOTE

echo
echo '===== VERIFY CORE STATE UNCHANGED ====='
AFTER="$(capture_state)"
printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
    echo 'ERROR: protected VPN/public-IPv6 state changed unexpectedly' >&2
    diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
    exit 1
fi

echo
echo '===== FINAL IPV6 STATE ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
echo '-- wg0 IPv6 --'
ip -6 -o addr show dev wg0 || true
echo '-- experimental NAT66 --'
nft -a list chain ip6 nat POSTROUTING 2>/dev/null |
grep -F 'fd66:66:66::/64' || true
echo '-- public ens3 IPv6 --'
ip -6 -o addr show dev ens3 scope global
ip -6 route show default
REMOTE

echo
echo '========================================'
echo 'WG IPV6 CLEANUP = PASS'
echo 'WG INTERNAL NETWORK = IPv4 ONLY (10.66.66.0/24)'
echo 'EXPERIMENTAL fd66:66:66::/64 = REMOVED'
echo 'PUBLIC VPS IPv6 = PRESERVED'
echo 'TAILSCALE IPv6 = PRESERVED'
echo 'AWG = UNCHANGED'
echo 'WG0 / AWG0 = NOT RESTARTED'
echo 'VPN-UI = HEALTHY AND IPv4-ONLY FOR WG'
echo '========================================'
\t' read -r pub allowed; do
        [[ -n "$pub" && -n "$allowed" ]] || continue
        wg set wg0 peer "$pub" allowed-ips "$allowed"
    done <"$BACKDIR/wg0-allowed-ips.before.txt"
}

rollback() {
    rc=$?
    trap - ERR
    echo "CLEANUP FAILED rc=$rc — ROLLBACK" >&2

    if (( HAD_ADDR == 1 )); then
        ip -6 addr show dev wg0 | grep -Fq "$ADDR" ||
            ip -6 addr add "$ADDR" dev wg0 || true
    fi

    if (( HAD_NAT == 1 )); then
        if ! nft -a list chain ip6 nat POSTROUTING 2>/dev/null |
             grep -F 'ip6 saddr fd66:66:66::/64' |
             grep -F 'oifname "ens3"' |
             grep -q 'masquerade'; then
            nft add rule ip6 nat POSTROUTING                 oifname "ens3" ip6 saddr "$NET" masquerade || true
        fi
    fi

    exit "$rc"
}
trap rollback ERR

if (( HAD_ADDR == 1 )); then
    ip -6 addr del "$ADDR" dev wg0
fi

if (( HAD_NAT == 1 )); then
    nft delete rule ip6 nat POSTROUTING handle "$NAT_HANDLE"
fi

# Exact cleanup checks.
if ip -6 -o addr show dev wg0 | grep -Fq "$ADDR"; then
    echo 'ERROR: experimental WG IPv6 address still present' >&2
    false
fi

if nft -a list chain ip6 nat POSTROUTING 2>/dev/null |
   grep -F 'ip6 saddr fd66:66:66::/64' |
   grep -F 'oifname "ens3"' |
   grep -q 'masquerade'; then
    echo 'ERROR: experimental NAT66 rule still present' >&2
    false
fi

# Public IPv6 and Tailscale NAT must remain available.
ip -6 -o addr show dev ens3 scope global | grep -Fq '2a0a:9300:1:104::1/48'
ip -6 route show default | grep -Fq 'via 2a0a:9300:1::1'
nft list chain ip6 nat POSTROUTING 2>/dev/null | grep -q 'ts-postrouting'

# VPN services were not restarted.
systemctl is-active --quiet wg-quick@wg0.service
systemctl is-active --quiet awg-quick@awg0.service
systemctl is-active --quiet vpn-ui.service
systemctl is-active --quiet nginx

trap - ERR

echo "cleanup backup = $BACKDIR"
echo "removed live wg0 IPv6 address = $HAD_ADDR"
echo "removed experimental NAT66    = $HAD_NAT"
REMOTE

echo
echo '===== VERIFY APPLICATIONS ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

printf '%s\n' '{"op":"list"}' | /usr/local/bin/vpn-ui-manage-wg |
python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d.get("ok") is True
peers=d.get("peers",[])
assert d.get("count", len(peers)) == len(peers)
assert len(peers) > 0
for p in peers:
    ip=p.get("vpn_ip") or ""
    assert ":" not in ip, p
print("WG inventory API = OK, IPv4-only")
'

printf '%s\n' '{"op":"settings_get"}' | /usr/local/bin/vpn-ui-manage-wg |
python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d.get("ok") is True
s=d.get("settings",{})
assert s.get("client_allowed_ips") == ["10.66.66.0/24"], s
print("WG UI defaults   = 10.66.66.0/24")
'

[[ "$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/healthz)" == 200 ]]
[[ "$(curl -k -sS -o /dev/null -w '%{http_code}' https://127.0.0.1/)" == 401 ]]

echo 'vpn-ui health     = OK'
echo 'nginx auth        = OK'
REMOTE

echo
echo '===== VERIFY CORE STATE UNCHANGED ====='
AFTER="$(capture_state)"
printf '%s\n' "$AFTER"

if [[ "$AFTER" != "$BEFORE" ]]; then
    echo 'ERROR: protected VPN/public-IPv6 state changed unexpectedly' >&2
    diff -u <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") || true
    exit 1
fi

echo
echo '===== FINAL IPV6 STATE ====='
ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail
echo '-- wg0 IPv6 --'
ip -6 -o addr show dev wg0 || true
echo '-- experimental NAT66 --'
nft -a list chain ip6 nat POSTROUTING 2>/dev/null |
grep -F 'fd66:66:66::/64' || true
echo '-- public ens3 IPv6 --'
ip -6 -o addr show dev ens3 scope global
ip -6 route show default
REMOTE

echo
echo '========================================'
echo 'WG IPV6 CLEANUP = PASS'
echo 'WG INTERNAL NETWORK = IPv4 ONLY (10.66.66.0/24)'
echo 'EXPERIMENTAL fd66:66:66::/64 = REMOVED'
echo 'PUBLIC VPS IPv6 = PRESERVED'
echo 'TAILSCALE IPv6 = PRESERVED'
echo 'AWG = UNCHANGED'
echo 'WG0 / AWG0 = NOT RESTARTED'
echo 'VPN-UI = HEALTHY AND IPv4-ONLY FOR WG'
echo '========================================'
