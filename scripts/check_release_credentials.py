#!/usr/bin/env python3
"""Fail closed unless all production desktop signing/notarization secrets are present."""

from __future__ import annotations

import os

REQUIRED = (
    "WINDOWS_CERTIFICATE_BASE64",
    "WINDOWS_CERTIFICATE_PASSWORD",
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_ID",
    "APPLE_PASSWORD",
    "APPLE_TEAM_ID",
)


def main() -> None:
    missing = [name for name in REQUIRED if not os.environ.get(name, "").strip()]
    if missing:
        raise SystemExit(
            "production release policy failed: missing required signing/notarization "
            f"credentials: {', '.join(missing)}"
        )
    print("Production release signing/notarization credential policy satisfied.")


if __name__ == "__main__":
    main()
