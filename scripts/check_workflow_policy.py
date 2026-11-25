#!/usr/bin/env python3
"""Static policy checks for SwarmCraft GitHub Actions release governance."""

from __future__ import annotations

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
JOB_LINE = re.compile(r"^  ([A-Za-z0-9_-]+):\s*$")
USES_LINE = re.compile(r"^\s*(?:-\s+)?uses:\s*([^\s#]+)(?:\s+#\s*(.+))?\s*$")

ALLOWED_CONTENTS_WRITE = {
    ("main-installers.yml", "publish"),
    ("release.yml", "publish"),
}


# Keep a deliberately small, explicit workflow allowlist. New workflow
# files or event types require a reviewed policy change, not an agent task.
ALLOWED_EVENTS = {
    "ci.yml": {"push", "pull_request"},
    "required-validation.yml": {"workflow_call", "workflow_dispatch"},
    "main-installers.yml": {"workflow_dispatch"},
    "release.yml": {"workflow_dispatch"},
    "three-machine-certification.yml": {"workflow_dispatch"},
}
DOCS_IGNORE = {"docs/**", "*.md", "**/*.md"}


def top_level_block(text: str, name: str) -> str:
    """Require the repository's explicit, block-style workflow structure.

    This lightweight guard is not a general YAML parser. Refuse alternative
    trigger spellings/structures instead of assuming they are harmless.
    """
    lines = text.splitlines()
    starts = [i for i, line in enumerate(lines) if line == name + ":"]
    if len(starts) != 1:
        fail(f"expected exactly one explicit {name}: block")
    start = starts[0] + 1
    end = next(
        (i for i in range(start, len(lines))
         if lines[i] and not lines[i][0].isspace() and not lines[i].startswith("#")),
        len(lines),
    )
    return "\n".join(lines[start:end])


