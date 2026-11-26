#!/usr/bin/env python3
"""Local installed native GUI acceptance. No mock IPC, browser, or app test hooks."""
from __future__ import annotations

import argparse
import base64
import errno
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from typing import NamedTuple
import urllib.error
import urllib.request

REPO = Path(__file__).resolve().parents[2]
SIDECARS = ("swarmcraft", "swarmcraft-host", "swarmcraft-runtime", "swarmcraft-import")
VARIANTS = ("shipping", "candidate", "broken")
WORLD_NAME = "Native QA private world"
WORLD_PATTERN = r"scworld:[0-9a-f]{64}"
HEX = re.compile(r"[0-9a-f]{64}")


class Failure(RuntimeError):
    pass


class Missing(Failure):
    pass


def require(condition, reason):
    if not condition:
        raise Failure(reason)


def wait(reason, predicate, seconds=90):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            value = predicate()
            if value:
                return value
        except Missing:
            pass
        time.sleep(0.35)
    raise Failure(reason + ": timed out")


def checked(command, *, env=None, stdin=None, seconds=45):
    result = subprocess.run([str(x) for x in command], input=stdin, capture_output=True,
                            env=env, timeout=seconds, check=False)
    # Arguments and stdout can contain private fixture identities. Never echo.
    require(result.returncode == 0, "native process returned nonzero")
    return result.stdout


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def checkout():
    git = ["git", "-C", REPO]
    checked([*git, "diff", "--quiet", "HEAD", "--"])
    commit = checked([*git, "rev-parse", "HEAD"]).decode().strip()
    tree = checked([*git, "rev-parse", "HEAD^{tree}"]).decode().strip()
    return commit, tree


def target_platform():
    name = {"Windows": "windows", "Darwin": "macos", "Linux": "linux"}.get(platform.system())
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "aarch64": "arm64", "arm64": "arm64"}.get(machine)
    require(name and arch, "unsupported host platform or architecture")
    if name == "macos":
        translated = subprocess.run(["sysctl", "-n", "sysctl.proc_translated"], capture_output=True)
        require(translated.stdout.strip() != b"1", "Rosetta is not native Intel macOS evidence")
    if name == "windows":
        require(arch == "x86_64" and not os.environ.get("PROCESSOR_ARCHITEW6432"),
                "Windows release matrix currently requires native x86_64")
    return name, arch


def sidecar(binary, name):
    return binary.parent / (name + (".exe" if os.name == "nt" else ""))


def app_bundle(binary):
    require(binary.parent.name == "MacOS" and binary.parent.parent.name == "Contents"
            and binary.parent.parent.parent.suffix == ".app", "macOS executable must be inside an installed .app")
    return binary.parent.parent.parent


def binary_arch(path, system, arch):
    if system == "macos":
        slices = checked(["lipo", "-archs", path]).decode().split()
        require(arch in slices, "installed Mach-O lacks native architecture")
    elif system == "windows":
        with path.open("rb") as f:
            require(f.read(2) == b"MZ", "installed executable is not PE")
            f.seek(0x3C)
            offset = int.from_bytes(f.read(4), "little")
            require(0x40 <= offset <= 1024 * 1024, "invalid PE header")
            f.seek(offset)
            require(f.read(4) == b"PE\0\0" and f.read(2) == b"\x64\x86", "installed PE is not x86_64")
    else:
        with path.open("rb") as f:
            header = f.read(20)
        expected = 62 if arch == "x86_64" else 183
        require(header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[18:20], "little") == expected,
                "installed ELF lacks native architecture")


def package_record(binary, package, variant):
    system, arch = target_platform()
    require(variant in VARIANTS, "unknown artifact variant")
    require(package.is_file() and not package.is_symlink(), "installer file required")
    allowed = {"windows": (".exe", ".msi"), "macos": (".dmg", ".pkg"), "linux": (".deb", ".rpm", ".appimage")}
    require(package.suffix.lower() in allowed[system], "platform installer required")
    if system == "macos":
        bundle = app_bundle(binary)
        with (bundle / "Contents" / "Info.plist").open("rb") as stream:
            identifier = plistlib.load(stream).get("CFBundleIdentifier")
        expected = {"shipping": "dev.swarmcraft.desktop",
                    "candidate": "dev.swarmcraft.desktop.qa-candidate",
                    "broken": "dev.swarmcraft.desktop.qa-broken"}[variant]
        require(identifier == expected, "installed macOS bundle identifier does not match QA variant")
    binaries = {"desktop": binary, **{name: sidecar(binary, name) for name in SIDECARS}}
    files = {}
    for name, path in binaries.items():
        require(path.is_file() and not path.is_symlink() and os.access(path, os.X_OK),
                "missing executable in installed bundle: " + name)
        binary_arch(path, system, arch)
        files[name] = sha(path)
    commit, tree = checkout()
    return {"schema": 1, "commit": commit, "tree": tree, "platform": system, "architecture": arch,
            "variant": variant, "package_sha256": sha(package), "files": files}


