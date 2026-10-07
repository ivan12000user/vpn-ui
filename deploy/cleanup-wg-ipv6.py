#!/usr/bin/env python3
import datetime
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess
import sys

WG_CONF = Path("/etc/wireguard/wg0.conf")
AWG_CONF = Path("/etc/amnezia/amneziawg/awg0.conf")
WG_ROOT = Path("/var/lib/vpn-ui/secure/wireguard")
AWG_ROOT = Path("/var/lib/vpn-ui/secure/amneziawg")
EXP_NET = ipaddress.ip_network("fd66:66:66::/64")
EXP_ADDR = "fd66:66:66::1/64"


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


def out(args):
    return run(args).stdout


def sha_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sha_text(value):
    return hashlib.sha256(value.encode()).hexdigest()


def helper(path, request):
    p = run(
        [path],
        input_text=json.dumps(request, separators=(",", ":")) + "\n",
    )
    data = json.loads(p.stdout)
    if data.get("ok") is not True:
        raise RuntimeError(f"helper failure: {path}")
    return data


def persistent_peer_count(path):
    text = path.read_text()
    return len(re.findall(r"(?m)^\s*\[Peer\]\s*$", text))


def live_peers(tool, iface):
    return [x for x in out([tool, "show", iface, "peers"]).splitlines() if x.strip()]


def inventory_count(path):
    data = helper(path, {"op": "list"})
    peers = data.get("peers", [])
    return int(data.get("count", len(peers)))


def core_state():
    ens3 = (
        out(["ip", "-6", "-o", "addr", "show", "dev", "ens3", "scope", "global"])
        + out(["ip", "-6", "route", "show", "default"])
    )
    return {
        "WG_CONFIG_SHA": sha_file(WG_CONF),
        "AWG_CONFIG_SHA": sha_file(AWG_CONF),
        "WG_SERVER_SHA": sha_file(WG_ROOT / "server.json"),
        "AWG_SERVER_SHA": sha_file(AWG_ROOT / "server.json"),
        "WG_TS": out([
            "systemctl", "show", "wg-quick@wg0.service",
            "-p", "ActiveEnterTimestampMonotonic", "--value",
        ]).strip(),
        "AWG_TS": out([
            "systemctl", "show", "awg-quick@awg0.service",
            "-p", "ActiveEnterTimestampMonotonic", "--value",
        ]).strip(),
        "WG_COUNTS": (
            inventory_count("/usr/local/bin/vpn-ui-manage-wg"),
            persistent_peer_count(WG_CONF),
            len(live_peers("wg", "wg0")),
        ),
        "AWG_COUNTS": (
            inventory_count("/usr/local/bin/vpn-ui-manage-awg"),
            persistent_peer_count(AWG_CONF),
            len(live_peers("awg", "awg0")),
        ),
        "WG_PEERS_SHA": sha_text("\n".join(live_peers("wg", "wg0")) + "\n"),
        "AWG_PEERS_SHA": sha_text("\n".join(live_peers("awg", "awg0")) + "\n"),
        "ENS3_IPV6_SHA": sha_text(ens3),
    }


def print_state(state):
    for k, v in state.items():
        if isinstance(v, tuple):
            v = "/".join(str(x) for x in v)
        print(f"{k}={v}")


def service_checks():
    for unit in (
        "wg-quick@wg0.service",
        "awg-quick@awg0.service",
        "vpn-ui.service",
        "nginx",
    ):
        run(["systemctl", "is-active", "--quiet", unit])
    run(["nginx", "-t"])


def inventory_ipv4_only():
    bad = []
    server = json.loads((WG_ROOT / "server.json").read_text())

    for key in ("default_client_allowed_ips", "default_dns_servers"):
        for value in server.get(key) or []:
            try:
                obj = (
                    ipaddress.ip_network(value, strict=False)
                    if "/" in value
                    else ipaddress.ip_address(value)
                )
            except ValueError:
                continue
            if obj.version == 6:
                bad.append(f"server.json {key}={value}")

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
                    continue
                if obj.version == 6:
                    bad.append(f"{path.name} {key}={value}")

    if bad:
        raise RuntimeError(
            "WG inventory contains IPv6; refusing cleanup: " + "; ".join(bad)
        )


def parse_allowed_ips(text):
    rows = []
    for raw in text.splitlines():
        fields = raw.split()
        if len(fields) < 2:
            continue
        public = fields[0]
        values = []
        for token in fields[1:]:
            for value in token.split(","):
                value = value.strip()
                if value and value != "(none)":
                    values.append(value)
        rows.append((public, values))
    return rows


def classify_live_ipv6(rows):
    experimental = []
    unexpected = []

    for public, values in rows:
        exp = []
        for value in values:
            try:
                net = ipaddress.ip_network(value, strict=False)
            except ValueError:
                raise RuntimeError(f"invalid live AllowedIP: {value}")
            if net.version != 6:
                continue
            if net.subnet_of(EXP_NET):
                exp.append(value)
            else:
                unexpected.append(value)
        if exp:
            experimental.append((public, values, exp))

    if unexpected:
        raise RuntimeError(
            "unexpected live WG IPv6 AllowedIPs: " + ", ".join(unexpected)
        )
    return experimental


