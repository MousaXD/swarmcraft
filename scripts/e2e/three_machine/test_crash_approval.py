#!/usr/bin/env python3
"""Tests for deliberate, disposable, world-bound authority crash approval.

No test opens a real process PID or sends a signal.
"""
import io
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

import node


WORLD = "ab" * 32
APPROVAL = ".swarmcraft-e2e-crash-approval"


def make_profile(folder, world=WORLD):
    root = Path(folder) / "swarmcraft-e2e"
    root.mkdir()
    root.chmod(0o700)
    marker = root / APPROVAL
    marker.write_text("SWARMCRAFT_E2E_CRASH\n" + world + "\n", encoding="ascii")
    marker.chmod(0o600)
    return root, marker


class TestDisposableAuthorityCrashApproval(unittest.TestCase):
    def test_exact_operator_marker_and_world_accepted(self):
        with tempfile.TemporaryDirectory() as temp:
            root, _ = make_profile(temp)
            self.assertTrue(node.approved_disposable_crash_profile(root, WORLD))
            self.assertTrue(node.approved_disposable_crash_profile(root, WORLD.upper()))

    def test_missing_wrong_world_and_wrong_file_format_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root, marker = make_profile(temp)
            self.assertFalse(node.approved_disposable_crash_profile(root, "cd" * 32))
            marker.unlink()
            self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))
            marker.write_text(WORLD + "\n", encoding="ascii")
            marker.chmod(0o600)
            self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))
            marker.write_text("SWARMCRAFT_E2E_CRASH\n" + WORLD + "\nEXTRA", encoding="ascii")
            self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))

    def test_root_operator_cannot_authorize_destructive_crash(self):
        with tempfile.TemporaryDirectory() as temp:
            root, _ = make_profile(temp)
            with patch.object(node.os, "geteuid", return_value=0):
                self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))

    def test_world_approval_not_accepted_in_normal_profile(self):
        with tempfile.TemporaryDirectory() as temp:
            root, _ = make_profile(temp)
            normal = root.with_name("swarmcraft")
            root.rename(normal)
            self.assertFalse(node.approved_disposable_crash_profile(normal, WORLD))

    def test_insecure_permissions_symlink_and_hardlink_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root, marker = make_profile(temp)
            marker.chmod(0o644)
            self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))
            marker.chmod(0o600)
            root.chmod(0o777)
            self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))
            root.chmod(0o700)
            alias = root / "alias"
            os.link(marker, alias)
            self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))
            alias.unlink()
            marker.unlink()
            outside = Path(temp) / "outside"
            outside.write_text("SWARMCRAFT_E2E_CRASH\n" + WORLD + "\n", encoding="ascii")
            marker.symlink_to(outside)
            self.assertFalse(node.approved_disposable_crash_profile(root, WORLD))

    def test_without_approval_crash_never_opens_a_pid_or_sends_a_signal(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "swarmcraft-e2e"
            root.mkdir()
            a = SimpleNamespace(
                confirm="CRASH-ALICE-AUTHORITY", world=WORLD,
                data_dir=str(root), desktop_pid=123, daemon_pid=124, server_pid=125,
                binary="/usr/bin/swarmcraft-desktop", cli="/usr/bin/swarmcraft",
            )
            with patch.object(node.os, "pidfd_open", create=True) as opener, \
                 patch.object(node.os, "kill") as numeric_signal, \
                 patch("sys.stdout", new_callable=io.StringIO) as printed:
                with self.assertRaises(SystemExit) as failure:
                    node.crash(a)
            self.assertEqual(failure.exception.code, 2)
            self.assertIn("disposable_world_bound_crash_approval_missing",
                          printed.getvalue())
            opener.assert_not_called()
            numeric_signal.assert_not_called()


if __name__ == "__main__":
    unittest.main()