def verify_artifacts(records):
    require(set(records) == set(VARIANTS), "all three installer variants are required")
    baseline = records["shipping"]
    for variant, record in records.items():
        require(record["variant"] == variant, "artifact variant mismatch")
        for key in ("commit", "tree", "platform", "architecture"):
            require(record[key] == baseline[key], "different source or platform in artifact set")
        require(all(record["files"][name] == baseline["files"][name] for name in SIDECARS),
                "variants must contain identical real sidecars")
    require(len({r["files"]["desktop"] for r in records.values()}) == 3, "distinct compiled desktop variants required")
    require(len({r["package_sha256"] for r in records.values()}) == 3, "distinct installer packages required")


class Target(NamedTuple):
    css: str
    label: str
    roles: tuple[str, ...] = ("AXButton",)
    prefix: bool = False
    exact_identifier: bool = False

    def ax(self):
        return {"identifier": self.css[1:] if re.fullmatch(r"#[A-Za-z0-9_-]+", self.css) else None,
                "label": self.label, "roles": self.roles, "prefix": self.prefix,
                "exactIdentifier": self.exact_identifier}


TEXT = ("AXStaticText", "AXHeading", "AXGroup")
FIELD = ("AXTextField", "AXTextArea", "AXIncrementor", "AXSpinButton")
CHECK = ("AXCheckBox",)


def button(id_, label):
    return Target("#" + id_, label)


def field(id_, label):
    return Target("#" + id_, label, FIELD)


def text(id_, label, prefix=False):
    return Target("#" + id_, label, TEXT, prefix)


# The two separate Ready indicators must not resolve to the same AX label.
READY = Target("#nodeBadge", "Ready", TEXT, exact_identifier=True)
HEADER_READY = Target("#status", "Ready", TEXT, exact_identifier=True)
EMPTY = text("worldCount", "No local worlds")
ONE_WORLD = text("worldCount", "1 world")
FAIL = text("status", "Startup failed")
BANNER = text("startupFailure", "SwarmCraft could not start (", True)
RETRY = button("retryStartup", "Retry startup")
PLAYER_ONLY = text("hostingContributionStatus", "Player only — 0 MiB / 0 CPU")
OPTED_IN = text("hostingContributionStatus", "Hosting resource preference saved")


def free_port(kind=socket.SOCK_STREAM, address="127.0.0.1"):
    with socket.socket(socket.AF_INET, kind) as sock:
        sock.bind((address, 0))
        return sock.getsockname()[1]


def private_lan(value):
    try:
        address = ipaddress.IPv4Address(value)
    except ipaddress.AddressValueError:
        raise Failure("literal private IPv4 address required") from None
    require(any(address in ipaddress.IPv4Network(network) for network in
                ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")),
            "QA invitation requires an explicitly selected RFC1918 LAN interface, not loopback or public IP")
    # The bind also refuses a private address belonging to a different host.
    free_port(socket.SOCK_DGRAM, str(address))
    return str(address)


def owned_process(command, profile):
    return subprocess.Popen([str(x) for x in command], env=profile_env(profile),
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=os.name != "nt")


def stop_owned_tree(proc):
    """Only the isolated process group/tree launched by this harness."""
    if os.name == "nt":
        if proc.poll() is None:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
    else:
        # Our private session is separate from the operator's daily app.
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=10)


def profile_env(profile):
    env = os.environ.copy()
    # Remove inherited overrides of provider/transport credentials and QA
    # WebView arguments. Do not attach to an existing browser or daily store.
    for key in list(env):
        if key.startswith(("SWARMCRAFT_", "WEBVIEW2_")):
            env.pop(key)
    env["SWARMCRAFT_DATA_DIR"] = str(profile / "data")
    env["WEBVIEW2_USER_DATA_FOLDER"] = str(profile / "webview2")
    env["RUST_LOG"] = "warn"
    return env


# Some WebKit versions retain layout rectangles for clipped descendants of a
# closed disclosure. A rectangle alone therefore does not establish that a
# control is reachable. Share this guard across observation and input.
RENDERED_CONTROL = """
const e = document.querySelector(arguments[0]);
function rendered(element) {
    if (!element || !element.isConnected ||
        getComputedStyle(element).visibility !== 'visible' ||
        !Array.from(element.getClientRects()).some(r => r.width > 0 && r.height > 0))
        return false;
    for (let ancestor = element; ancestor; ancestor = ancestor.parentElement) {
        const style = getComputedStyle(ancestor);
        if (ancestor.hidden || style.display === 'none' ||
            style.contentVisibility === 'hidden' || style.opacity === '0')
            return false;
        if (ancestor !== element && ancestor.tagName === 'DETAILS' && !ancestor.open) {
            const summary = Array.from(ancestor.children).find(child => child.tagName === 'SUMMARY');
            if (!summary || !summary.contains(element)) return false;
        }
    }
    return true;
}
"""