def nat_chain():
    p = run(
        ["nft", "-a", "list", "chain", "ip6", "nat", "POSTROUTING"],
        check=False,
    )
    return p.stdout if p.returncode == 0 else ""


def nat_handles(text):
    handles = []
    for line in text.splitlines():
        if (
            'oifname "ens3"' in line
            and "ip6 saddr fd66:66:66::/64" in line
            and "masquerade" in line
        ):
            m = re.search(r"# handle (\d+)", line)
            if not m:
                raise RuntimeError("matching NAT66 rule has no nft handle")
            handles.append(m.group(1))
    return handles


def has_exp_addr():
    return EXP_ADDR in out(["ip", "-6", "-o", "addr", "show", "dev", "wg0"])


def has_exp_nat():
    return bool(nat_handles(nat_chain()))


def write_backup(path, name, value):
    p = path / name
    p.write_text(value)
    os.chmod(p, 0o600)


def restore_allowed(plan):
    for public, original, _keep, _removed in plan:
        if original:
            run([
                "wg", "set", "wg0", "peer", public,
                "allowed-ips", ",".join(original),
            ])


def main():
    print("===== WG IPV6 CLEANUP PRECHECK =====")
    service_checks()

    if "fd66:66:66" in WG_CONF.read_text().lower():
        raise RuntimeError(
            "persistent wg0.conf contains experimental fd66:66:66 state"
        )

    inventory_ipv4_only()

    baseline = core_state()
    print()
    print("===== CAPTURE BASELINE =====")
    print_state(baseline)

    if len(set(baseline["WG_COUNTS"])) != 1:
        raise RuntimeError(f"WG inventory/config/live drift: {baseline['WG_COUNTS']}")
    if len(set(baseline["AWG_COUNTS"])) != 1:
        raise RuntimeError(f"AWG inventory/config/live drift: {baseline['AWG_COUNTS']}")

    allowed_before_text = out(["wg", "show", "wg0", "allowed-ips"])
    allowed_before = parse_allowed_ips(allowed_before_text)
    experimental = classify_live_ipv6(allowed_before)

    print()
    print("===== VALIDATE CURRENT DESIGN =====")
    print("persistent wg0.conf = IPv4-only")
    print("WG inventory       = IPv4-only")
    print(f"live WG peers with experimental IPv6 = {len(experimental)}")
    print(
        "live WG experimental IPv6 AllowedIPs = "
        + str(sum(len(x[2]) for x in experimental))
    )
    print("precheck           = PASS")

    handles = nat_handles(nat_chain())
    if len(handles) > 1:
        raise RuntimeError("more than one matching experimental NAT66 rule")

    had_addr = has_exp_addr()
    had_nat = len(handles) == 1

    stamp = datetime.datetime.now().strftime("%Y%m%d_%H%M%S")
    backup = Path(f"/root/wg-ipv6-cleanup-backup-{stamp}")
    backup.mkdir(mode=0o700)

    write_backup(
        backup, "wg0-ipv6.before.txt",
        out(["ip", "-6", "addr", "show", "dev", "wg0"]),
    )
    write_backup(
        backup, "ens3-ipv6.before.txt",
        out(["ip", "-6", "addr", "show", "dev", "ens3", "scope", "global"]),
    )
    write_backup(
        backup, "ip6-routes.before.txt",
        out(["ip", "-6", "route", "show", "table", "all"]),
    )
    write_backup(backup, "ip6-nat.before.txt", nat_chain())
    write_backup(backup, "wg0-allowed-ips.before.txt", allowed_before_text)
    write_backup(
        backup, "hashes.before.txt",
        f"{baseline['WG_CONFIG_SHA']}  {WG_CONF}\n"
        f"{baseline['AWG_CONFIG_SHA']}  {AWG_CONF}\n",
    )

    plan = []
    for public, original, removed in experimental:
        keep = []
        for value in original:
            net = ipaddress.ip_network(value, strict=False)
            if net.version == 6 and net.subnet_of(EXP_NET):
                continue
            keep.append(value)
        if not keep:
            raise RuntimeError("refusing to leave WG peer with empty AllowedIPs")
        plan.append((public, original, keep, removed))

    plan_public = [
        {
            "peer": index + 1,
            "original_count": len(original),
            "kept": keep,
            "removed": removed,
        }
        for index, (_public, original, keep, removed) in enumerate(plan)
    ]
    write_backup(
        backup, "cleanup-plan.json",
        json.dumps(plan_public, ensure_ascii=False, indent=2) + "\n",
    )

    print()
    print("===== REMOVE EXPERIMENTAL WG IPV6 ONLY =====")
    print(f"cleanup backup = {backup}")
    print(f"peers scheduled for IPv6 cleanup = {len(plan)}")

    try:
        for public, _original, keep, _removed in plan:
            run([
                "wg", "set", "wg0", "peer", public,
                "allowed-ips", ",".join(keep),
            ])

        if had_addr:
            run(["ip", "-6", "addr", "del", EXP_ADDR, "dev", "wg0"])

        if had_nat:
            run([
                "nft", "delete", "rule", "ip6", "nat",
                "POSTROUTING", "handle", handles[0],
            ])

        current = parse_allowed_ips(out(["wg", "show", "wg0", "allowed-ips"]))
        if classify_live_ipv6(current):
            raise RuntimeError("experimental IPv6 AllowedIPs still present")
        if has_exp_addr():
            raise RuntimeError("experimental WG IPv6 address still present")
        if has_exp_nat():
            raise RuntimeError("experimental NAT66 rule still present")

        service_checks()

        settings = helper(
            "/usr/local/bin/vpn-ui-manage-wg",
            {"op": "settings_get"},
        ).get("settings", {})
        if settings.get("client_allowed_ips") != ["10.66.66.0/24"]:
            raise RuntimeError("WG UI defaults are not IPv4-only")

        wg_list = helper(
            "/usr/local/bin/vpn-ui-manage-wg",
            {"op": "list"},
        )
        for peer in wg_list.get("peers", []):
            if ":" in str(peer.get("vpn_ip") or ""):
                raise RuntimeError("vpn-ui returned IPv6 WG vpn_ip")

        if "10.66.66.1/24" not in out([
            "ip", "-4", "-o", "addr", "show", "dev", "wg0",
        ]):
            raise RuntimeError("WG IPv4 address missing")

        health = run([
            "curl", "-sS", "-o", "/dev/null", "-w", "%{http_code}",
            "http://127.0.0.1:8090/healthz",
        ]).stdout
        if health != "200":
            raise RuntimeError(f"vpn-ui health HTTP {health}")

        auth = run([
            "curl", "-k", "-sS", "-o", "/dev/null", "-w", "%{http_code}",
            "https://127.0.0.1/",
        ]).stdout
        if auth != "401":
            raise RuntimeError(f"nginx HTTPS auth HTTP {auth}")

        after = core_state()
        if after != baseline:
            raise RuntimeError(
                "protected VPN/public-IPv6 state changed unexpectedly:\n"
                + json.dumps(
                    {"before": baseline, "after": after},
                    ensure_ascii=False,
                    indent=2,
                )
            )

    except Exception:
        print("CLEANUP FAILED — ROLLBACK", file=sys.stderr)
        try:
            restore_allowed(plan)
        except Exception as exc:
            print(f"rollback AllowedIPs failed: {exc}", file=sys.stderr)

        try:
            if had_addr and not has_exp_addr():
                run(["ip", "-6", "addr", "add", EXP_ADDR, "dev", "wg0"])
        except Exception as exc:
            print(f"rollback wg0 IPv6 address failed: {exc}", file=sys.stderr)

        try:
            if had_nat and not has_exp_nat():
                run([
                    "nft", "add", "rule", "ip6", "nat", "POSTROUTING",
                    "oifname", "ens3", "ip6", "saddr",
                    str(EXP_NET), "masquerade",
                ])
        except Exception as exc:
            print(f"rollback NAT66 failed: {exc}", file=sys.stderr)

        raise

    print(f"removed live wg0 IPv6 address = {int(had_addr)}")
    print(f"removed experimental NAT66    = {int(had_nat)}")
    print(f"changed peer AllowedIPs       = {len(plan)}")

    print()
    print("===== FINAL IPV6 STATE =====")
    wg6 = out(["ip", "-6", "-o", "addr", "show", "dev", "wg0"]).strip()
    print("wg0 IPv6 =", wg6 or "none")

    final_v6 = []
    for _public, values in parse_allowed_ips(
        out(["wg", "show", "wg0", "allowed-ips"])
    ):
        for value in values:
            net = ipaddress.ip_network(value, strict=False)
            if net.version == 6:
                final_v6.append(value)
    print("WG live IPv6 AllowedIPs =", ", ".join(final_v6) if final_v6 else "none")

    exp_nat_lines = [
        line.strip()
        for line in nat_chain().splitlines()
        if "fd66:66:66::/64" in line
    ]
    print("experimental NAT66 =", "present" if exp_nat_lines else "none")

    print("public ens3 IPv6:")
    print(out(["ip", "-6", "-o", "addr", "show", "dev", "ens3", "scope", "global"]).strip())
    print(out(["ip", "-6", "route", "show", "default"]).strip())

    print()
    print("========================================")
    print("WG IPV6 CLEANUP = PASS")
    print("WG INTERNAL NETWORK = IPv4 ONLY (10.66.66.0/24)")
    print("EXPERIMENTAL fd66:66:66::/64 = REMOVED")
    print("PUBLIC VPS IPv6 = PRESERVED")
    print("TAILSCALE IPv6 = PRESERVED")
    print("AWG = UNCHANGED")
    print("WG0 / AWG0 = NOT RESTARTED")
    print("VPN-UI = HEALTHY AND IPv4-ONLY FOR WG")
    print("========================================")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)