def check_local_first(path: Path, text: str) -> None:
    if path.name not in ALLOWED_EVENTS:
        fail(f"{path.name}: unapproved workflow (temporary/agent workflows are forbidden)")
    events_block = top_level_block(text, "on")
    events = []
    for line in events_block.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if line.startswith("  ") and not line.startswith("   "):
            match = re.fullmatch(r"  ([a-z_]+):", line)
            if match is None:
                fail(f"{path.name}: use explicit block-style event declarations")
            events.append(match[1])
    if len(events) != len(set(events)) or set(events) != ALLOWED_EVENTS[path.name]:
        fail(f"{path.name}: unexpected trigger(s); expected {sorted(ALLOWED_EVENTS[path.name])}")
    if re.search(r"(?m)^\s*(?:schedule|cron)\s*:", events_block):
        fail(f"{path.name}: scheduled workflows are forbidden")
    if not re.search(r"(?m)^  cancel-in-progress: true\s*$", top_level_block(text, "concurrency")):
        fail(f"{path.name}: concurrency must cancel obsolete runs")

    blocks = job_blocks(text)
    if path.name == "ci.yml":
        for event in ("push", "pull_request"):
            event_block = re.search(
                rf"(?ms)^  {event}:\s*\n(.*?)(?=^  [a-z_]+:|\Z)", events_block
            )
            if not event_block or not re.search(r"(?m)^    branches: \[main\]\s*$", event_block[1]):
                fail(f"{path.name}: {event} must be restricted to main")
            ignore = re.search(r"(?m)^    paths-ignore:\s*\n((?:^      - .+\n?)+)", event_block[1])
            if ignore is None or set(re.findall(r"(?m)^      - ['\"]([^'\"]+)['\"]\s*$", ignore[1])) != DOCS_IGNORE:
                fail(f"{path.name}: {event} must ignore docs and Markdown-only changes")
        if set(blocks) != {"metadata"}:
            fail(f"{path.name}: automatic CI must contain only the lightweight metadata job")
        metadata = blocks["metadata"]
        if "runs-on: ubuntu-latest" not in metadata or "timeout-minutes: 5" not in metadata:
            fail(f"{path.name}: metadata runner must have a five-minute limit")
        if "uses: ./" in metadata or "strategy:" in metadata:
            fail(f"{path.name}: automatic CI cannot invoke reusable or matrix jobs")
        if re.search(r"(?m)^\s*- uses: (?!actions/checkout@)", metadata):
            fail(f"{path.name}: automatic CI may only use the pinned checkout action")
        # The sole automatic job must not acquire dependencies or run builds,
        # tests, live source probes, patching scripts, or other arbitrary commands.
        allowed_commands = [
            "python3 scripts/check_workflow_policy.py",
            "python3 scripts/check_release_version.py",
        ]
        lines = metadata.splitlines()
        run_indexes = [i for i, line in enumerate(lines) if line.strip().startswith("run:")]
        if len(run_indexes) != 1 or lines[run_indexes[0]].strip() != "run: |":
            fail(f"{path.name}: expected one metadata-only script")
        commands = []
        for line in lines[run_indexes[0] + 1:]:
            if line and not line.startswith("          "):
                break
            if line.strip():
                commands.append(line.strip())
        if commands != allowed_commands:
            fail(f"{path.name}: unexpected automatic CI commands")
        if "Required validation gate" in metadata:
            fail(f"{path.name}: metadata alone must not satisfy the final validation status")
    elif path.name == "required-validation.yml":
        if set(blocks) != {"preflight", "native", "gate"}:
            fail(f"{path.name}: final validation must have preflight, native and gate jobs")
        preflight, native, gate = (blocks[n] for n in ("preflight", "native", "gate"))
        for required in ('test "$LOCAL_SHA" = "$GITHUB_SHA"', "inputs.local_validation_sha"):
            if required not in preflight:
                fail(f"{path.name}: missing exact-SHA local-first attestation")
        if "needs: preflight" not in native or "os: [windows-latest, macos-latest]" not in native:
            fail(f"{path.name}: native verification must follow preflight and use Windows/macOS only")
        if ("name: Required validation gate" not in gate or "needs: [preflight, native]" not in gate
            or 'test "$PREFLIGHT_RESULT" = success' not in gate or 'test "$NATIVE_RESULT" = success' not in gate):
            fail(f"{path.name}: final status must require successful preflight and native verification")
    elif path.name in {"main-installers.yml", "release.yml"}:
        for job in ("linux", "fabric"):
            if "runs-on: [self-hosted, linux, swarmcraft-release]" not in blocks.get(job, ""):
                fail(f"{path.name}: {job} packaging must use the dedicated local release builder")
    elif path.name == "three-machine-certification.yml":
        for job, block in blocks.items():
            if "runs-on: [self-hosted, linux, swarmcraft-certification]" not in block:
                fail(f"{path.name}: {job} must use an explicitly provisioned local certification runner")

    for line in text.splitlines():
        match = USES_LINE.match(line)
        if match and match[1].startswith("./.github/workflows/"):
            if path.name not in {"main-installers.yml", "release.yml"} or match[1] != "./.github/workflows/required-validation.yml":
                fail(f"{path.name}: unapproved reusable workflow invocation")

    if re.search(r"(?i)(?:--test\s+live_sources|scripts/acceptance/clean-machine-live\.sh|SWARMCRAFT_LIVE_[A-Z_]+)", text):
        fail(f"{path.name}: live third-party service probes must run locally, outside hosted Actions")


def fail(message: str) -> None:
    raise SystemExit(f"workflow policy failed: {message}")


def job_blocks(text: str) -> dict[str, str]:
    lines = text.splitlines()
    in_jobs = False
    current: str | None = None
    blocks: dict[str, list[str]] = {}
    for line in lines:
        if line == "jobs:":
            in_jobs = True
            current = None
            continue
        if not in_jobs:
            continue
        match = JOB_LINE.match(line)
        if match:
            current = match.group(1)
            blocks[current] = [line]
            continue
        if current is not None:
            blocks[current].append(line)
    return {name: "\n".join(lines_) for name, lines_ in blocks.items()}


