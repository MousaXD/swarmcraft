#!/usr/bin/env bash
# Verify the actual staged disk image and the app it contains, not just a build-tree app.
set -euo pipefail
test "$#" -eq 4
directory="$1"
commit="$2"
arch="$3"
team="$4"
[[ "$commit" =~ ^[a-f0-9]{40}$ ]]
[[ "$team" =~ ^[A-Z0-9]{10}$ ]]
case "$arch" in arm64|x86_64) ;; *) exit 1 ;; esac
shopt -s nullglob
images=("$directory"/*.dmg)
test "${#images[@]}" -eq 1
dmg="${images[0]}"
test -f "$dmg"
test ! -L "$dmg"
test -s "$dmg"
digest="$(shasum -a 256 "$dmg" | cut -d ' ' -f 1)"
size="$(stat -f '%z' "$dmg")"
xcrun stapler validate "$dmg"
mount="$(mktemp -d)"
mounted=0
cleanup() {
    if test "$mounted" = 1; then
        hdiutil detach "$mount" >/dev/null || return 1
    fi
    rmdir "$mount"
}
trap cleanup EXIT
hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$mount" "$dmg"
mounted=1
app="$mount/SwarmCraft.app"
test -d "$app"
test ! -L "$app"
codesign --verify --deep --strict --verbose=2 "$app"
signature="$(codesign -dv --verbose=4 "$app" 2>&1)"
printf '%s\n' "$signature" | grep -F 'Authority=Developer ID Application:'
printf '%s\n' "$signature" | grep -Fx "TeamIdentifier=$team"
xcrun stapler validate "$app"
spctl --assess --type execute --verbose=2 "$app"
executable="$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$app/Contents/Info.plist")"
[[ "$executable" =~ ^[A-Za-z0-9_.-]+$ ]]
test "$executable" != "."
test "$executable" != ".."
test "$(lipo -archs "$app/Contents/MacOS/$executable")" = "$arch"
hdiutil detach "$mount"
mounted=0
test "$(shasum -a 256 "$dmg" | cut -d ' ' -f 1)" = "$digest"
test "$(stat -f '%z' "$dmg")" = "$size"
python3 - "$directory" "$commit" "$arch" "$team" "$dmg" "$size" "$digest" <<'PY'
import json
import sys
from pathlib import Path
directory, commit, arch, team, dmg, size, digest = sys.argv[1:]
receipt = {
    "schema": "swarmcraft-native-signing-v1", "commit": commit,
    "platform": "macos-" + arch, "file": Path(dmg).name,
    "bytes": int(size), "sha256": digest,
    "checks": ["dmg_staple", "mounted_app_codesign", "developer_id_team",
               "app_staple", "gatekeeper", "binary_architecture"],
    "identity": team,
}
with (Path(directory) / ("SIGNING_EVIDENCE_MACOS_" + arch + ".json")).open("x", encoding="utf-8") as f:
    json.dump(receipt, f)
    f.write("\n")
PY
