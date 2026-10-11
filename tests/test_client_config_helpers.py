#!/usr/bin/env python3
"""Offline checks for the WG/AWG 2.0 profile helpers imported from production.

These tests never load VPN inventories, keys or live interfaces.
"""
import ast
import pathlib
import stat
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
HELPERS = [
    "deploy/libexec/vpn-ui-client-config",
    "deploy/libexec/vpn-ui-privileged",
    "deploy/bin/vpn-ui-wg-client-config",
    "deploy/bin/vpn-ui-awg-client-config",
]


class ClientConfigHelperTests(unittest.TestCase):
    def test_scripts_exist_and_are_executable(self):
        for name in HELPERS:
            path = ROOT / name
            with self.subTest(script=name):
                self.assertTrue(path.is_file())
                self.assertTrue(path.stat().st_mode & stat.S_IXUSR)

    def test_bash_syntax(self):
        for name in HELPERS:
            path = ROOT / name
            with self.subTest(script=name):
                result = subprocess.run(
                    ["bash", "-n", str(path)],
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_embedded_python_syntax(self):
        source = (ROOT / HELPERS[0]).read_text().splitlines()
        for marker in ("PYWG", "PYAWG"):
            with self.subTest(marker=marker):
                starts = [
                    i for i, line in enumerate(source)
                    if "<<" + "'" + marker + "'" in line
                ]
                self.assertEqual(len(starts), 1)
                end = source.index(marker, starts[0] + 1)
                ast.parse("\n".join(source[starts[0] + 1:end]))

    def test_generator_rejects_bad_input_before_reading_inventory(self):
        path = ROOT / HELPERS[0]
        for args, request in [
            ([], ""),
            (["wg", "wg0"], "not-a-public-key\n"),
            (["awg", "awg0"], "not-a-public-key\n"),
            (["unsupported", "wg0"], "not-a-public-key\n"),
        ]:
            with self.subTest(args=args):
                result = subprocess.run(
                    ["bash", str(path), *args],
                    input=request,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 64)

    def test_privileged_helper_denies_unsupported_operations(self):
        path = ROOT / HELPERS[1]
        for args in ([], ["wg", "show", "wg0", "private-key"]):
            with self.subTest(args=args):
                result = subprocess.run(
                    ["bash", str(path), *args],
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 64)

    def test_wrappers_call_only_expected_entry_point(self):
        for path, tail in [
            (ROOT / HELPERS[2], "wg wg0"),
            (ROOT / HELPERS[3], "awg awg0"),
        ]:
            with self.subTest(wrapper=str(path)):
                lines = path.read_text().splitlines()
                self.assertIn("/usr/local/libexec/vpn-ui-client-config", "\n".join(lines))
                self.assertEqual(lines[-1].strip(), tail)


if __name__ == "__main__":
    unittest.main()
