"""Fail-closed regression tests for native packaged GUI failure observation."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("packaged-desktop-webdriver.py")
spec = importlib.util.spec_from_file_location("packaged_desktop_webdriver", SCRIPT)
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)

class NativeFailureObservationTests(unittest.TestCase):
    def make_app(self):
        app = driver.Application(Path("/path/not/launched"), Path("/not/used"))
        app.session = "test"
        ids = {"html": "html", "#status": "status",
               "#startupFailure": "banner", "#retryStartup": "retry"}
        app.element = lambda selector: ids[selector]
        return app

    def test_native_dom_requires_actual_failure_and_retry(self):
        values = {
            "html/attribute/data-swarmcraft-startup": "failed",
            "status/text": "Startup failed",
            "banner/text": "SwarmCraft could not start (backend, Device setup). Error Retry startup",
            "banner/displayed": True,
            "retry/text": "Retry startup",
            "retry/displayed": True,
        }
        def fake_request(path, method="GET", payload=None, timeout=75):
            return values[path.split("/element/", 1)[1]]
        app = self.make_app()
        with patch.object(driver, "request", side_effect=fake_request):
            result = app.visible_startup_failure()
            self.assertEqual(result["startup"], "failed")
            self.assertTrue(result["error"])
            self.assertTrue(result["retryVisible"])
            self.assertEqual(result["observedVia"], "native-webdriver-elements")
            values["html/attribute/data-swarmcraft-startup"] = "ready"
            self.assertIsNone(app.visible_startup_failure())
            values["html/attribute/data-swarmcraft-startup"] = "failed"
            values["retry/displayed"] = False
            self.assertIsNone(app.visible_startup_failure())

    def test_page_source_verifies_failure_without_javascript_execution(self):
        markup = (
            '<html data-swarmcraft-startup="failed"><body>'
            '<div id="status">Startup failed</div>'
            '<div id="startupFailure"><span>SwarmCraft could not start (backend, Device setup).'
            ' Missing required sidecar.</span>'
            '<button id="retryStartup">Retry startup</button></div>'
            '</body></html>'
        )
        result = driver._failure_from_page_source(markup)
        self.assertEqual(result["observedVia"], "native-webdriver-page-source")
        self.assertTrue(result["retryVisible"])
        self.assertIsNone(driver._failure_from_page_source(
            markup.replace('data-swarmcraft-startup="failed"', 'data-swarmcraft-startup="ready"')
        ))
        self.assertIsNone(driver._failure_from_page_source(
            markup.replace('id="startupFailure"', 'id="startupFailure" hidden')
        ))
        self.assertIsNone(driver._failure_from_page_source(
            markup.replace("Retry startup", "Retry later")
        ))

    def test_execute_failure_uses_only_verified_native_dom(self):
        class FakeApp:
            def __init__(self, verified):
                self.verified = verified
                self.screenshots = []
            def execute(self, script):
                raise RuntimeError("JavaScript evaluation failed")
            def screenshot(self, name):
                self.screenshots.append(name)
            def visible_startup_failure(self):
                return self.verified
        verified = {"startup": "failed", "error": True, "retryVisible": True,
                    "status": "Startup failed", "observedVia": "native-webdriver-elements"}
        valid = FakeApp(verified)
        self.assertEqual(driver.state(valid), verified)
        self.assertEqual(valid.screenshots, ["webdriver-failure.png"])
        invalid = FakeApp(None)
        with self.assertRaisesRegex(RuntimeError, "JavaScript evaluation failed"):
            driver.state(invalid)

if __name__ == "__main__":
    unittest.main()