class WebDriver:
    """Native WebKitGTK on Linux; native WebView2/msedgedriver on Windows."""
    def __init__(self, binary, profile, driver, native):
        self.binary, self.profile = binary, profile
        self.driver, self.native = driver, native
        self.port = free_port()
        self.native_port = free_port()
        while self.native_port == self.port:
            self.native_port = free_port()
        self.proc = self.session = None
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.capabilities = {}

    def request(self, path, method="GET", payload=None, seconds=30):
        data = None if payload is None else json.dumps(payload).encode()
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data, method=method,
                                     headers={"Content-Type": "application/json"})
        try:
            with self.opener.open(req, timeout=seconds) as response:
                reply = json.load(response)
        except urllib.error.HTTPError as error:
            try:
                reply = json.loads(error.read(64 * 1024))
            except (ValueError, OSError):
                raise Failure("invalid native WebDriver response") from None
            require(isinstance(reply.get("value"), dict) and reply["value"].get("error"),
                    "native WebDriver HTTP failure")
        value = reply.get("value")
        if isinstance(value, dict) and value.get("error"):
            if value["error"] in ("no such element", "stale element reference"):
                raise Missing("native element not found")
            # Keep a standard diagnostic category, never the driver message,
            # which can include private input or application paths.
            category = value["error"]
            if category in ("element click intercepted", "element not interactable",
                            "invalid element state", "invalid argument", "timeout",
                            "script timeout", "javascript error", "no such window",
                            "session not created", "unknown error"):
                raise Failure("native WebDriver operation failed: " + category)
            raise Failure("native WebDriver operation failed")
        return value

    def __enter__(self):
        try:
            self.proc = owned_process([self.driver, "--port", self.port, "--native-port",
                                       self.native_port, "--native-driver", self.native], self.profile)
            def listening():
                require(self.proc.poll() is None, "native driver exited")
                try:
                    value = self.request("/status", seconds=2)
                    return isinstance(value, dict) and value.get("ready") is True
                except (OSError, urllib.error.URLError):
                    return False
            wait("native WebDriver readiness", listening, 30)
            value = self.request("/session", "POST", {"capabilities": {"alwaysMatch": {
                "browserName": "wry", "tauri:options": {"application": str(self.binary), "webviewOptions": {}}
            }}}, 90)
            self.session = value.get("sessionId")
            require(isinstance(self.session, str) and self.session, "native session ID absent")
            caps = value.get("capabilities", {})
            if os.name == "nt":
                # A Chrome session or an unknown browser is not WebView2 proof.
                require(str(caps.get("browserName", "")).lower() in ("microsoftedge", "msedge", "webview2"),
                        "native session did not identify Edge WebView2")
                version = str(caps.get("browserVersion", ""))
                driver_version = checked([self.native, "--version"]).decode(errors="replace")
                expected = re.search(r"\d+\.\d+\.\d+", driver_version)
                require(expected and version.startswith(expected.group() + "."), "EdgeDriver/WebView2 build mismatch")
                self.capabilities = {"engine": "WebView2", "version": version, "driver_build": expected.group()}
            else:
                self.capabilities = {"engine": "WebKitGTK", "native_driver": self.native.name}
            self.call("/timeouts", "POST", {"script": 10000, "pageLoad": 20000, "implicit": 0})
            return self
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise

    def __exit__(self, *exc):
        clean = True
        if self.session:
            try:
                self.call("", "DELETE", seconds=20)
            except Exception:
                clean = False
            self.session = None
        if self.proc:
            stop_owned_tree(self.proc)
        if not clean and exc[0] is None:
            raise Failure("native WebDriver session did not close cleanly")

    def call(self, path, method="GET", payload=None, seconds=30):
        return self.request("/session/" + self.session + path, method, payload, seconds)

    def element(self, target):
        value = self.call("/element", "POST", {"using": "css selector", "value": target.css})
        id_ = value.get("element-6066-11e4-a52e-4f735466cecf") or value.get("ELEMENT")
        require(id_, "native element reference missing")
        return "/element/" + id_

    def read(self, target):
        # DOM inspection only; all mutations go through native element commands.
        try:
            value = self.call("/execute/sync", "POST", {"script": RENDERED_CONTROL + """
          if (!rendered(e)) return null;
          return {value: ('value' in e && e.tagName !== 'BUTTON') ? String(e.value) : e.textContent.trim(),
                  enabled: !e.disabled, checked: e.checked === true};
            """, "args": [target.css]})
        except Failure:
            # Some WebKit drivers reject script evaluation after module or
            # sidecar failure. Native element observation still must prove
            # actual rendered text, visibility and retry availability.
            element = self.element(target)
            if self.call(element + "/displayed") is not True:
                raise Missing("native element not visible")
            operation = "/property/value" if target.roles == FIELD else "/text"
            value = {"value": str(self.call(element + operation)),
                     "enabled": self.call(element + "/enabled") is True,
                     "checked": self.call(element + "/selected") is True if target.roles == CHECK else False}
        if value is None:
            raise Missing("native element not visible")
        return value

    def press(self, target):
        # WebKitGTK may not scroll a deeply nested disclosure in the
        # selected-world pane into view on its own. Scroll the real rendered
        # control before using native WebDriver input; never open hidden
        # panels or dispatch a synthetic JavaScript click.
        self.scroll_to_control(target)
        self.call(self.element(target) + "/click", "POST", {})

    def type(self, target, value):
        self.scroll_to_control(target)
        element = self.element(target)
        self.call(element + "/clear", "POST", {})
        self.call(element + "/value", "POST", {"text": value, "value": list(value)})
        require(self.read(target)["value"] == value, "native field input not reflected")

    def scroll_to_control(self, target):
        try:
            visible = self.call("/execute/sync", "POST", {"script": RENDERED_CONTROL + """
                if (!rendered(e)) return false;
                e.scrollIntoView({behavior: 'auto', block: 'center', inline: 'nearest'});
                const r = e.getBoundingClientRect();
                return rendered(e) && r.width > 0 && r.height > 0 && r.bottom > 0 && r.right > 0
                    && r.top < innerHeight && r.left < innerWidth;
            """, "args": [target.css]})
        except Failure:
            # A broken-module negative may not permit script evaluation.
            # Fall back only to the native displayed/enabled proof.
            element = self.element(target)
            visible = self.call(element + "/displayed") is True
        require(visible is True, "native control not visibly reachable")

    def screenshot(self, path):
        data = base64.b64decode(self.call("/screenshot"), validate=True)
        require(data.startswith(b"\x89PNG\r\n\x1a\n"), "invalid native screenshot")
        path.write_bytes(data)


