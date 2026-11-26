"""Local contract tests for the native orchestration, not installed-OS evidence."""
from __future__ import annotations

import copy
import errno
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import native_desktop_qa as qa


def records():
    return {
        variant: {
            "commit": "a" * 40, "tree": "b" * 40, "platform": "windows",
            "architecture": "x86_64", "variant": variant,
            "package_sha256": str(index) * 64,
            "files": {"desktop": str(index + 3) * 64,
                      **{name: "f" * 64 for name in qa.SIDECARS}},
        }
        for index, variant in enumerate(qa.VARIANTS)
    }


class ArtifactTests(unittest.TestCase):
    def test_exact_variant_set_and_identical_real_sidecars(self):
        good = records()
        qa.verify_artifacts(good)
        for field, value in (("commit", "c" * 40), ("tree", "d" * 40),
                             ("architecture", "arm64"), ("platform", "macos")):
            bad = copy.deepcopy(good)
            bad["candidate"][field] = value
            with self.assertRaises(qa.Failure):
                qa.verify_artifacts(bad)
        bad = copy.deepcopy(good)
        bad["broken"]["files"]["swarmcraft"] = "e" * 64
        with self.assertRaises(qa.Failure):
            qa.verify_artifacts(bad)

    def test_duplicate_artifact_cannot_certify_negative(self):
        for field in ("desktop", "package_sha256"):
            bad = records()
            if field == "desktop":
                bad["broken"]["files"][field] = bad["candidate"]["files"][field]
            else:
                bad["broken"][field] = bad["candidate"][field]
            with self.assertRaises(qa.Failure):
                qa.verify_artifacts(bad)

    def test_missing_sidecar_negative_never_mutates_original(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(qa.platform, "system", return_value="Linux"):
            root = Path(directory)
            installed = root / "installed"
            installed.mkdir()
            binary = installed / "swarmcraft-desktop"
            binary.write_bytes(b"desktop")
            for name in qa.SIDECARS:
                qa.sidecar(binary, name).write_bytes(name.encode())
            before = {p.name: qa.sha(p) for p in installed.iterdir()}
            destination = root / "negative"
            negative = qa.copy_missing_sidecar(binary, destination)
            self.assertFalse(qa.sidecar(negative, "swarmcraft").exists())
            self.assertTrue(negative.is_file())
            self.assertEqual(before, {p.name: qa.sha(p) for p in installed.iterdir()})
            with self.assertRaises(qa.Failure):
                qa.copy_missing_sidecar(binary, destination)

    def test_broken_assets_keep_positive_and_watchdog_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            source, destination = Path(directory) / "dist", Path(directory) / "broken"
            source.mkdir()
            (source / "assets").mkdir()
            original = '<script type="module" crossorigin src="/assets/index-abc.js"></script>'
            (source / "index.html").write_text(original)
            (source / "bootstrap.js").write_text("watchdog")
            (source / "candidate-startup-guard.js").write_text("guard")
            (source / "assets/index-abc.js").write_text("real module")
            qa.prepare_broken(source, destination)
            self.assertEqual((source / "index.html").read_text(), original)
            self.assertIn("required-entry-deliberately-missing.js", (destination / "index.html").read_text())
            self.assertFalse((destination / "assets/required-entry-deliberately-missing.js").exists())
            self.assertEqual(qa.sha(source / "bootstrap.js"), qa.sha(destination / "bootstrap.js"))
            self.assertEqual(qa.sha(source / "candidate-startup-guard.js"), qa.sha(destination / "candidate-startup-guard.js"))
            with self.assertRaises(qa.Failure):
                qa.prepare_broken(source, destination)

    def test_shipping_configuration_and_workflows_do_not_include_qa_hooks(self):
        root = qa.REPO
        shipping = json.loads((root / "apps/desktop/src-tauri/tauri.conf.json").read_text())
        self.assertEqual(shipping["identifier"], "dev.swarmcraft.desktop")
        self.assertEqual(shipping["build"]["frontendDist"], "../src")
        for variant in ("candidate", "broken"):
            overlay = json.loads((root / f"apps/desktop/frontend/tauri.native-qa.{variant}.conf.json").read_text())
            self.assertEqual(overlay["identifier"], f"dev.swarmcraft.desktop.qa-{variant}")
            self.assertNotIn("app", overlay)
        for workflow in (root / ".github/workflows").glob("*.yml"):
            self.assertNotIn("tauri.native-qa.", workflow.read_text())
            self.assertNotIn("native_desktop_ax", workflow.read_text())
        manifest = (root / "apps/desktop/src-tauri/Cargo.toml").read_text()
        main = (root / "apps/desktop/src-tauri/src/main.rs").read_text()
        self.assertNotIn("native_desktop_qa", manifest + main)
        self.assertNotIn("native_desktop_ax", manifest + main)


class IsolationTests(unittest.TestCase):
    def test_private_invitation_address_does_not_bypass_network_policy(self):
        for ip in ("127.0.0.1", "0.0.0.0", "8.8.8.8", "169.254.10.1", "192.0.2.1", "localhost", "::1"):
            with self.subTest(ip=ip), self.assertRaises(qa.Failure):
                qa.private_lan(ip)
        with patch.object(qa, "free_port", return_value=42000) as bind:
            self.assertEqual(qa.private_lan("192.168.5.3"), "192.168.5.3")
            bind.assert_called_once_with(qa.socket.SOCK_DGRAM, "192.168.5.3")

    def test_profiles_do_not_inherit_data_or_webview_overrides(self):
        with patch.dict(os.environ, {"SWARMCRAFT_DATA_DIR": "daily", "SWARMCRAFT_PROVIDER_KEY": "secret",
                                     "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "unsafe"}, clear=False):
            env = qa.profile_env(Path("/isolated"))
        self.assertEqual(env["SWARMCRAFT_DATA_DIR"], str(Path("/isolated/data")))
        self.assertNotIn("SWARMCRAFT_PROVIDER_KEY", env)
        self.assertNotIn("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", env)

    def test_cli_mutations_and_identity_creation_are_refused(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(qa.subprocess, "run") as run:
            root = Path(directory)
            for args in (("world", "create"), ("world", "join"), ("world", "hosting", "set"),
                         ("invite", "create"), ("identity",)):
                with self.assertRaises(qa.Failure):
                    qa.cli(root / "binary", root, *args)
            run.assert_not_called()

    def test_dynamic_world_selector_is_not_used_as_ax_identifier(self):
        target = qa.Target('#worldCards .world-row[data-world-id="fixture"]', qa.WORLD_NAME, ("AXButton",), True)
        self.assertIsNone(target.ax()["identifier"])
        self.assertEqual(target.ax()["label"], qa.WORLD_NAME)
        self.assertEqual(qa.button("createWorld", "Create world").ax()["identifier"], "createWorld")

    @unittest.skipIf(os.name == "nt", "POSIX fixture permissions")
    def test_fresh_profile_is_private_even_with_permissive_umask(self):
        with tempfile.TemporaryDirectory() as directory:
            original = os.umask(0)
            try:
                profile = qa.fresh_profile(Path(directory), "profile")
            finally:
                os.umask(original)
            self.assertEqual(profile.stat().st_mode & 0o777, 0o700)
            self.assertEqual((profile / "data").stat().st_mode & 0o777, 0o700)


class NativeObservationTests(unittest.TestCase):
    class FakeUI:
        def __init__(self, mapping):
            self.mapping = mapping
            self.presses = []
            self.screens = []

        def __enter__(self):
            return self

        def __exit__(self, *exc):
            pass

        def read(self, target):
            if target.css not in self.mapping:
                raise qa.Missing("absent")
            value, enabled = self.mapping[target.css]
            return {"value": value, "enabled": enabled, "checked": False}

        def press(self, target):
            self.presses.append(target.css)

        def screenshot(self, path):
            self.screens.append(path.name)

    def test_negative_requires_visible_failure_and_actionable_retry_not_just_exception(self):
        mapping = {"#status": ("Startup failed", True),
                   "#startupFailure": ("SwarmCraft could not start (module-load, initial setup).", True),
                   "#retryStartup": ("Retry startup", True)}
        ui = self.FakeUI(mapping)
        with patch.object(qa.time, "sleep"), patch.object(qa, "wait", side_effect=lambda reason, predicate, *args: qa.require(predicate(), reason)):
            report = qa.run_negative(lambda *_: ui, Path("/binary"), Path("/data"), Path("/artifacts"))
            self.assertTrue(report["visible_failure"])
            self.assertEqual(ui.presses, ["#retryStartup"])
            self.assertEqual(ui.screens, ["failure.png", "retry-failure.png"])
            for changed in (
                {"#nodeBadge": ("Ready", True)},
                {"#startupFailure": ("Arbitrary error", True)},
                {"#retryStartup": ("Retry startup", False)},
                {"#status": ("Starting", True)},
            ):
                invalid = self.FakeUI({**mapping, **changed})
                with self.assertRaises(qa.Failure):
                    qa.run_negative(lambda *_: invalid, Path("/binary"), Path("/data"), Path("/artifacts"))

    def test_startup_requires_header_node_and_completed_world_list(self):
        self.assertTrue(qa.READY.ax()["exactIdentifier"])
        self.assertTrue(qa.HEADER_READY.ax()["exactIdentifier"])
        self.assertNotEqual(qa.READY.ax()["identifier"], qa.HEADER_READY.ax()["identifier"])
        mapping = {"#nodeBadge": ("Ready", True), "#status": ("Ready", True),
                   "#worldCount": ("No local worlds", True)}
        with patch.object(qa, "wait", side_effect=lambda reason, predicate, *args: qa.require(predicate(), reason)):
            qa.ready(self.FakeUI(mapping), empty=True)
            for field, value in (("#status", "Networking needs attention"), ("#worldCount", "Loading…"),
                                 ("#nodeBadge", "Checking…")):
                with self.assertRaises(qa.Failure):
                    qa.ready(self.FakeUI({**mapping, field: (value, True)}), empty=True)

    def test_diagnostics_opens_collapsed_tools_using_native_control(self):
        class CollapsedTools(self.FakeUI):
            def press(self, target):
                super().press(target)
                if target.css == "#advancedNavigation > summary":
                    self.mapping["#navDiagnostics"] = ("Diagnostics", True)

        ui = CollapsedTools({"#advancedNavigation > summary": ("Tools", True)})
        qa.open_diagnostics(ui, True)
        self.assertEqual(ui.presses, ["#advancedNavigation > summary", "#navDiagnostics"])
        ui.presses.clear()
        qa.open_diagnostics(ui, True)
        self.assertEqual(ui.presses, ["#navDiagnostics"])

    def test_shipping_uses_world_details_instead_of_hidden_sidebar(self):
        class WorldTools(self.FakeUI):
            def press(self, target):
                super().press(target)
                if target.css == "#advancedWorldTools > summary":
                    self.mapping[".details-panel > summary"] = ("World details", True)
                elif target.css == ".details-panel > summary":
                    self.mapping["#openWorldDiagnostics"] = ("Diagnostics", True)

        ui = WorldTools({"#advancedWorldTools > summary": ("Advanced world controls", True)})
        qa.open_diagnostics(ui, False)
        self.assertEqual(ui.presses, ["#advancedWorldTools > summary", ".details-panel > summary",
                                      "#openWorldDiagnostics"])

    def test_hosting_preference_opens_advanced_controls_when_hidden(self):
        class WorldTools(self.FakeUI):
            def press(self, target):
                super().press(target)
                if target.css == "#advancedWorldTools > summary":
                    self.mapping["#hostingContributionStatus"] = ("Player only — 0 MiB / 0 CPU", True)
                    self.mapping["#hostingContributionHeap"] = ("1024", True)

        ui = WorldTools({"#advancedWorldTools > summary": ("Advanced world controls", True)})
        qa.open_hosting_contribution(ui)
        self.assertEqual(ui.presses, ["#advancedWorldTools > summary"])
        ui.presses.clear()
        qa.open_hosting_contribution(ui)
        self.assertEqual(ui.presses, [])

    def test_automatic_invitation_uses_only_visible_native_controls(self):
        class InviteUI(self.FakeUI):
            def __init__(self):
                super().__init__({"#startDaemon": ("Start networking", True),
                                  "#status": ("Ready", True),
                                  "#inviteWorld": ("Invite", True),
                                  "#createInvite": ("Create invite", True),
                                  "#cancelInvite": ("Done", True)})
                self.typed = []

            def type(self, target, value):
                self.typed.append((target.css, value))

            def press(self, target):
                super().press(target)
                if target.css == "#createInvite":
                    self.mapping["#inviteResult"] = ("scinvite:abcd", True)

        ui = InviteUI()
        with patch.object(qa, "stop_network"), patch.object(qa, "select_world"), \
                patch.object(qa, "free_port", return_value=42000), \
                patch.object(qa, "udp_listener_in_use", return_value=True), \
                patch.object(qa, "cli", return_value="2" * 32):
            token, port = qa.start_invite_source(ui, Path("/binary"), Path("/profile"), "fixture", "10.0.0.1", False)
        self.assertEqual((token, port), ("scinvite:abcd", 42000))
        self.assertEqual(ui.typed, [("#daemonListen", "/ip4/10.0.0.1/udp/42000/quic-v1")])
        self.assertEqual(ui.presses, ["#startDaemon", "#inviteWorld", "#createInvite", "#cancelInvite"])

    def test_udp_listener_probe_distinguishes_occupied_from_invalid_interface(self):
        with patch.object(qa.socket, "socket") as socket_factory:
            sock = socket_factory.return_value.__enter__.return_value
            self.assertFalse(qa.udp_listener_in_use("10.0.0.1", 42000))
            sock.bind.side_effect = OSError(errno.EADDRINUSE, "occupied")
            self.assertTrue(qa.udp_listener_in_use("10.0.0.1", 42000))
            sock.bind.side_effect = OSError(errno.EADDRNOTAVAIL, "not assigned")
            with self.assertRaisesRegex(qa.Failure, "exact local UDP listener"):
                qa.udp_listener_in_use("10.0.0.1", 42000)

    def test_native_webdriver_elements_can_independently_prove_failure(self):
        app = qa.WebDriver(Path("/binary"), Path("/data"), Path("/driver"), Path("/native"))
        app.session = "fixture"
        replies = {
            "/element": {"element-6066-11e4-a52e-4f735466cecf": "banner"},
            "/element/banner/displayed": True,
            "/element/banner/text": "SwarmCraft could not start (missing sidecar).",
            "/element/banner/enabled": True,
        }
        def call(path, *args, **kwargs):
            if path == "/execute/sync":
                raise qa.Failure("unavailable script evaluation")
            return replies[path]
        with patch.object(app, "call", side_effect=call):
            self.assertTrue(qa.read_is(app, qa.BANNER, "SwarmCraft could not start (", prefix=True))
            replies["/element/banner/displayed"] = False
            self.assertFalse(qa.read_is(app, qa.BANNER, "SwarmCraft could not start (", prefix=True))

    def test_hidden_observation_does_not_fall_back_to_native_click(self):
        app = qa.WebDriver(Path("/binary"), Path("/data"), Path("/driver"), Path("/native"))
        app.session = "fixture"
        target = qa.button("openWorldDiagnostics", "Diagnostics")
        with patch.object(app, "call", return_value=None) as call:
            with self.assertRaises(qa.Missing):
                app.read(target)
            call.assert_called_once()
            self.assertIn(qa.RENDERED_CONTROL, call.call_args.args[2]["script"])
        with patch.object(app, "call", return_value=False) as call:
            with self.assertRaises(qa.Failure):
                app.press(target)
            # A negative geometry/disclosure observation is not a driver
            # exception. Do not let element/displayed or click override it.
            call.assert_called_once()
            self.assertIn(qa.RENDERED_CONTROL, call.call_args.args[2]["script"])
            self.assertIn("scrollIntoView", call.call_args.args[2]["script"])

    def test_machine_verifier_retains_normal_cli_warnings(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(qa.subprocess, "run") as run:
            profile = Path(directory)
            run.return_value.returncode = 0
            run.return_value.stdout = b'{"consented": false}'
            with patch.dict(os.environ, {"RUST_LOG": "debug"}):
                self.assertEqual(qa.cli(profile / "desktop", profile, "world", "hosting", "status",
                                        "fixture", "--json"), '{"consented": false}')
                self.assertEqual(run.call_args.kwargs["env"]["RUST_LOG"], "warn")
                self.assertEqual(os.environ["RUST_LOG"], "debug")
                self.assertEqual(qa.profile_env(profile)["RUST_LOG"], "warn")


if __name__ == "__main__":
    unittest.main()
