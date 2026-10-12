#!/usr/bin/env python3
"""Regression tests for the AWG 3.1 userspace peer config compatibility."""

from pathlib import Path
import runpy
import unittest


CORE = Path(__file__).resolve().parents[1] / "deploy/libexec/vpn-ui-manage-core-v070"
render_peer = runpy.run_path(str(CORE), run_name="awg31_render_test")["render_peer"]


def peer_record():
    return {
        "public_key": "fake-public-key-for-rendering-only",
        "preshared_key": "fake-psk-for-rendering-only",
        "server_allowed_ips": ["10.88.88.2/32"],
        "persistent_keepalive": "25",
        "server_peer_options": {"AdvancedSecurity": ["off"]},
    }


class PeerRenderingTests(unittest.TestCase):
    def test_awg31_omits_legacy_advanced_security(self):
        output = render_peer("amneziawg31", peer_record())
        self.assertNotIn("AdvancedSecurity", output)
        self.assertIn("AllowedIPs = 10.88.88.2/32", output)
        self.assertIn("PersistentKeepalive = 25", output)

    def test_awg20_still_emits_legacy_advanced_security(self):
        output = render_peer("amneziawg", peer_record())
        self.assertIn("AdvancedSecurity = off", output)

    def test_awg20_unchanged_when_option_not_explicit(self):
        record = peer_record()
        record["server_peer_options"] = {}
        self.assertIn("AdvancedSecurity = off", render_peer("amneziawg", record))
        self.assertNotIn("AdvancedSecurity", render_peer("amneziawg31", record))


if __name__ == "__main__":
    unittest.main()