class Accessibility:
    """Native WKWebView via out-of-process, PID-bound macOS AX client."""
    def __init__(self, binary, profile, helper):
        self.binary, self.profile, self.helper = binary, profile, helper
        self.proc = None
        self.capabilities = {"engine": "WKWebView", "adapter": "macOS Accessibility"}

    def __enter__(self):
        self.proc = owned_process([self.binary], self.profile)
        return self

    def invoke(self, action, target=None, value=None):
        require(self.proc.poll() is None, "native macOS application exited")
        request = {"pid": self.proc.pid, "action": action, **(target.ax() if target else {})}
        if value is not None:
            request["value"] = value
        result = subprocess.run([str(self.helper)], input=json.dumps(request).encode(), capture_output=True,
                                timeout=35, check=False)
        try:
            reply = json.loads(result.stdout)
        except ValueError:
            raise Failure("invalid macOS Accessibility response") from None
        if result.returncode != 0:
            kind = reply.get("error")
            if kind == "element-or-window-not-found":
                raise Missing("native accessibility element not found")
            require(False, kind if kind in ("accessibility-permission-required", "ambiguous-accessibility-element",
                                           "control-disabled", "native-accessibility-operation-failed")
                    else "native Accessibility operation failed")
        return reply

    def read(self, target):
        return self.invoke("read", target)

    def press(self, target):
        self.invoke("press", target)

    def type(self, target, value):
        self.invoke("type", target, value)
        require(self.read(target)["value"] == value, "native field input not reflected")

    def screenshot(self, path):
        window = self.invoke("window")["window"]
        checked(["screencapture", "-x", "-l", str(window), path])
        require(path.read_bytes().startswith(b"\x89PNG\r\n\x1a\n"), "native screenshot not captured")

    def __exit__(self, *exc):
        if self.proc is None:
            return
        clean = True
        if self.proc.poll() is None:
            try:
                self.invoke("close")
                self.proc.wait(timeout=25)
            except Exception:
                clean = False
                stop_owned_tree(self.proc)
        if not clean and exc[0] is None:
            raise Failure("native macOS application did not close cleanly")


def read_is(ui, target, expected, prefix=False):
    try:
        value = ui.read(target)["value"]
        return value.startswith(expected) if prefix else value == expected
    except Missing:
        return False


def click(ui, target):
    label = target.css if re.fullmatch(r"#[A-Za-z0-9_-]+", target.css) else "selected native control"
    wait("enabled native control " + label, lambda: ui.read(target)["enabled"])
    try:
        ui.press(target)
    except Failure as error:
        raise Failure("native click " + label + ": " + str(error)) from None


def ready(ui, empty):
    def check():
        require(not read_is(ui, FAIL, "Startup failed"), "native startup failure")
        return (read_is(ui, READY, "Ready") and read_is(ui, HEADER_READY, "Ready")
                and read_is(ui, EMPTY if empty else ONE_WORLD, "No local worlds" if empty else "1 world"))
    wait("native ready and world list", check)


def cli(binary, profile, *args):
    """Independent read-only verification. No creation, membership or consent writes."""
    allowed = {("identity",), ("transport-id",), ("world", "list"), ("world", "status"),
               ("world", "verify"), ("world", "hosting", "status")}
    require(any(tuple(args[:len(prefix)]) == prefix for prefix in allowed), "QA CLI mutation forbidden")
    if args == ("identity",):
        key = profile / "data" / "identity" / "peer.key"
        require(key.is_file() and not key.is_symlink() and key.stat().st_size == 32,
                "GUI did not initialize a durable identity before inspection")
    # Keep normal warnings enabled; the CLI must send them to stderr.
    # Do not strip mixed output or conceal machine-contract regressions.
    env = profile_env(profile)
    result = subprocess.run([str(sidecar(binary, "swarmcraft")), "--data-dir", str(profile / "data"), *args],
                            env=env, capture_output=True, timeout=20, check=False)
    if result.returncode:
        raise Missing("local read-only CLI evidence not available")
    return result.stdout.decode("utf-8", errors="strict").strip()


