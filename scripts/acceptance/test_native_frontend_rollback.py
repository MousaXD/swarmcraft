"""Contract tests for the local frontend replacement rehearsal."""
import copy
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import native_desktop_qa as qa
import native_frontend_rollback as rollback


def package_fixture():
    shipping = {
        "metadata": "Package: swarm-craft\nVersion: 0.5.1\nArchitecture: amd64\n",
        "package_sha256": "shipping",
        "files": {"desktop": "legacy", **{name: name for name in qa.SIDECARS}},
    }
    candidate = copy.deepcopy(shipping)
    candidate["package_sha256"] = "candidate"
    candidate["files"]["desktop"] = "svelte"
    return shipping, candidate


class RollbackContractTests(unittest.TestCase):
    def test_distinct_frontends_share_exact_package_identity_and_sidecars(self):
        shipping, candidate = package_fixture()
        rollback.validate_pair(shipping, candidate)
        for field, value in (
            ("metadata", "other-version"),
            ("package_sha256", shipping["package_sha256"]),
        ):
            altered = copy.deepcopy(candidate)
            altered[field] = value
            with self.assertRaises(qa.Failure):
                rollback.validate_pair(shipping, altered)
        for name in ("desktop", *qa.SIDECARS):
            altered = copy.deepcopy(candidate)
            altered["files"][name] = shipping["files"][name] if name == "desktop" else "different"
            with self.assertRaises(qa.Failure):
                rollback.validate_pair(shipping, altered)

    def test_process_exit_checks_group_and_exact_executable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            proc = root / "proc"
            proc.mkdir()
            entry = proc / "123"
            entry.mkdir()
            binary = root / "shipping/swarmcraft-desktop"
            binary.parent.mkdir()
            binary.touch()
            unrelated = root / "unrelated/swarmcraft"
            unrelated.parent.mkdir()
            unrelated.touch()
            stat = entry / "stat"
            exe = entry / "exe"
            with patch.object(rollback, "Path", side_effect=lambda value: proc if value == "/proc" else Path(value)):
                stat.write_text("123 (worker) S 1 42 42")
                exe.symlink_to(unrelated)
                self.assertFalse(rollback.packaged_processes_exited((binary,), 42))
                self.assertTrue(rollback.packaged_processes_exited((binary,), 43))
                exe.unlink()
                exe.symlink_to(qa.sidecar(binary, "swarmcraft"))
                self.assertFalse(rollback.packaged_processes_exited((binary,), 43))
                stat.write_text("123 (worker) Z 1 42 42")
                self.assertTrue(rollback.packaged_processes_exited((binary,), 42))


if __name__ == "__main__":
    unittest.main()
