#!/usr/bin/python3
"""Offline tests for AWG 3.1 isolation; no root, network or device access."""
import base64
import contextlib
import importlib.util
import io
import pathlib
import runpy
import unittest

CORE = pathlib.Path(__file__).resolve().parents[1] / "deploy/libexec/vpn-ui-manage-core-v070"
WRAPPER = pathlib.Path(__file__).resolve().parents[1] / "deploy/libexec/vpn-ui-manage"
mod = runpy.run_path(str(CORE), run_name="awg31_isolation_test")
safe = runpy.run_path(str(WRAPPER), run_name="awg31_safe_settings_test")


class IsolationTests(unittest.TestCase):
    def test_provider_networks_and_files_do_not_overlap(self):
        configs = mod["PROVIDERS"]
        self.assertEqual(configs["wireguard"]["iface"], "wg0")
        self.assertEqual(configs["amneziawg"]["iface"], "awg0")
        self.assertEqual(configs["amneziawg31"]["iface"], "awg1")
        self.assertEqual(str(configs["amneziawg31"]["network"]), "10.88.88.0/24")
        for key in ("config", "inventory", "candidate", "stripped"):
            self.assertEqual(len({str(v[key]) for v in configs.values()}), 3)
        self.assertNotEqual(
            str(safe["PROVIDERS"]["amneziawg31"]["inventory"]),
            str(safe["PROVIDERS"]["amneziawg"]["inventory"]),
        )

    def test_awg31_requires_header_and_trailers(self):
        values = {key: ["16"] for key in ("S1", "S2", "S3", "S4")}
        values["HeaderProtectionKey"] = [base64.b64encode(bytes(range(32))).decode()]
        values["RandomTrailers"] = ["on"]
        mod["require_awg31_parameters"](values)

        bad = dict(values, S3=["8"])
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit):
            mod["require_awg31_parameters"](bad)

        bad = dict(values, RandomTrailers=["off"])
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit):
            mod["require_awg31_parameters"](bad)

        bad = dict(values)
        bad.pop("HeaderProtectionKey")
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit):
            mod["require_awg31_parameters"](bad)


if __name__ == "__main__":
    unittest.main()
