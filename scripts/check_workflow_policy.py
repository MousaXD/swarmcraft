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
    if "run_network_soak: true" not in validation:
        fail(f"{path.name}: release-candidate validation must include network soak")

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


def main() -> None:
    paths = sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml"))
    if not paths:
        fail("no workflow files found")
    for path in paths:
        text = path.read_text(encoding="utf-8")
        check_actions(path, text)
        check_permissions(path, text)

    require_validation_dependency(
        WORKFLOWS / "main-installers.yml", ("linux", "windows", "macos", "fabric", "publish")
    )
    require_validation_dependency(
        WORKFLOWS / "release.yml", ("linux", "windows", "macos", "fabric", "publish")
    )

    release = (WORKFLOWS / "release.yml").read_text(encoding="utf-8")
    if "release_tag: ${{ github.ref_name }}" not in release:
        fail("release.yml must bind the exact tag name into release identity validation")
    if "python3 scripts/check_release_credentials.py" not in release:
        fail("release.yml must fail closed through the production credential policy")

    gradle = (ROOT / "minecraft/fabric/gradle.properties").read_text(encoding="utf-8")
    if re.search(r"(?im)^loom_version=.*snapshot", gradle):
        fail("Fabric Loom build tooling must not use a mutable snapshot coordinate")

    print(f"Workflow policy accepted {len(paths)} workflow files.")


if __name__ == "__main__":
    main()
