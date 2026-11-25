"""Offline regression tests for the local-first Actions policy."""

from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from check_workflow_policy import check_local_first, check_native_signing_receipts


class LocalFirstPolicyTests(unittest.TestCase):
    def reject_change(self, name, old, new):
        path = ROOT / ".github/workflows" / name
        text = path.read_text()
        self.assertIn(old, text)
        with self.assertRaises(SystemExit):
            check_local_first(path, text.replace(old, new, 1))

    def test_all_retained_workflows_pass(self):
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            with self.subTest(path=path.name):
                check_local_first(path, path.read_text())

    def test_production_native_receipts_cannot_be_removed(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        check_native_signing_receipts(release)
        for script in ("verify_windows_signing.ps1", "verify_macos_signing.sh"):
            with self.subTest(script=script), self.assertRaises(SystemExit):
                check_native_signing_receipts(release.replace(script, "not-verified"))
        with self.assertRaises(SystemExit):
            check_native_signing_receipts(release.replace("steps.windows-signer.outputs.thumbprint", "inputs.fake"))

    def test_agent_workflow_rejected(self):
        with self.assertRaises(SystemExit):
            check_local_first(Path("agent1-lockfiles.yml"), "")

    def test_scheduled_event_rejected(self):
        self.reject_change("ci.yml", "on:\n", "on:\n  schedule:\n    - cron: '0 0 * * *'\n")

    def test_inline_trigger_rejected(self):
        self.reject_change("ci.yml", "on:\n", "on:\n  workflow_dispatch: {}\n")

    def test_cancel_in_progress_required(self):
        self.reject_change("ci.yml", "cancel-in-progress: true", "cancel-in-progress: false")

    def test_docs_ignored_for_both_auto_events(self):
        self.reject_change("ci.yml", "      - 'docs/**'\n", "")
        self.reject_change("ci.yml", "  pull_request:\n    branches: [main]", "  pull_request:\n    branches: [develop]")

    def test_automatic_ci_cannot_build(self):
        self.reject_change("ci.yml", "          python3 scripts/check_release_version.py", "          cargo build --workspace")

    def test_final_gate_requires_exact_sha(self):
        self.reject_change("required-validation.yml", 'test "$LOCAL_SHA" = "$GITHUB_SHA"', "true")

    def test_certification_is_manual_only(self):
        self.reject_change("three-machine-certification.yml", "on:\n", "on:\n  push:\n")

    def test_certification_cannot_use_hosted_runner(self):
        self.reject_change("three-machine-certification.yml", "runs-on: [self-hosted, linux, swarmcraft-certification]", "runs-on: ubuntu-latest")

    def test_release_linux_packaging_cannot_use_hosted_runner(self):
        self.reject_change("release.yml", "runs-on: [self-hosted, linux, swarmcraft-release]", "runs-on: ubuntu-latest")


if __name__ == "__main__":
    unittest.main()
