#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf '%s\n' '===== WG IPV6 AUDIT (READ ONLY) ====='
printf 'branch = %s\n' "$(git branch --show-current)"

ssh "$REMOTE" 'bash -s' <<'REMOTE'
set -Eeuo pipefail

echo
echo '===== WG0 INTERFACE CONFIG (SAFE FIELDS) ====='
python3 - <<'PY'
from pathlib import Path
import re
p=Path("/etc/wireguard/wg0.conf")
text=p.read_text()
head=re.split(r"(?m)^\s*\[Peer\]\s*$", text, maxsplit=1)[0]
for raw in head.splitlines():
    line=raw.strip()
    if not line or line.startswith(("#",";")) or "=" not in line:
        continue
    k,v=[x.strip() for x in line.split("=",1)]
    if k in {"Address","ListenPort","MTU","Table","PostUp","PostDown","PreUp","PreDown","SaveConfig"}:
        print(f"{k} = {v}")
PY

echo
echo '===== WG0 LIVE ADDRESSES ====='
ip -4 -o addr show dev wg0 || true
ip -6 -o addr show dev wg0 || true

echo
echo '===== WG0 LIVE ROUTES ====='
ip -4 route show dev wg0 || true
ip -6 route show dev wg0 || true

echo
echo '===== IPV6 FORWARDING ====='
for key in   net.ipv6.conf.all.forwarding   net.ipv6.conf.default.forwarding   net.ipv6.conf.ens3.forwarding   net.ipv6.conf.wg0.forwarding
do
    printf '%-38s = ' "$key"
    sysctl -n "$key" 2>/dev/null || echo 'N/A'
done

echo
echo '===== PUBLIC IPV6 ON WAN ====='
ip -6 -o addr show dev ens3 scope global || true
ip -6 route show default || true

echo
echo '===== WG PEER ALLOWEDIPS SUMMARY ====='
python3 - <<'PY'
from pathlib import Path
import re, ipaddress
text=Path("/etc/wireguard/wg0.conf").read_text()
blocks=re.split(r"(?m)^\s*\[Peer\]\s*$", text)[1:]
v4=v6=dual=0
for i,b in enumerate(blocks,1):
    vals=[]
    name=""
    for raw in b.splitlines():
        line=raw.strip()
        if line.startswith("#") and not name:
            name=line.lstrip("#").strip()
        if "=" not in line or line.startswith(("#",";")):
            continue
        k,val=[x.strip() for x in line.split("=",1)]
        if k=="AllowedIPs":
            vals += [x.strip() for x in val.split(",") if x.strip()]
    fam=set()
    for x in vals:
        try:
            fam.add(ipaddress.ip_network(x, strict=False).version)
        except ValueError:
            fam.add(0)
    if 4 in fam: v4+=1
    if 6 in fam: v6+=1
    if 4 in fam and 6 in fam: dual+=1
    label=name or f"peer-{i}"
    print(f"{i:02d} {label}: {', '.join(vals) if vals else 'NO AllowedIPs'}")
print(f"TOTAL={len(blocks)} IPv4={v4} IPv6={v6} DUAL={dual}")
PY

echo
echo '===== WG INVENTORY (NO KEYS) ====='
python3 - <<'PY'
from pathlib import Path
import json
root=Path("/var/lib/vpn-ui/secure/wireguard")
server=json.loads((root/"server.json").read_text())
safe={
  "default_client_allowed_ips": server.get("default_client_allowed_ips"),
  "default_dns_servers": server.get("default_dns_servers"),
  "default_endpoint": server.get("default_endpoint"),
  "default_persistent_keepalive": server.get("default_persistent_keepalive"),
}
print("server.json =", json.dumps(safe, ensure_ascii=False))
clients=[]
for p in sorted((root/"clients").glob("*.json")):
    d=json.loads(p.read_text())
    clients.append({
      "name": d.get("name"),
      "enabled": d.get("enabled", True),
      "allocated_ips": d.get("allocated_ips") or [],
      "client_allowed_ips": d.get("client_allowed_ips") or [],
      "server_allowed_ips": d.get("server_allowed_ips") or [],
      "extra_allowed_ips": d.get("extra_allowed_ips") or [],
    })
for i,d in enumerate(clients,1):
    print(f"{i:02d} " + json.dumps(d, ensure_ascii=False))
print("TOTAL =", len(clients))
PY

echo
echo '===== UI HELPER DEFAULTS ====='
printf '%s\n' '{"op":"defaults"}' | /usr/local/bin/vpn-ui-manage-wg |
python3 -c 'import json,sys; d=json.load(sys.stdin); [print(f"{k} = {d[k]}") for k in ("ok","vpn_ip","client_allowed_ips","persistent_keepalive") if k in d]'

echo
echo '===== UI HELPER LIST IPV6 COVERAGE ====='
printf '%s\n' '{"op":"list"}' | /usr/local/bin/vpn-ui-manage-wg |
python3 -c '
import json,sys,ipaddress
d=json.load(sys.stdin)
peers=d.get("peers",[])
for i,p in enumerate(peers,1):
    values=[]
    for key in ("vpn_ip","server_allowed_ips","client_allowed_ips"):
        val=p.get(key)
        if isinstance(val,list):
            values += val
        elif isinstance(val,str) and val:
            values.append(val)
    has6=False
    for x in values:
        try:
            if ipaddress.ip_network(x, strict=False).version==6:
                has6=True
        except ValueError:
            pass
    print(f"{i:02d} {p.get(chr(110)+chr(97)+chr(109)+chr(101))}: vpn_ip={p.get(chr(118)+chr(112)+chr(110)+chr(95)+chr(105)+chr(112))} ipv6_visible={has6}")
print("TOTAL =", len(peers))
'

echo
echo '===== WG SETTINGS COMMAND ====='
/usr/local/bin/vpn-ui-wg-settings 2>/dev/null || true

echo
echo '===== NFTABLES WG/IPV6 RELEVANT ====='
nft list ruleset 2>/dev/null |
grep -Ei 'wg0|ip6|ipv6|masquerade|snat|forward' |
head -200 || true

echo
echo '===== SYSCTL PERSISTENCE (IPV6 FORWARDING) ====='
grep -RnsE '^[[:space:]]*net\.ipv6\.conf\.(all|default|wg0|ens3)\.forwarding[[:space:]]*='   /etc/sysctl.conf /etc/sysctl.d 2>/dev/null || true

echo
echo '========================================'
echo 'WG IPV6 AUDIT = PASS'
echo 'READ ONLY = YES'
echo 'NO WG/AWG/UI/NGINX CHANGES'
echo '========================================'
REMOTE
