#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"

echo '===== VPN-UI PRODUCTION VERIFY (READ ONLY) ====='
echo "remote = $REMOTE"

ssh "$REMOTE" python3 - <<'PY'
from pathlib import Path
import ipaddress
import json
import re
import subprocess
import sys

WG_CONF = Path("/etc/wireguard/wg0.conf")
AWG_CONF = Path("/etc/amnezia/amneziawg/awg0.conf")
WG_ROOT = Path("/var/lib/vpn-ui/secure/wireguard")
AWG_ROOT = Path("/var/lib/vpn-ui/secure/amneziawg")
EXP = ipaddress.ip_network("fd66:66:66::/64")


def run(args, *, input_text=None, check=True):
    p = subprocess.run(
        args,
        input=input_text,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if check and p.returncode != 0:
        msg = p.stderr.strip() or p.stdout.strip() or f"exit {p.returncode}"
        raise RuntimeError(f"{args[0]} failed: {msg}")
    return p


def output(args):
    return run(args).stdout


def helper(path, request):
    p = run(
        [path],
        input_text=json.dumps(request, separators=(",", ":")) + "\n",
    )
    data = json.loads(p.stdout)
    if data.get("ok") is not True:
        raise RuntimeError(f"helper returned failure: {path}")
    return data


def config_peer_count(path):
    return len(
        re.findall(
            r"(?m)^\s*\[Peer\]\s*$",
            path.read_text(),
        )
    )


def live_peer_count(tool, iface):
    return len(
        [
            x
            for x in output([tool, "show", iface, "peers"]).splitlines()
            if x.strip()
        ]
    )


def inventory_count(helper_path):
    d = helper(helper_path, {"op": "list"})
    peers = d.get("peers", [])
    return int(d.get("count", len(peers)))


def parse_allowed_ips(text):
    result = []
    for raw in text.splitlines():
        fields = raw.split()
        if len(fields) < 2:
            continue
        values = []
        for token in fields[1:]:
            for value in token.split(","):
                value = value.strip()
                if value and value != "(none)":
                    values.append(value)
        result.append(values)
    return result


def ensure_inventory_ipv4_only():
    server = json.loads((WG_ROOT / "server.json").read_text())
    bad = []

    for key in ("default_client_allowed_ips",):
        for value in server.get(key) or []:
            try:
                net = ipaddress.ip_network(value, strict=False)
            except ValueError:
                bad.append(f"server.json {key} invalid")
                continue
            if net.version == 6:
                bad.append(f"server.json {key} contains IPv6")

    for path in sorted((WG_ROOT / "clients").glob("*.json")):
        data = json.loads(path.read_text())
        for key in (
            "allocated_ips",
            "client_allowed_ips",
            "server_allowed_ips",
            "extra_allowed_ips",
        ):
            for value in data.get(key) or []:
                try:
                    obj = (
                        ipaddress.ip_network(value, strict=False)
                        if "/" in value
                        else ipaddress.ip_address(value)
                    )
                except ValueError:
                    bad.append(f"{path.name} {key} invalid")
                    continue
                if obj.version == 6:
                    bad.append(f"{path.name} {key} contains IPv6")

    if bad:
        raise RuntimeError("; ".join(bad))


def service_state(unit):
    active = run(
        ["systemctl", "is-active", unit],
        check=False,
    ).stdout.strip() or "unknown"
    enabled = run(
        ["systemctl", "is-enabled", unit],
        check=False,
    ).stdout.strip() or "unknown"
    return active, enabled


try:
    print()
    print("===== SERVICES =====")
    for unit in (
        "wg-quick@wg0.service",
        "awg-quick@awg0.service",
        "vpn-ui.service",
        "nginx",
    ):
        run(["systemctl", "is-active", "--quiet", unit])
        print(f"{unit} = active")

    run(["nginx", "-t"])
    print("nginx -t = PASS")

    print()
    print("===== STRUCTURE =====")
    wg_counts = (
        inventory_count("/usr/local/bin/vpn-ui-manage-wg"),
        config_peer_count(WG_CONF),
        live_peer_count("wg", "wg0"),
    )
    awg_counts = (
        inventory_count("/usr/local/bin/vpn-ui-manage-awg"),
        config_peer_count(AWG_CONF),
        live_peer_count("awg", "awg0"),
    )

    print("WG inventory/config/live = " + "/".join(map(str, wg_counts)))
    print("AWG inventory/config/live = " + "/".join(map(str, awg_counts)))

    if len(set(wg_counts)) != 1:
        raise RuntimeError(f"WG count mismatch: {wg_counts}")
    if len(set(awg_counts)) != 1:
        raise RuntimeError(f"AWG count mismatch: {awg_counts}")

    print()
    print("===== WG IPV4-ONLY DESIGN =====")
    text = WG_CONF.read_text()
    if "fd66:66:66" in text.lower():
        raise RuntimeError("persistent wg0.conf contains experimental ULA")

    wg4 = output(["ip", "-4", "-o", "addr", "show", "dev", "wg0"])
    if "10.66.66.1/24" not in wg4:
        raise RuntimeError("wg0 missing 10.66.66.1/24")

    wg6 = output(["ip", "-6", "-o", "addr", "show", "dev", "wg0"]).strip()
    if wg6:
        raise RuntimeError("wg0 unexpectedly has IPv6 address")

    ensure_inventory_ipv4_only()

    bad_live = []
    for values in parse_allowed_ips(output(["wg", "show", "wg0", "allowed-ips"])):
        for value in values:
            net = ipaddress.ip_network(value, strict=False)
            if net.version == 6:
                bad_live.append(value)
    if bad_live:
        raise RuntimeError("live WG peers contain IPv6 AllowedIPs")

    nft = run(
        ["nft", "list", "table", "ip6", "nat"],
        check=False,
    ).stdout
    if "fd66:66:66::/64" in nft:
        raise RuntimeError("experimental fd66 NAT66 rule still exists")

    print("wg0 address = 10.66.66.1/24")
    print("wg0 IPv6 = none")
    print("WG live IPv6 AllowedIPs = none")
    print("WG experimental NAT66 = none")
    print("WG inventory IPv6 = none")

    print()
    print("===== SAVED DEFAULTS =====")
    wg_settings = helper(
        "/usr/local/bin/vpn-ui-manage-wg",
        {"op": "settings_get"},
    ).get("settings", {})
    awg_settings = helper(
        "/usr/local/bin/vpn-ui-manage-awg",
        {"op": "settings_get"},
    ).get("settings", {})

    if wg_settings.get("client_allowed_ips") != ["10.66.66.0/24"]:
        raise RuntimeError("unexpected WG default Client AllowedIPs")
    if awg_settings.get("client_allowed_ips") != ["10.77.77.0/24"]:
        raise RuntimeError("unexpected AWG default Client AllowedIPs")

    print("WG Client AllowedIPs default = 10.66.66.0/24")
    print("AWG Client AllowedIPs default = 10.77.77.0/24")

    print()
    print("===== PUBLIC IPV6 =====")
    wan6 = output(
        ["ip", "-6", "-o", "addr", "show", "dev", "ens3", "scope", "global"]
    ).strip()
    default6 = output(["ip", "-6", "route", "show", "default"]).strip()
    if not wan6:
        raise RuntimeError("ens3 has no global IPv6 address")
    if not default6:
        raise RuntimeError("IPv6 default route missing")
    print(wan6)
    print(default6)

    print()
    print("===== WEB =====")
    health = run(
        [
            "curl", "-sS", "-o", "/dev/null",
            "-w", "%{http_code}",
            "http://127.0.0.1:8090/healthz",
        ]
    ).stdout
    auth = run(
        [
            "curl", "-k", "-sS", "-o", "/dev/null",
            "-w", "%{http_code}",
            "https://127.0.0.1/",
        ]
    ).stdout
    if health != "200":
        raise RuntimeError(f"vpn-ui health returned HTTP {health}")
    if auth != "401":
        raise RuntimeError(f"Nginx HTTPS entry returned HTTP {auth}, expected 401")
    print("vpn-ui health = HTTP 200")
    print("Nginx HTTPS unauthenticated = HTTP 401")

    print()
    print("===== LEGACY UI SERVICES (INFORMATION ONLY) =====")
    for unit in (
        "wgui.path",
        "wgui.service",
        "wireguard-ui-daemon.service",
        "amneziawg-web.service",
    ):
        active, enabled = service_state(unit)
        print(f"{unit}: active={active} enabled={enabled}")

    print()
    print("========================================")
    print("VPN-UI PRODUCTION VERIFY = PASS")
    print("MODE = READ ONLY")
    print("WG = IPv4-only")
    print("AWG = unchanged")
    print("PUBLIC VPS IPv6 = present")
    print("========================================")

except Exception as exc:
    print()
    print("========================================", file=sys.stderr)
    print("VPN-UI PRODUCTION VERIFY = FAIL", file=sys.stderr)
    print(f"ERROR: {exc}", file=sys.stderr)
    print("NO CHANGES WERE MADE", file=sys.stderr)
    print("========================================", file=sys.stderr)
    raise SystemExit(1)
PY
