"""Static safety checks for host-scoped IHOR VPN UI deployment scripts.

No access to VPN servers, secrets, running interfaces or host namespaces.
"""
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1] / "deploy" / "ihor-v088"


class IhorDeploymentSafety(unittest.TestCase):
    def test_embedded_python_syntax(self):
        for name in ("audit.sh", "install-ui.sh"):
            text = (ROOT / name).read_text()
            fragments = re.findall(r"(?ms)^python3(?:\s+[^\n]+)?\s+<<'PY'\n(.*?)\nPY\s*$", text)
            self.assertTrue(fragments, name)
            for fragment in fragments:
                compile(fragment, f"{name}:embedded-python", "exec")

    def test_audit_is_read_only(self):
        script = (ROOT / "audit.sh").read_text()
        self.assertIn("READ ONLY", script)
        self.assertIn("Expected Tailscale IP: 100.88.88.1", script)
        operations = [line.strip() for line in script.splitlines()
                      if line.strip() and not line.lstrip().startswith("#")]
        for line in operations:
            self.assertNotRegex(line, r"\bsystemctl\s+(restart|stop|start|enable)\b")
            self.assertNotRegex(line, r"\btailscale\s+(up|down|set)\b")
            self.assertNotRegex(line, r"\bzerotier-cli\s+(join|leave)\b")

    def test_installer_is_bound_to_ihor_and_verified_binary(self):
        script = (ROOT / "install-ui.sh").read_text()
        self.assertIn("EXPECTED_HOST_IP=100.88.88.1", script)
        self.assertIn(
            "EXPECTED_SHA256=232bfda6d12508dd8ea07c768766cdef3529de54f544d77eb7f7d7c37ad6b41a",
            script,
        )
        self.assertIn('[[ $(tailscale ip -4) != "$EXPECTED_HOST_IP" ]]', script)
        self.assertIn("sha256sum -c -", script)
        self.assertIn("trap restore EXIT", script)
        self.assertIn("cp -a \"$DST\" \"$BACKUP/vpn-ui.before\"", script)
        self.assertIn("WG_BEFORE=", script)
        self.assertIn("AWG_BEFORE=", script)
        self.assertIn("WG_PEERS_BEFORE=", script)
        self.assertIn("AWG_PEERS_BEFORE=", script)
        self.assertIn("ZeroTier", script)
        self.assertIn("Tailscale", script)
        self.assertIn("inventory", script)

    def test_restart_is_limited_to_ui(self):
        script = (ROOT / "install-ui.sh").read_text()
        commands = re.findall(r"(?m)^\s*systemctl\s+restart\s+([^\n]+)", script)
        self.assertTrue(commands)
        self.assertEqual(set(commands), {"vpn-ui.service", "vpn-ui.service || true"})
        self.assertNotRegex(script, r"(?m)^\s*(?:cp|mv|install|sed|tee)\s+[^\n]*(?:/etc/wireguard|/etc/amnezia)")


if __name__ == "__main__":
    unittest.main()
