#!/usr/bin/env python3
"""Crash injection PID-reuse defenses; tests never signal actual processes."""
from pathlib import Path
import os
import signal
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import call, patch

import node


class TestPidfdCrashSafety(unittest.TestCase):
    WORLD = "ab" * 32

    def setUp(self):
        # PID-signaling tests control the separate disposable-profile guard;
        # actual approval-file safety has its own negative test suite.
        approval = patch.object(node, "approved_disposable_crash_profile", return_value=True)
        approval.start()
        self.addCleanup(approval.stop)

    def args(self, home):
        root = Path(home)
        binary = root / "swarmcraft-desktop"
        cli = root / "swarmcraft-cli"
        binary.touch()
        cli.touch()
        return SimpleNamespace(
            data_dir=str(root), binary=str(binary), cli=str(cli),
            world=self.WORLD, desktop_pid=12345, daemon_pid=12346,
            server_pid=12347, confirm="CRASH-ALICE-AUTHORITY",
        )

    def test_validated_processes_are_signaled_by_pinned_fd_never_numeric_pid(self):
        with tempfile.TemporaryDirectory() as temp:
            args = self.args(temp)
            with patch.object(node.os, "pidfd_open", side_effect=[101, 102, 103],
                              create=True) as opener, \
                 patch.object(node.signal, "pidfd_send_signal", create=True) as sender, \
                 patch.object(node.os, "close") as closer, \
                 patch.object(node, "isolated_process", return_value=True) as ownership, \
                 patch.object(node, "process_exited", return_value=True):
                result = node.crash(args)
            self.assertTrue(result["process_gone"])
            self.assertEqual(result["crashed_components"], ["desktop", "daemon", "server"])
            self.assertEqual(opener.call_args_list,
                             [call(12345, 0), call(12346, 0), call(12347, 0)])
            self.assertEqual(ownership.call_count, 3)
            self.assertEqual(sender.call_args_list, [
                call(101, signal.SIGSTOP), call(102, signal.SIGSTOP),
                call(103, signal.SIGSTOP), call(101, signal.SIGKILL),
                call(102, signal.SIGKILL), call(103, signal.SIGKILL),
            ])
            self.assertEqual(closer.call_count, 3)

    def test_missing_pidfd_capability_refuses_unsafe_numeric_pid_fallback(self):
        with tempfile.TemporaryDirectory() as temp:
            args = self.args(temp)
            fake_signal = SimpleNamespace(SIGSTOP=signal.SIGSTOP,
                                          SIGKILL=signal.SIGKILL,
                                          SIGCONT=signal.SIGCONT)
            with patch.object(node, "signal", fake_signal), \
                 patch.object(node.os, "kill") as numeric_kill:
                with self.assertRaises(SystemExit) as failure:
                    node.crash(args)
            self.assertEqual(failure.exception.code, 2)
            numeric_kill.assert_not_called()

    def test_missing_process_or_unowned_target_never_sends_a_signal(self):
        with tempfile.TemporaryDirectory() as temp:
            args = self.args(temp)
            with patch.object(node.os, "pidfd_open", side_effect=ProcessLookupError,
                              create=True), \
                 patch.object(node.signal, "pidfd_send_signal", create=True) as sender, \
                 patch.object(node.os, "kill") as numeric_kill:
                with self.assertRaises(SystemExit):
                    node.crash(args)
                sender.assert_not_called()
                numeric_kill.assert_not_called()

            with patch.object(node.os, "pidfd_open", side_effect=[201, 202, 203],
                              create=True), \
                 patch.object(node.signal, "pidfd_send_signal", create=True) as sender, \
                 patch.object(node.os, "close") as closer, \
                 patch.object(node, "isolated_process", side_effect=[True, False]):
                with self.assertRaises(SystemExit):
                    node.crash(args)
                sender.assert_not_called()
                self.assertEqual(closer.call_count, 3)

    def test_partial_freeze_failure_resumes_only_frozen_processes(self):
        with tempfile.TemporaryDirectory() as temp:
            args = self.args(temp)
            sent = []

            def send(fd, sig):
                sent.append((fd, sig))
                if fd == 302 and sig == signal.SIGSTOP:
                    raise PermissionError("synthetic signal failure")

            with patch.object(node.os, "pidfd_open", side_effect=[301, 302, 303],
                              create=True), \
                 patch.object(node.signal, "pidfd_send_signal",
                              side_effect=send, create=True), \
                 patch.object(node.os, "close"), \
                 patch.object(node, "isolated_process", return_value=True), \
                 patch.object(node.os, "kill") as numeric_kill:
                with self.assertRaises(SystemExit):
                    node.crash(args)
                numeric_kill.assert_not_called()
            self.assertEqual(sent, [
                (301, signal.SIGSTOP), (302, signal.SIGSTOP),
                (301, signal.SIGCONT),
            ])


if __name__ == "__main__":
    unittest.main()