def world_from_list(binary, profile):
    listing = cli(binary, profile, "world", "list")
    match = re.fullmatch("(" + WORLD_PATTERN + r")\s+" + re.escape(WORLD_NAME) + r"\s+snapshots=\d+", listing)
    return match.group(1) if match else None


def consent(binary, profile, world):
    value = json.loads(cli(binary, profile, "world", "hosting", "status", world, "--json"))
    require(value.get("world_id") == world and isinstance(value.get("consented"), bool), "invalid hosting status")
    return value


def select_world(ui, world):
    row = Target(f'#worldCards .world-row[data-world-id="{world}"]', WORLD_NAME, ("AXButton",), True)
    click(ui, button("navWorlds", "Worlds"))
    click(ui, row)
    wait("selected world", lambda: read_is(ui, text("selectedName", WORLD_NAME), WORLD_NAME))


def create_world(ui, binary, profile):
    click(ui, button("openCreate", "New world"))
    name = field("createName", "World name")
    wait("native world-name field", lambda: ui.read(name)["enabled"])
    ui.type(name, WORLD_NAME)
    # Catalog-controlled enablement is driven by the real application.
    click(ui, button("createWorld", "Create world"))
    world = wait("GUI-created persisted world", lambda: world_from_list(binary, profile))
    select_world(ui, world)
    status = cli(binary, profile, "world", "status", world)
    require("Visibility: Private" in status,
            "GUI creation did not retain private visibility")
    require("Epoch: 1" in status.splitlines() and "Local member role: Accepted signed member" in status.splitlines(),
            "GUI creation did not establish the signed genesis quorum and first checkpoint")
    require(consent(binary, profile, world)["consented"] is False, "GUI creation implicitly opted in to hosting")
    require("Verified checkpoint:" in cli(binary, profile, "world", "verify", world),
            "initial signed checkpoint not verified")
    return world


def visible(ui, target):
    try:
        ui.read(target)
        return True
    except Missing:
        return False


def open_hosting_contribution(ui):
    """Reach the real candidate preference without changing the DOM."""
    heap = field("hostingContributionHeap", "Java server heap budget (MiB)")
    disclosure = Target("#advancedWorldTools > summary", "Advanced world controls",
                        ("AXDisclosureTriangle", "AXButton"), True)
    wait("hosting preference or advanced controls visible",
         lambda: visible(ui, heap) or visible(ui, disclosure))
    if not visible(ui, heap):
        click(ui, disclosure)
    # The caller still has to prove the exact persisted consent state.
    wait("hosting preference visible", lambda: visible(ui, heap))


def open_diagnostics(ui, candidate):
    disclosure = ("AXDisclosureTriangle", "AXButton")
    if candidate:
        target = button("navDiagnostics", "Diagnostics")
        if not visible(ui, target):
            click(ui, Target("#advancedNavigation > summary", "Tools", disclosure))
    else:
        # The shipping runtime wizard intentionally hides the sidebar's
        # Diagnostics entry, even when Tools is open. Enter from the selected
        # world's actual advanced controls and World details disclosure.
        target = button("openWorldDiagnostics", "Diagnostics")
        details = Target(".details-panel > summary", "World details", disclosure)
        if not visible(ui, target):
            if not visible(ui, details):
                click(ui, Target("#advancedWorldTools > summary", "Advanced world controls",
                                 disclosure, True))
            if not visible(ui, target):
                click(ui, details)
    click(ui, target)


def stop_network(ui, candidate):
    # Reuse Diagnostics when already on that page (the source and guest
    # return here after the native networking actions).
    if not visible(ui, button("stopDaemon", "Stop networking")):
        open_diagnostics(ui, candidate)
    click(ui, button("stopDaemon", "Stop networking"))
    if candidate:
        message = "Desktop networking stopped."
        wait("network stop completed", lambda: read_is(
            ui, text("networkServiceNotice", message, True), message, prefix=True)
            and ui.read(button("stopDaemon", "Stop networking"))["enabled"])
    else:
        wait("network stop completed", lambda: read_is(ui, HEADER_READY, "Ready"))


def udp_listener_in_use(lan_ip, port):
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        if hasattr(socket, "SO_EXCLUSIVEADDRUSE"):
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        try:
            sock.bind((lan_ip, port))
        except OSError as error:
            if error.errno == errno.EADDRINUSE or getattr(error, "winerror", None) == 10048:
                return True
            raise Failure("could not verify the exact local UDP listener") from None
    return False


