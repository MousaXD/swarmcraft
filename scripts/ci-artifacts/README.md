# Agent 08: CI artifact integrity and HTTP 504 remediation

Issue: https://github.com/MousaXD/swarmcraft/issues/97

## Verified evidence

Required Validation run 42466519745, attempt 2 (PR #90):
- Linux package job 128691726499 produced a .deb and then received HTTP 504 fetching Tauri AppRun/linuxdeploy.
- Windows package job 128691726449 compiled the binary, but NSIS 3.11 and nsis_tauri_utils-v0.5.3 downloads failed HTTP 504, despite three full build retries.
- Live player journey job 128691726180 reached managed Java installation; the official Adoptium-backed curl returned 504 three times.
- Main Desktop Installers run 42466519780 also failed the Linux AppImage stage.

These establish failure at external HTTP acquisition, not a proven root cause at a particular network intermediary. The failed runners reported eastus/eastus3; no Rust safety-code regression was demonstrated.

## Remediation

scripts/ci-artifacts/acquire.py preloads exact Tauri CLI 2.11.4 tool files, verifying SHA-256 on every restored or downloaded file before copying into Tauri's OS-specific tools directory. Source files have hardcoded expected names and official GitHub-hosted URLs; official GitHub REST release-asset IDs are an alternate route to the very same content. There is no third-party mirror, substituted version, or unverifiable fallback. The helper retries four rounds with bounded waits (3, 8, 18 seconds), uses atomic publishing, enforces Linux x86_64 / Windows x86_64, and refuses invalid symlinks or checksum mismatches. Windows NSIS archives are extracted with traversal/symlink/size checks and the DLL checksum verified before publication.

The clean-machine live workflow retries only on an observed curl HTTP 429/5xx; all retries rerun the entire genuine Minecraft/Fabric/EULA/managed Java journey from a new, empty data directory, retaining logs separately. Product failures, checksum errors and missing success markers remain blocking. Java continues to be verified against the official Adoptium metadata SHA-256 by the existing Rust installer.

## SHA-256 pins

- AppRun-x86_64 (apprun-old): f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f
- linuxdeploy-x86_64.AppImage (linuxdeploy): e762bea85c8eb0d4b3508d46e5c1f037f717d0f9303ae3b4aafc8b04991fa1ef
- linuxdeploy-plugin-gtk.sh (commit b5eb8d05b4c0ed40107fe2158c5d8527f94568ef): cb379f9b0733e9ad9f8bd78f8c2fa038aef2478523bb7d4c8e64ff6a1ea3501a
- linuxdeploy-plugin-gstreamer.sh (commit 2a2e67491c32995a3f279ad0ecbe77abd512b42a): c107b49d84edbffc6ab226ed1007e0626a4f7aa2c3a36b7782bef62351d49e94
- linuxdeploy-plugin-appimage.AppImage (continuous, hash-pinned): 49d6a17160675a6bd1781699aae6bdf7692d98552e02a3671d2183d10547842e
- nsis-3.11.zip: c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1
- nsis_tauri_utils-v0.5.3 DLL: 5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709

The AppRun, NSIS and DLL digests are present in GitHub upstream release metadata. The older linuxdeploy release does not publish a digest in that metadata; its SHA-256 was cross-checked against an independently published artifact manifest. Tauri itself deliberately patches linuxdeploy during bundling; only pristine immutable bytes are stored in the separate Actions cache. The upstream output plugin has a moving 'continuous' URL; the pinned checksum intentionally causes a cold-cache failure if that asset changes, pending reviewed re-pinning.

## Negative tests

From the repository root:

    python -m unittest discover -s scripts/ci-artifacts -p 'test_*.py'

Fifteen tests cover invalid/missing artifacts, unavailable upstream, bounded HTTP retries, checksum failure, poisoned cache, wrong architecture and NSIS ZIP traversal/version confusion. Additional tests verify that official GitHub asset API metadata rejects substituted identities, duplicate assets and mismatched published digests, that an HTTP 504 on the canonical asset URL can succeed through the same official API asset ID with the exact pinned bytes, and that corrupted fallback bytes fail immediately. The required Linux and Windows Desktop jobs run these tests before package builds.

## Boundaries

No production Rust or frontend code changed; no required check removed. Production release.yml was deliberately not edited because Agent 00 approval was required. That workflow still relies on Tauri's default acquisition and must be separately reviewed before a production tag. Issue #97 and PR #90 stay open until coordinator integration and Agent 07 certification.