def check_actions(path: Path, text: str) -> None:
    for lineno, line in enumerate(text.splitlines(), 1):
        match = USES_LINE.match(line)
        if not match:
            continue
        target, comment = match.groups()
        if target.startswith("./"):
            continue
        if "@" not in target:
            fail(f"{path.name}:{lineno}: external uses entry has no ref: {target}")
        _, ref = target.rsplit("@", 1)
        if not FULL_SHA.fullmatch(ref):
            fail(f"{path.name}:{lineno}: external action is not pinned to a full commit SHA: {target}")
        if not comment or not comment.strip():
            fail(f"{path.name}:{lineno}: pinned action lacks a human-readable version/ref comment")


def check_permissions(path: Path, text: str) -> None:
    if re.search(r"(?m)^permissions:\s*write-all\s*$", text):
        fail(f"{path.name}: workflow-level write-all is forbidden")
    if not re.search(r"(?m)^permissions:\s*$", text):
        fail(f"{path.name}: workflow must declare top-level permissions")

    current_job: str | None = None
    in_jobs = False
    for lineno, line in enumerate(text.splitlines(), 1):
        if line == "jobs:":
            in_jobs = True
            current_job = None
            continue
        if in_jobs:
            match = JOB_LINE.match(line)
            if match:
                current_job = match.group(1)
        if "contents: write" in line:
            allowed = (path.name, current_job or "") in ALLOWED_CONTENTS_WRITE
            if not allowed:
                fail(
                    f"{path.name}:{lineno}: contents: write is allowed only on the final publisher job"
                )


def require_validation_dependency(path: Path, builder_jobs: tuple[str, ...]) -> None:
    text = path.read_text(encoding="utf-8")
    blocks = job_blocks(text)
    validation = blocks.get("validation", "")
    if "uses: ./.github/workflows/required-validation.yml" not in validation:
        fail(f"{path.name}: validation job must call required-validation.yml")
    if "local_validation_sha: ${{ inputs.local_validation_sha }}" not in validation:
        fail(f"{path.name}: final validation requires the exact locally validated SHA")
    if path.name == "main-installers.yml" and "expected_ref: refs/heads/main" not in validation:
        fail(f"{path.name}: preview validation must be bound to main")

    for job in builder_jobs:
        block = blocks.get(job)
        if not block:
            fail(f"{path.name}: expected release job {job!r} is missing")
        if "validation" not in re.findall(r"needs:[^\n]*|needs:\s*\[[^\]]*\]", block):
            # Multi-line/inline YAML is easier to validate by requiring the token
            # within the job block near its needs declaration.
            needs_index = block.find("needs:")
            if needs_index < 0 or "validation" not in block[needs_index : needs_index + 180]:
                fail(f"{path.name}: {job} does not depend on exact-SHA validation")

    publish = blocks.get("publish", "")
    if "contents: write" not in publish:
        fail(f"{path.name}: final publisher must declare its narrowly scoped contents: write permission")
    # Prevent an otherwise-green validation from publishing a missing platform,
    # stale artifact group, tampered package, or unverified SHA-256 sidecar.
    channel = "preview" if path.name == "main-installers.yml" else "production"
    verify = blocks.get("verify-packages", "")
    for name, block in (("publish", publish), ("verify-packages", verify)):
        if name == "verify-packages" and channel != "preview":
            continue
        if "scripts/ci-release/verify_artifacts.py" not in block:
            fail(f"{path.name}: {name} must invoke the release-artifact verifier")
        if f"--channel {channel}" not in block or '--commit "$GITHUB_SHA"' not in block:
            fail(f"{path.name}: {name} must bind artifact verification to its SHA and channel")
        if "merge-multiple: false" not in block or "path: release-downloads" not in block:
            fail(f"{path.name}: {name} must retain exact-run artifact source group boundaries")
    if channel == "preview":
        if "needs: [validation, linux, windows, macos, fabric]" not in verify or "contents: read" not in verify:
            fail(f"{path.name}: read-only verification must require all preview artifact builders")
        if "needs: [validation, linux, windows, macos, fabric, verify-packages]" not in publish:
            fail(f"{path.name}: publisher must wait for all artifact builders and read-only verification")
        if "scripts/ci-release/publish_preview.py" not in publish or "--assets release-assets" not in publish:
            fail(f"{path.name}: preview publisher must stage and verify its uploads before publication")
        if re.search(r"gh\s+release\s+(?:delete|create|edit|upload)\b", publish):
            fail(f"{path.name}: preview must use the staged publisher for release changes")
    elif "release-assets/*" not in publish:
        fail(f"{path.name}: publisher must use only verified staged assets")