def start_invite_source(ui, binary, profile, world, lan_ip, candidate):
    stop_network(ui, candidate)
    port = free_port(socket.SOCK_DGRAM, lan_ip)
    address = f"/ip4/{lan_ip}/udp/{port}/quic-v1"
    ui.type(field("daemonListen", "Listen multiaddress"), address)
    click(ui, button("startDaemon", "Start networking"))
    wait("local GUI-started UDP listener", lambda: udp_listener_in_use(lan_ip, port), 35)
    if candidate:
        message = "Networking started."
        wait("network start completed", lambda: read_is(
            ui, text("networkServiceNotice", message, True), message, prefix=True)
            and ui.read(button("startDaemon", "Start networking"))["enabled"])
    else:
        wait("network start completed", lambda: read_is(ui, HEADER_READY, "Ready"))
    transport = cli(binary, profile, "transport-id")
    require(re.fullmatch(r"[1-9A-HJ-NP-Za-km-z]{20,100}", transport), "invalid local transport identity")
    select_world(ui, world)
    click(ui, button("inviteWorld", "Invite"))
    # The launcher deliberately hides manual bootstrap overrides. Test the
    # real automatic invitation, sourced from current daemon connectivity,
    # rather than forcing a hidden input or constructing a token ourselves.
    click(ui, button("createInvite", "Create invite"))
    token_field = field("inviteResult", "Signed invite")
    def signed_invitation():
        require(not visible(ui, text("inviteError", "")), "native invite creation returned a visible error")
        return ui.read(token_field)["value"]
    token = wait("automatic signed invitation rendered", signed_invitation)
    require(len(token) <= 32768 and re.fullmatch(r"scinvite:(?:[0-9a-f]{2})+", token), "invalid invitation shape")
    click(ui, button("cancelInvite", "Done"))
    return token, port


def verified_checkpoint(binary, profile, world):
    raw = cli(binary, profile, "world", "verify", world)
    hashes = re.findall(r"^Verified checkpoint: (\S+)$", raw, re.M)
    require(hashes, "no verified signed checkpoint")
    return hashes[-1]


def run_positive(factory, binary, profile, guest, artifacts, candidate, lan_ip):
    with factory(binary, profile) as ui:
        ready(ui, empty=True)
        require(cli(binary, profile, "world", "list") == "No local worlds.", "fresh profile is not empty")
        identity = cli(binary, profile, "identity")
        ui.screenshot(artifacts / "first-boot.png")
        world = create_world(ui, binary, profile)
        if candidate:
            open_hosting_contribution(ui)
            wait("player-only GUI default", lambda: read_is(ui, PLAYER_ONLY, "Player only — 0 MiB / 0 CPU"))
            require(consent(binary, profile, world)["consented"] is False, "hosting consent defaulted on")
            opt = button("hostingContributionOptIn", "Save explicit opt-in")
            require(not ui.read(opt)["enabled"], "opt-in possible without explicit confirmation")
            ui.type(field("hostingContributionHeap", "Java server heap budget (MiB)"), "512")
            confirm = Target("#hostingContributionConfirm",
                             "I explicitly allow this device to offer the selected Java heap and CPU limit when eligible.", CHECK)
            require(not ui.read(confirm)["checked"], "hosting checkbox preselected")
            click(ui, confirm)
            require(ui.read(confirm)["checked"], "hosting checkbox not checked by UI")
            click(ui, opt)
            wait("durable host preference", lambda: consent(binary, profile, world)["selected_heap_mib"] == 512)
            wait("opted-in GUI", lambda: read_is(ui, OPTED_IN, "Hosting resource preference saved"))
        stop_network(ui, candidate)
        engine = ui.capabilities

    # New OS process and real persisted data; not a reload of the web page.
    with factory(binary, profile) as host:
        ready(host, empty=False)
        require(cli(binary, profile, "identity") == identity and world_from_list(binary, profile) == world,
                "identity or world changed across process restart")
        select_world(host, world)
        if candidate:
            open_hosting_contribution(host)
            wait("opt-in persisted in GUI", lambda: read_is(host, OPTED_IN, "Hosting resource preference saved"))
            require(consent(binary, profile, world)["selected_heap_mib"] == 512, "opt-in did not persist")
            click(host, button("hostingContributionWithdraw", "Player only — withdraw"))
            wait("durable withdrawal", lambda: consent(binary, profile, world)["selected_heap_mib"] == 0)
            wait("withdrawn GUI", lambda: read_is(host, PLAYER_ONLY, "Player only — 0 MiB / 0 CPU"))
        token, port = start_invite_source(host, binary, profile, world, lan_ip, candidate)
        with factory(binary, guest) as joiner:
            ready(joiner, empty=True)
            require(cli(binary, guest, "identity") != identity, "joiner reused host identity")
            click(joiner, button("openJoin", "Join"))
            ui_field = field("joinInvite", "Invite")
            wait("native invitation input", lambda: joiner.read(ui_field)["enabled"])
            joiner.type(ui_field, token)
            require(not joiner.read(Target("#joinHostCandidate", "Request eligibility to host this world in the future",
                                           CHECK, True))["checked"], "joiner requested host role without consent")
            click(joiner, button("joinWorld", "Join world"))
            del token
            def staged_join():
                require(not visible(joiner, text("joinError", "")), "native join returned a visible error")
                return world_from_list(binary, guest) == world
            wait("GUI-joined persisted world", staged_join)
            def accepted():
                status = cli(binary, guest, "world", "status", world)
                if "Local member role: Accepted signed member" not in status:
                    return False
                require("Local host candidate: Not granted" in status, "player-only join unexpectedly granted host role")
                require(consent(binary, guest, world)["consented"] is False, "joiner hosting consent enabled")
                return verified_checkpoint(binary, guest, world) == verified_checkpoint(binary, profile, world)
            wait("signed membership and exact checkpoint synchronization", accepted, 150)
            select_world(joiner, world)
            joiner.screenshot(artifacts / "joined-world.png")
            stop_network(joiner, candidate)
        stop_network(host, candidate)
        wait("explicit host UDP listener released", lambda: not udp_listener_in_use(lan_ip, port), 30)

    if candidate:
        with factory(binary, profile) as ui:
            ready(ui, empty=False)
            select_world(ui, world)
            open_hosting_contribution(ui)
            wait("withdrawal persisted in restarted GUI", lambda: read_is(ui, PLAYER_ONLY, "Player only — 0 MiB / 0 CPU"))
            require(consent(binary, profile, world)["consented"] is False, "withdrawal did not persist")
            stop_network(ui, candidate)
    return {"engine": engine, "boot_create_restart": True, "automatic_signed_invite": True, "real_signed_join_checkpoint": True,
            "host_consent_restart_withdraw": candidate}


