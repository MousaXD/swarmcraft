"""Fail-closed checks for sanitized, stage-specific installed GUI evidence."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest

TARGET = Path(__file__).with_name("visual-window-acceptance.py")
spec = importlib.util.spec_from_file_location("svelte_visual_evidence", TARGET)
if spec is None or spec.loader is None:
    raise RuntimeError("Visual evidence helper missing")
mod = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = mod
spec.loader.exec_module(mod)


class VisualEvidenceTests(unittest.TestCase):
    def test_does_not_serialize_invite_secrets_paths_or_arbitrary_errors(self):
        mod.mark_stage("real-signed-invite-issuance", {
            "hasSignedValue": True,
            "visible": True,
            "message": "scinvite:secret-private-token",
            "signed": "scinvite:secret-private-token",
            "absolutePath": "/home/player/.minecraft/saves/private-world",
            "rawNativeError": "private key leaked",
            "startup": "ready",
            "width": 980,
        })
        self.assertEqual(mod.CURRENT_STAGE, "real-signed-invite-issuance")
        self.assertEqual(mod.CURRENT_UI_STATE, {
            "hasSignedValue": True, "visible": True, "startup": "ready", "width": 980,
        })
        self.assertNotIn("secret", str(mod.CURRENT_UI_STATE))
        self.assertNotIn("/home/", str(mod.CURRENT_UI_STATE))

    def test_stage_reset_discards_previous_sensitive_observations(self):
        mod.mark_stage("layout-720x560", {
            "playVisible": False, "rootScrollWidth": 750,
            "startup": ["malformed", "untrusted"],
            "width": 720,
        })
        self.assertEqual(mod.CURRENT_UI_STATE, {
            "playVisible": False, "rootScrollWidth": 750, "width": 720,
        })
        mod.mark_stage("leave-confirmation-cancel")
        self.assertEqual(mod.CURRENT_UI_STATE, {})
        mod.record_ui({"worldStillListed": True, "signedInviteStillEmpty": True})
        self.assertEqual(mod.CURRENT_UI_STATE, {
            "worldStillListed": True, "signedInviteStillEmpty": True,
        })

    def test_categories_ignore_untrusted_exception_text(self):
        self.assertEqual(mod.failure_category(AssertionError("scinvite:token")), "assertion")
        self.assertEqual(mod.failure_category(TimeoutError("/private/file")), "timeout")
        self.assertEqual(mod.failure_category(RuntimeError("Bearer secret")), "native-driver-or-runtime")


if __name__ == "__main__":
    unittest.main()
