#!/usr/bin/env python3
"""Drive the installed SwarmCraft Tauri/WebKitGTK process through native WebDriver.

No mock invoke, browser fixture, source-server, or direct CLI world creation.
The application executable must be extracted from the current commit's .deb.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request


PORT = 4464
NATIVE_PORT = 4465


def request(path: str, method: str = "GET", payload=None, timeout=75):
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"http://127.0.0.1:{PORT}{path}",
        data=data,
        headers={"Content-Type": "application/json"} if data is not None else {},
        method=method,
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            result = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"WebDriver {method} {path}: HTTP {error.code}: {body[:1000]}") from error
    value = result.get("value", {})
    if isinstance(value, dict) and value.get("error"):
        raise RuntimeError(f"WebDriver {method} {path}: {value.get('error')}: {str(value.get('message'))[:500]}")
    return value


class Application:
    def __init__(self, binary: Path, artifacts: Path):
        self.binary = binary
        self.artifacts = artifacts
        self.proc = None
        self.logs = None
        self.session = None

    def __enter__(self):
        self.logs = (self.artifacts / "tauri-driver.log").open("ab")
        self.proc = subprocess.Popen(
            ["tauri-driver", "--port", str(PORT), "--native-port", str(NATIVE_PORT)],
            stdout=self.logs,
            stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"tauri-driver exited with {self.proc.returncode}")
            try:
                with socket.create_connection(("127.0.0.1", PORT), timeout=1):
                    break
            except OSError:
                time.sleep(0.2)
        else:
            raise TimeoutError("tauri-driver did not open its WebDriver endpoint")

        capabilities = {
            "browserName": "wry",
            "tauri:options": {"application": str(self.binary), "webviewOptions": {}},
        }
        value = request("/session", "POST", {"capabilities": {"alwaysMatch": capabilities}}, timeout=90)
        self.session = value.get("sessionId")
        if not self.session:
            raise RuntimeError(f"native session missing sessionId: {str(value)[:400]}")
        request(f"/session/{self.session}/timeouts", "POST",
                {"script": 20000, "pageLoad": 20000, "implicit": 0})
        return self

    def __exit__(self, *exc):
        if self.session:
            try:
                request(f"/session/{self.session}", "DELETE", timeout=20)
            except Exception:
                pass
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=10)
        if self.logs:
            self.logs.close()

    def execute(self, script, args=None):
        return request(f"/session/{self.session}/execute/sync", "POST",
                       {"script": script, "args": args or []})

    def element(self, selector):
        value = request(f"/session/{self.session}/element", "POST",
                        {"using": "css selector", "value": selector})
        return value.get("element-6066-11e4-a52e-4f735466cecf") or value.get("ELEMENT")

    def click(self, selector):
        element = self.element(selector)
        request(f"/session/{self.session}/element/{element}/click", "POST", {})

    def type(self, selector, text):
        element = self.element(selector)
        request(f"/session/{self.session}/element/{element}/value", "POST",
                {"text": text, "value": list(text)})

    def screenshot(self, name):
        try:
            result = request(f"/session/{self.session}/screenshot", timeout=15)
            (self.artifacts / name).write_bytes(base64.b64decode(result))
        except Exception as error:
            (self.artifacts / f"{name}.error.txt").write_text(type(error).__name__ + ": screenshot unavailable\n")


# Read only a bounded, explicitly non-secret observability contract.
# No raw identity string, invitation, CLI stderr, or arbitrary frontend output.
STATE_SCRIPT = """
const byId = id => document.getElementById(id);
const value = id => byId(id)?.textContent?.trim() || '';
const rows = [...document.querySelectorAll('#worldCards .world-row')];
return {
  documentReady: document.readyState,
  bridge: typeof window.__TAURI__?.core?.invoke === 'function',
  bridgeWrapped: window.__TAURI__?.core?.invoke?.__swarmcraftProviderContractBridge === true,
  bridgeWritable: Object.getOwnPropertyDescriptor(window.__TAURI__?.core || {}, 'invoke')?.writable ?? null,
  playerExperienceObserver: document.documentElement.dataset.playerExperienceObserver === 'true',
  startup: document.documentElement.dataset.swarmcraftStartup || 'not-run',
  phase: document.documentElement.dataset.swarmcraftStartupPhase || '',
  status: value('status'),
  node: value('nodeBadge'),
  worldCount: value('worldCount'),
  worldBusy: byId('worldCards')?.getAttribute('aria-busy'),
  empty: !!document.querySelector('#worldCards .empty-state'),
  worlds: rows.map(row => ({
    name: row.querySelector('.world-row-title strong')?.textContent || '',
    idHash: row.dataset.worldId ? 'sha-hidden' : '',
    selected: row.getAttribute('aria-pressed') === 'true',
  })),
  error: !!byId('startupFailure') && !byId('startupFailure').hidden,
  worldError: !!byId('worldLoadError') && !byId('worldLoadError').hidden,
  createPanelVisible: !!byId('createPanel') && !byId('createPanel').hidden,
  createButtonDisabled: byId('createWorld')?.disabled === true,
  createNamePopulated: !!byId('createName')?.value?.trim(),
  minecraftChosen: !!byId('createMinecraft')?.value,
  fabricChosen: !!byId('createLoader')?.value,
  compatibilityChosen: !!byId('createCompatibility')?.value,
  catalogErrorVisible: !!byId('createCatalogError') && !byId('createCatalogError').hidden,
  catalogRetryVisible: !!byId('createCatalogRetry') && !byId('createCatalogRetry').hidden,
  createNoticeKind: byId('createError')?.dataset?.kind || byId('createError')?.dataset?.tone || '',
  createErrorVisible: !!byId('createError') && !byId('createError').hidden
    && (byId('createError').dataset?.kind === 'error' || byId('createError').dataset?.tone === 'danger'),
  createRepairVisible: !!byId('createError') && !byId('createError').hidden
    && byId('createError').dataset?.kind === 'warning',
  scriptFiles: performance.getEntriesByType('resource').filter(item => item.name.includes('.js')).map(item => {
    try { return new URL(item.name).pathname.split('/').pop(); } catch { return 'unknown-script'; }
  }).slice(-24),
  errors: (window.__swarmcraftStartup?.errors || []).map(item => ({kind: item.kind, phase: item.phase})),
};
"""


def state(app: Application):
    try:
        return app.execute(STATE_SCRIPT)
    except Exception:
        app.screenshot("webdriver-failure.png")
        raise


def await_ready(app: Application, artifacts: Path):
    deadline = time.monotonic() + 85
    latest = None
    while time.monotonic() < deadline:
        latest = state(app)
        (artifacts / "last-state.json").write_text(json.dumps(latest, indent=2))
        if latest.get("startup") == "failed" or latest.get("error") or latest.get("worldError"):
            app.screenshot("startup-failed.png")
            raise AssertionError(f"Packaged desktop startup failed: {latest}")
        if latest.get("startup") in ("ready", "degraded"):
            assert latest["bridge"], "Tauri command bridge is not connected"
            assert latest["node"] == "Ready", f"Device not initialized: {latest}"
            assert latest["worldBusy"] == "false", f"World list is still busy: {latest}"
            assert "Loading" not in latest["worldCount"], f"World list stuck: {latest}"
            assert "Checking" not in latest["node"], f"Device stuck checking: {latest}"
            assert latest["empty"] or latest["worlds"], f"No valid list or empty-state UI: {latest}"
            assert latest["status"] != "Starting…", f"Header falsely reports startup: {latest}"
            return latest
        time.sleep(0.35)
    app.screenshot("startup-timeout.png")
    raise TimeoutError(f"Real packaged desktop did not finish starting: {latest}")


def await_catalog_ready(app: Application, artifacts: Path):
    # The real Mojang/Fabric catalogs are loaded asynchronously through Tauri.
    # Clicking disabled Create is a no-op that could otherwise look like an
    # unexplained world-creation timeout; require valid, backend-issued choices.
    deadline = time.monotonic() + 80
    retried = False
    last = None
    while time.monotonic() < deadline:
        last = state(app)
        (artifacts / "last-state.json").write_text(json.dumps(last, indent=2))
        if (not last["createButtonDisabled"] and last["minecraftChosen"]
                and last["fabricChosen"] and last["compatibilityChosen"]):
            return
        if last["catalogErrorVisible"]:
            if not retried and last["catalogRetryVisible"]:
                retried = True
                app.click("#createCatalogRetry")
                time.sleep(1)
                continue
            app.screenshot("catalog-unavailable.png")
            raise AssertionError(
                "Real installed GUI cannot create a world: official Minecraft/Fabric "
                f"catalog retrieval failed after retry: {last}"
            )
        time.sleep(0.4)
    app.screenshot("catalog-timeout.png")
    raise TimeoutError(f"Real GUI catalog never provided compatible version selections: {last}")


def await_world(app: Application, name: str, artifacts: Path):
    deadline = time.monotonic() + 70
    last = None
    while time.monotonic() < deadline:
        last = state(app)
        (artifacts / "last-state.json").write_text(json.dumps(last, indent=2))
        if (last.get("startup") == "failed" or last.get("worldError")
                or last.get("createErrorVisible") or last.get("createRepairVisible")):
            app.screenshot("world-create-failed.png")
            raise AssertionError(f"Real GUI world creation/list command failed: {last}")
        if any(world["name"] == name for world in last.get("worlds", [])) and last["worldBusy"] == "false":
            return last
        time.sleep(0.4)
    app.screenshot("world-create-timeout.png")
    raise TimeoutError(f"Newly created world never appeared in the actual GUI: {last}")


def run(binary: Path, artifacts: Path):
    assert binary.is_file() and os.access(binary, os.X_OK), f"Not an installed executable: {binary}"
    # Isolate actual CLI data. Persistence is tested across two fresh app processes.
    assert os.getenv("SWARMCRAFT_DATA_DIR"), "CI must supply isolated SWARMCRAFT_DATA_DIR"
    data = Path(os.environ["SWARMCRAFT_DATA_DIR"])
    data.mkdir(parents=True, exist_ok=True)
    assert not any(data.iterdir()), "First launch was not from clean state"

    world_name = f"Desktop E2E {os.environ.get('GITHUB_RUN_ID', 'local')}"
    first_identity = None
    with Application(binary, artifacts) as app:
        initial = await_ready(app, artifacts)
        assert initial["empty"], f"Fresh installation should have no worlds: {initial}"
        first_identity = app.execute("return document.getElementById('identity')?.textContent || '';")
        assert first_identity.strip(), "Initialized device has no identity"
        app.screenshot("first-launch.png")
        app.click("#openCreate")
        await_catalog_ready(app, artifacts)
        app.type("#createName", world_name)
        entered = state(app)
        assert entered["createPanelVisible"] and entered["createNamePopulated"], (
            f"Create form did not accept the world name: {entered}"
        )
        app.screenshot("before-world-create.png")
        app.click("#createWorld")
        created = await_world(app, world_name, artifacts)
        assert any(w["name"] == world_name for w in created["worlds"])
        app.screenshot("world-created.png")

    # Driver session deletion must close the application. New session launches
    # the same extracted *packaged* binary with the same persistent data path.
    with Application(binary, artifacts) as app:
        launched = await_ready(app, artifacts)
        assert any(w["name"] == world_name for w in launched["worlds"]), "World disappeared after restart"
        restored_identity = app.execute("return document.getElementById('identity')?.textContent || '';")
        assert restored_identity == first_identity, "Device identity changed after restart"
        app.click("#navWorlds")
        app.click("#worldCards .world-row")
        selected = state(app)
        assert any(w["name"] == world_name and w["selected"] for w in selected["worlds"]), (
            "Persisted world cannot be selected through the real GUI"
        )
        app.screenshot("after-relaunch.png")
    print("PASS: installed Tauri/WebKitGTK + real bundled sidecars: initial state, GUI create, relaunch, identity/world persistence")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    args.artifacts.mkdir(parents=True, exist_ok=True)
    try:
        run(args.binary.resolve(), args.artifacts)
    except Exception as error:
        # Capture only a bounded, sanitized failure classification, never the
        # complete stdout, invitation, identity or arbitrary JavaScript stack.
        message = str(error)
        if re.search(r"bearer|private.?key|password|secret|invite.?token|authorization", message, re.I):
            message = "Sensitive diagnostic details suppressed"
        message = re.sub(r"/home/runner/[^ ]+", "<runner-path>", message)[:500]
        (args.artifacts / "failure.txt").write_text(
            f"{type(error).__name__}: {message}\n"
        )
        print(f"FAIL: packaged GUI acceptance: {type(error).__name__}: {message}", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
