#!/usr/bin/env python3
"""Offline checks of WG/AWG2 read-only wrappers and narrowly scoped sudoers."""

from pathlib import Path
import shutil
import stat
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "deploy/bin"
POLICIES = ROOT / "deploy/sudoers"

WRAPPER_ARGS = {
    "vpn-ui-wg-ro": 'wg "$@"',
    "vpn-ui-awg-ro": 'awg "$@"',
    "vpn-ui-wg-meta": "wg metadata wg0 names",
    "vpn-ui-awg-meta": "awg metadata awg0 names",
    "vpn-ui-wg-settings": "wg settings wg0 show",
    "vpn-ui-awg-settings": "awg settings awg0 show",
}
PRIVILEGED = "/usr/local/libexec/vpn-ui-privileged"
MANAGER = "/usr/local/libexec/vpn-ui-manage"
EXPORTER = "/usr/local/libexec/vpn-ui-client-config"

EXPECTED_POLICIES = {
    "vpn-ui": [
        f"{PRIVILEGED} wg show wg0 dump",
        f"{PRIVILEGED} awg show awg0 dump",
        f"{PRIVILEGED} wg metadata wg0 names",
        f"{PRIVILEGED} awg metadata awg0 names",
        f"{PRIVILEGED} wg settings wg0 show",
        f"{PRIVILEGED} awg settings awg0 show",
    ],
    "vpn-ui-manage": [
        f"{MANAGER} wireguard",
        f"{MANAGER} amneziawg",
    ],
    "vpn-ui-client-config": [
        f"{EXPORTER} wg wg0",
        f"{EXPORTER} awg awg0",
    ],
}


class ReadOnlyDeploymentTests(unittest.TestCase):
    def test_wrappers_are_executable_and_have_valid_syntax(self):
        for name in WRAPPER_ARGS:
            with self.subTest(wrapper=name):
                path = BIN / name
                self.assertTrue(path.is_file())
                self.assertTrue(path.stat().st_mode & stat.S_IXUSR)
                result = subprocess.run(
                    ["sh", "-n", str(path)],
                    capture_output=True, text=True, timeout=5,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_wrappers_use_only_expected_helper(self):
        for name, tail in WRAPPER_ARGS.items():
            with self.subTest(wrapper=name):
                source = (BIN / name).read_text().splitlines()
                self.assertEqual(source[0], "#!/bin/sh")
                self.assertEqual(len(source), 4)
                self.assertEqual(source[1], "exec /usr/bin/sudo -n " + chr(92))
                self.assertEqual(source[2], "    " + PRIVILEGED + " " + chr(92))
                self.assertEqual(source[3].strip(), tail)

    def test_sudoers_match_exact_command_allowlist(self):
        for name, commands in EXPECTED_POLICIES.items():
            with self.subTest(policy=name):
                path = POLICIES / name
                self.assertTrue(path.is_file())
                lines = [s.strip() for s in path.read_text().splitlines() if s.strip()]
                expected = [
                    f"vpn-ui ALL=(root) NOPASSWD: {command}"
                    for command in commands
                ]
                self.assertEqual(lines, expected)
                self.assertFalse(any("*" in s or "SETENV" in s for s in lines))

    def test_sudoers_syntax(self):
        executable = shutil.which("visudo")
        if not executable:
            self.skipTest("visudo not installed on this runner")
        for name in [*EXPECTED_POLICIES, "vpn-ui-awg31"]:
            with self.subTest(policy=name):
                result = subprocess.run(
                    [executable, "-cf", str(POLICIES / name)],
                    capture_output=True, text=True, timeout=5,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_historical_sudoers_backups_are_not_packaged(self):
        self.assertFalse([
            p.name for p in POLICIES.iterdir()
            if p.name.endswith((".bak", ".save", "~")) or ".bak-" in p.name
        ])


if __name__ == "__main__":
    unittest.main()
