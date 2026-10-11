#!/usr/bin/env python3
"""Stage vpn-ui release files in a disposable directory without root or VPN access.

This is NOT a production installer. It never invokes sudo, starts systemd,
creates interfaces, edits real /etc or /usr/local, or imports private configs.
"""
import configparser
import hashlib
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

PACKAGES = (
    ("deploy/bin", "usr/local/bin", 0o755),
    ("deploy/libexec", "usr/local/libexec", 0o755),
    ("deploy/sudoers", "etc/sudoers.d", 0o440),
    ("deploy/systemd", "etc/systemd/system", 0o644),
    ("deploy/nginx", "etc/nginx/conf.d", 0o644),
)
EXPECTED = (
    "usr/local/bin/vpn-ui-wg-ro",
    "usr/local/bin/vpn-ui-wg-meta",
    "usr/local/bin/vpn-ui-wg-settings",
    "usr/local/bin/vpn-ui-wg-client-config",
    "usr/local/bin/vpn-ui-manage-wg",
    "usr/local/bin/vpn-ui-awg-ro",
    "usr/local/bin/vpn-ui-awg-meta",
    "usr/local/bin/vpn-ui-awg-settings",
    "usr/local/bin/vpn-ui-awg-client-config",
    "usr/local/bin/vpn-ui-manage-awg",
    "usr/local/bin/vpn-ui-awg31-meta",
    "usr/local/bin/vpn-ui-awg31-status",
    "usr/local/bin/vpn-ui-awg31-settings",
    "usr/local/bin/vpn-ui-awg31-client-config",
    "usr/local/bin/vpn-ui-manage-awg31",
    "usr/local/libexec/vpn-ui-manage",
    "usr/local/libexec/vpn-ui-manage-core-v070",
    "usr/local/libexec/vpn-ui-privileged",
    "usr/local/libexec/vpn-ui-client-config",
    "usr/local/libexec/vpn-ui-awg31-status",
    "usr/local/libexec/vpn-ui-awg31-settings",
    "usr/local/libexec/vpn-ui-awg31-client-config",
    "usr/local/libexec/vpn-ui-awg31-net",
    "etc/sudoers.d/vpn-ui",
    "etc/sudoers.d/vpn-ui-manage",
    "etc/sudoers.d/vpn-ui-client-config",
    "etc/sudoers.d/vpn-ui-awg31",
    "etc/systemd/system/vpn-ui-awg31.service",
)
EXTERNAL_ENGINE = "/usr/local/libexec/amneziawg-go-3.1.20260828"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    if not (ROOT / "deploy/bin").is_dir():
        raise SystemExit("Run from a complete vpn-ui source checkout")
    count = 0
    with tempfile.TemporaryDirectory(prefix="vpn-ui-stage-only-") as temp:
        stage = Path(temp)
        for source, dest, mode in PACKAGES:
            origin = ROOT / source
            if not origin.is_dir():
                continue
            for file in sorted(origin.rglob("*")):
                if not file.is_file() or file.is_symlink():
                    continue
                relative = file.relative_to(origin)
                target = stage / dest / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(file, target)
                target.chmod(mode)
                assert target.stat().st_mode & 0o777 == mode
                assert digest(file) == digest(target)
                count += 1

        rust = ROOT / "target/release/vpn-ui"
        if not rust.is_file():
            raise SystemExit("Build target/release/vpn-ui before staging")
        target = stage / "usr/local/bin/vpn-ui"
        shutil.copyfile(rust, target)
        target.chmod(0o755)
        assert digest(rust) == digest(target)
        count += 1

        for name in EXPECTED:
            if not (stage / name).is_file():
                raise SystemExit(f"Missing deployment prerequisite: {name}")

        for name in ("vpn-ui", "vpn-ui-manage", "vpn-ui-client-config", "vpn-ui-awg31"):
            policy = stage / "etc/sudoers.d" / name
            assert policy.stat().st_mode & 0o777 == 0o440
            visudo = shutil.which("visudo")
            if visudo:
                subprocess.run([visudo, "-cf", str(policy)], check=True,
                               capture_output=True, text=True, timeout=10)

        unit = configparser.ConfigParser(interpolation=None)
        unit.read(stage / "etc/systemd/system/vpn-ui-awg31.service")
        if not unit.has_section("Service"):
            raise SystemExit("AWG31 service section missing")
        for key in ("ExecStart", "ExecStartPost", "ExecStopPost"):
            cmd = unit["Service"][key].split()[0]
            if cmd == EXTERNAL_ENGINE:
                continue
            if not cmd.startswith("/usr/local/libexec/"):
                raise SystemExit(f"Unexpected AWG31 executable: {key}: {cmd}")
            if not (stage / cmd.lstrip("/")).is_file():
                raise SystemExit(f"Missing AWG31 helper in stage: {cmd}")

        if (stage / EXTERNAL_ENGINE.lstrip("/")).exists():
            raise SystemExit("External upstream AWG31 binary must not be packaged")

        print(f"OFFLINE STAGING PASS: {count} files copied and hashes verified")
        print("WG, AWG2, AWG31 helpers and all sudoers present")
        print("AWG31 engine is correctly treated as an external prerequisite")
        print("No real VPN configuration read; no root, sudo, systemd or network operations")


if __name__ == "__main__":
    main()
