#!/usr/bin/python3
"""Unit tests for the isolated AWG1 status reader. No VPN devices required."""
import runpy
import pathlib
import unittest

FILE = pathlib.Path(__file__).resolve().parents[1] / "deploy/libexec/vpn-ui-awg31-status"
redact = runpy.run_path(str(FILE), run_name="awg31_status_test")["sanitized_dump"]


class StatusTests(unittest.TestCase):
    def test_private_and_preshared_keys_are_removed(self):
        text = (
            "SUPERPRIVATE\tSERVERPUBLIC\t8444\toff\n"
            "PEERPUB\tPSKSECRET\t(none)\t10.88.88.2/32\t123\t456\t789\t25\n"
        )
        result = redact(text)
        self.assertEqual(
            result,
            "(hidden)\tSERVERPUBLIC\t8444\toff\n"
            "PEERPUB\t(hidden)\t(none)\t10.88.88.2/32\t123\t456\t789\t25\n"
        )
        self.assertNotIn("SUPERPRIVATE", result)
        self.assertNotIn("PSKSECRET", result)

    def test_bad_dump_is_rejected(self):
        for text in ("", "secret", "private\tpublic\t8444\toff\npeer\tpsk\n"):
            with self.assertRaises(ValueError):
                redact(text)


if __name__ == "__main__":
    unittest.main()
