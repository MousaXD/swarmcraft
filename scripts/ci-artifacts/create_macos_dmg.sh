#!/usr/bin/env bash
set -euo pipefail

app="$1"
dmg="$2"
volume="$3"
test -d "$app"
mkdir -p "$(dirname "$dmg")"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT
# DiskImages can be busy briefly on a shared runner. Other errors are fatal.
for attempt in 1 2 3; do
  if hdiutil create -volname "$volume" -srcfolder "$app" -ov -format UDZO "$dmg" >"$log" 2>&1; then
    cat "$log"
    break
  fi
  cat "$log" >&2
  if ! grep -qi 'Resource busy' "$log" || [ "$attempt" -eq 3 ]; then
    exit 1
  fi
  rm -f "$dmg"
  echo "DiskImages resource busy. Retrying in $((attempt * 5)) seconds."
  sleep "$((attempt * 5))"
done

hdiutil verify "$dmg"
test -s "$dmg"