def run_negative(factory, binary, profile, artifacts):
    with factory(binary, profile) as ui:
        def observed():
            require(not read_is(ui, READY, "Ready"), "broken package reported a ready device")
            return (read_is(ui, FAIL, "Startup failed") and
                    read_is(ui, BANNER, "SwarmCraft could not start (", prefix=True) and
                    read_is(ui, RETRY, "Retry startup") and ui.read(RETRY)["enabled"])
        wait("visible startup failure and retry", observed)
        ui.screenshot(artifacts / "failure.png")
        click(ui, RETRY)
        time.sleep(1)
        wait("failure remains visible after retry", observed)
        ui.screenshot(artifacts / "retry-failure.png")
    return {"visible_failure": True, "retry_clicked": True, "failure_visible_after_retry": True}


def copy_missing_sidecar(binary, destination):
    source = app_bundle(binary) if platform.system() == "Darwin" else binary.parent
    require(source != Path(source.anchor) and source not in (Path("/usr/bin"), Path("/usr/local/bin"), Path("/bin")),
            "use a dedicated isolated installer directory, not a shared system bin directory")
    require(not destination.exists(), "negative destination already exists")
    shutil.copytree(source, destination, symlinks=True)
    copy = destination / binary.relative_to(source)
    missing = sidecar(copy, "swarmcraft")
    require(missing.is_file() and not missing.is_symlink(), "cannot remove nonregular test sidecar")
    require(missing.resolve().is_relative_to(destination.resolve()), "negative control path escaped copy")
    missing.unlink()
    if platform.system() == "Darwin":
        # Deliberately damaged copy, not production signature evidence. Ad-hoc
        # sign only this QA-owned bundle so Gatekeeper is not the negative test.
        checked(["codesign", "--force", "--deep", "--sign", "-", destination], seconds=90)
        checked(["codesign", "--verify", "--deep", "--strict", destination])
    return copy


def fresh_profile(root, name):
    path = root / name
    # A permissive desktop umask can create group-writable ancestors,
    # which the runtime ownership guard correctly refuses to trust.
    path.mkdir(mode=0o700)
    (path / "data").mkdir(mode=0o700)
    return path


def write_json(path, value):
    with path.open("x", encoding="utf-8") as out:
        json.dump(value, out, indent=2, sort_keys=True)
        out.write("\n")


def prepare_broken(source, destination):
    require(source.is_dir() and not destination.exists(), "fresh broken-build directory required")
    index = source / "index.html"
    require(index.is_file() and (source / "bootstrap.js").is_file()
            and (source / "candidate-startup-guard.js").is_file(), "compiled candidate startup assets required")
    original = index.read_text(encoding="utf-8")
    corrupted, count = re.subn(
        r'(<script\b[^>]*\btype="module"[^>]*\bsrc=")[^"]+(")',
        r'\g<1>/assets/required-entry-deliberately-missing.js\g<2>', original, count=1)
    require(count == 1 and corrupted != original, "compiled module entry not found")
    require(not (source / "assets" / "required-entry-deliberately-missing.js").exists(),
            "negative module entry is not absent")
    require(not destination.resolve().is_relative_to(source.resolve()), "negative output must be outside candidate assets")
    shutil.copytree(source, destination)
    (destination / "index.html").write_text(corrupted, encoding="utf-8")


