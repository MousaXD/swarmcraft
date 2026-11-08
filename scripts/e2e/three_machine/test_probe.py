#!/usr/bin/env python3
"""Diagnostic redaction regression tests; no live network or simulated certification."""
import json
from pathlib import Path
import tempfile
import unittest

import node


class TestSanitizedDiagnostics(unittest.TestCase):
    def test_does_not_publish_addresses_tokens_or_error_details(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "connectivity-diagnostics-v1.json"
            secret = "SCINVITE-SENSITIVE-SECRET"
            path.write_text(json.dumps({
                "state": "relay_connected",
                "nat_status": "private",
                "direct_connectivity": False,
                "relay_connectivity": True,
                "hole_punch": {"failed": secret},
                "selected_relay": "/ip4/203.0.113.10/tcp/4001/p2p/SENSITIVE",
                "observed_public_address": "203.0.113.20",
                "last_failure": secret,
                "recent_failures": [{"detail": secret, "address": "203.0.113.20"}],
                "ipv4_available": True,
                "ipv6_available": False,
            }))
            summary = node.connectivity_summary(temp)
            self.assertEqual(summary["state"], "relay_connected")
            self.assertEqual(summary["hole_punch"], "failed")
            redacted = json.dumps(summary)
            for forbidden in (secret, "203.0.113", "/p2p/", "last_failure"):
                self.assertNotIn(forbidden, redacted)

    def test_missing_diagnostics_is_unavailable_not_fake_direct(self):
        with tempfile.TemporaryDirectory() as temp:
            self.assertEqual(node.connectivity_summary(temp), {"available": False})


if __name__ == "__main__":
    unittest.main()