def check_native_signing_receipts(release: str) -> None:
    blocks = job_blocks(release)
    for name, command in (
        ("windows", "./scripts/ci-release/verify_windows_signing.ps1 -Directory dist -Commit $env:GITHUB_SHA -ExpectedThumbprint $env:EXPECTED_SIGNER_THUMBPRINT"),
        ("macos", 'bash scripts/ci-release/verify_macos_signing.sh dist "$GITHUB_SHA" "$RELEASE_ARCH" "$APPLE_TEAM_ID"'),
    ):
        block = blocks.get(name, "")
        upload = block.find("uses: actions/upload-artifact@")
        check = block.find(command)
        if check < 0 or upload < 0 or check >= upload:
            fail(f"release.yml: {name} must verify exact staged package before artifact upload")
    if "EXPECTED_SIGNER_THUMBPRINT: ${{ steps.windows-signer.outputs.thumbprint }}" not in blocks["windows"]:
        fail("release.yml: Windows receipt must bind expected production certificate")
    if "RELEASE_ARCH: ${{ matrix.arch }}" not in blocks["macos"]:
        fail("release.yml: macOS receipt must bind the matrix architecture")



def main() -> None:
    paths = sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml"))
    if {path.name for path in paths} != set(ALLOWED_EVENTS):
        fail("workflow files differ from the reviewed local-first allowlist")
    for path in paths:
        text = path.read_text(encoding="utf-8")
        check_local_first(path, text)
        check_actions(path, text)
        check_permissions(path, text)

    require_validation_dependency(
        WORKFLOWS / "main-installers.yml", ("linux", "windows", "macos", "fabric", "verify-packages", "publish")
    )
    require_validation_dependency(
        WORKFLOWS / "release.yml", ("linux", "windows", "macos", "fabric", "publish")
    )

    release = (WORKFLOWS / "release.yml").read_text(encoding="utf-8")
    check_native_signing_receipts(release)
    if "release_tag: ${{ github.ref_name }}" not in release:
        fail("release.yml must bind the exact tag name into release identity validation")
    if "python3 scripts/check_release_credentials.py" not in release:
        fail("release.yml must fail closed through the production credential policy")
    if 'test "$GITHUB_REF_TYPE" = tag' not in release or 'test "$LOCAL_SHA" = "$GITHUB_SHA"' not in release:
        fail("release.yml must require a version tag and exact local-validation attestation")
    # Presence of credentials alone does not certify the produced binaries.
    # Verify Authenticode and Apple's developer identity/stapled tickets
    # *after* signing/notarization and before artifact publication.
    if release.count("verify /pa /all") < 2:
        fail("release.yml must independently verify signed Windows sidecars and installer")
    for required in (
        "codesign --verify --deep --strict",
        "Authority=Developer ID Application:",
        'xcrun stapler validate "$app"',
        'xcrun stapler validate "${dmg[0]}"',
        "spctl --assess --type execute",
    ):
        if required not in release:
            fail(f"release.yml must prove macOS notarization/signing: {required}")


    gradle = (ROOT / "minecraft/fabric/gradle.properties").read_text(encoding="utf-8")
    if re.search(r"(?im)^loom_version=.*snapshot", gradle):
        fail("Fabric Loom build tooling must not use a mutable snapshot coordinate")

    print(f"Workflow policy accepted {len(paths)} workflow files.")


if __name__ == "__main__":
    main()