def run_matrix(args):
    system, arch = target_platform()
    require(args.disposable_account, "explicit --disposable-account required; never use your daily app account")
    lan_ip = private_lan(args.lan_ip)
    records, binaries = {}, {}
    for variant in VARIANTS:
        binary, package, manifest = (Path(p).resolve(strict=True) for p in getattr(args, variant))
        actual = package_record(binary, package, variant)
        require(actual == json.loads(manifest.read_text()), "installed artifact does not match captured manifest")
        records[variant], binaries[variant] = actual, binary
    verify_artifacts(records)
    require(len(set(binaries.values())) == 3, "installer variants cannot share executable path")
    evidence = args.artifacts.resolve()
    evidence.mkdir(mode=0o700)  # never overwrite prior evidence
    report = {"schema": 1, "artifacts": records, "platform": system, "architecture": arch,
              "release_platform_evidence": False, "results": {}, "complete": False}
    stage = "adapter"
    try:
        with tempfile.TemporaryDirectory(prefix="swarmcraft-native-qa-") as temp:
            root = Path(temp)
            if system == "macos":
                require(args.ax_helper is not None, "compiled external macOS Accessibility helper required")
                helper = args.ax_helper.resolve(strict=True)
                require(args.driver is None and args.native_driver is None, "macOS must not use tauri-driver")
                report["adapter"] = {"kind": "macOS Accessibility", "sha256": sha(helper),
                                     "source_sha256": sha(REPO / "scripts/acceptance/native_desktop_ax.swift")}
                factory = lambda binary, profile: Accessibility(binary, profile, helper)
            else:
                require(args.driver is not None and args.native_driver is not None, "explicit native drivers required")
                driver, native = args.driver.resolve(strict=True), args.native_driver.resolve(strict=True)
                expected = "msedgedriver.exe" if system == "windows" else "WebKitWebDriver"
                require(native.name.lower() == expected.lower() and driver.stem.lower() == "tauri-driver",
                        "wrong platform native driver")
                report["adapter"] = {"kind": expected, "tauri_driver_sha256": sha(driver),
                                     "native_driver_sha256": sha(native)}
                factory = lambda binary, profile: WebDriver(binary, profile, driver, native)
            for variant in ("shipping", "candidate"):
                stage = variant
                dest = evidence / variant
                dest.mkdir()
                report["results"][variant] = run_positive(
                    factory, binaries[variant], fresh_profile(root, variant + "-host"),
                    fresh_profile(root, variant + "-guest"), dest, variant == "candidate", lan_ip)
            stage = "broken-module"
            dest = evidence / stage
            dest.mkdir()
            report["results"][stage] = run_negative(factory, binaries["broken"], fresh_profile(root, stage), dest)
            stage = "missing-sidecar"
            dest = evidence / stage
            dest.mkdir()
            copy = copy_missing_sidecar(binaries["candidate"], root / ("Missing.app" if system == "macos" else "missing"))
            report["results"][stage] = run_negative(factory, copy, fresh_profile(root, stage), dest)
            require(sha(sidecar(binaries["candidate"], "swarmcraft")) == records["candidate"]["files"]["swarmcraft"],
                    "original installed sidecar changed")
        report["complete"] = True
        report["release_platform_evidence"] = system != "linux"
    except BaseException as error:
        report["complete"] = False
        report["release_platform_evidence"] = False
        report["failure"] = {"stage": stage, "classification": type(error).__name__}
        # Failure details from the harness are fixed messages, never driver
        # responses, private IDs, invitations or arbitrary exception strings.
        if isinstance(error, Failure):
            report["failure"]["reason"] = str(error)[:240]
        raise
    finally:
        write_json(evidence / "result.json", report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prepare = sub.add_parser("prepare-broken", help="create separate compiled assets with a missing entry module")
    prepare.add_argument("--source", type=Path, default=REPO / "apps/desktop/frontend/dist")
    prepare.add_argument("--destination", type=Path, default=REPO / "apps/desktop/frontend/broken-dist")
    capture = sub.add_parser("capture", help="capture installer + installed binary hashes at the exact clean commit")
    capture.add_argument("--binary", type=Path, required=True)
    capture.add_argument("--package", type=Path, required=True)
    capture.add_argument("--variant", choices=VARIANTS, required=True)
    capture.add_argument("--manifest", type=Path, required=True)
    run = sub.add_parser("run", help="exercise all three installed variants and a disposable missing-sidecar copy")
    for variant in VARIANTS:
        run.add_argument("--" + variant, nargs=3, metavar=("BINARY", "PACKAGE", "MANIFEST"), required=True)
    run.add_argument("--artifacts", type=Path, required=True)
    run.add_argument("--driver", type=Path)
    run.add_argument("--native-driver", type=Path)
    run.add_argument("--ax-helper", type=Path)
    run.add_argument("--lan-ip", required=True, help="local RFC1918 IPv4 address on isolated test LAN")
    run.add_argument("--disposable-account", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "prepare-broken":
            prepare_broken(args.source, args.destination)
        elif args.command == "capture":
            write_json(args.manifest, package_record(args.binary.resolve(strict=True), args.package.resolve(strict=True),
                                                     args.variant))
        else:
            run_matrix(args)
    except Exception as error:
        print("FAIL: native desktop QA: " + type(error).__name__ +
              (": " + str(error) if isinstance(error, Failure) else ""), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
