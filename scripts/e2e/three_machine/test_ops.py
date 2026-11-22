#!/usr/bin/env python3
"""Provisioning negative controls; never contacts a remote host."""
from pathlib import Path
import tempfile
import unittest

import ops
import run as gate


class TestProvisionSafety(unittest.TestCase):
    def test_only_dedicated_opt_staging(self):
        self.assertEqual(ops.checked_destination("/opt/swarmcraft-e2e/candidate.deb"),
                         "/opt/swarmcraft-e2e/candidate.deb")
        for path in ("/tmp/candidate.deb", "/home/e2e/world/candidate.deb",
                     "/opt/swarmcraft-e2e/../../etc/candidate.deb",
                     "/opt/swarmcraft-e2e/candidate.jar"):
            with self.subTest(path=path), self.assertRaises(gate.Blocked):
                ops.checked_destination(path)

    def test_provision_requires_explicit_approval_before_io(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(gate.Blocked, "explicit_install_approval_required"):
                ops.provision({}, "/nonexistent.deb", Path(temp), None)

    def test_cleanup_cannot_touch_file_without_approval(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(gate.Blocked, "explicit_cleanup_approval_required"):
                ops.cleanup_stage({}, Path(temp), None)


if __name__ == "__main__":
    unittest.main()
