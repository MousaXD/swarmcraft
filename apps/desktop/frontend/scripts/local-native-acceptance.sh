#!/usr/bin/env bash
# Local-only replacement for the removed Svelte candidate hosted GUI workflow.
# Use disposable extracted installers, never the daily installed application.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$repo"

if [[ "${1:-}" == prepare-broken ]]; then
    source_dir=apps/desktop/frontend/dist
    broken_dir=apps/desktop/frontend/broken-dist
    test -s "$source_dir/index.html"
    test -s "$source_dir/bootstrap.js"
    mkdir "$broken_dir" # Refuse an existing directory instead of overwriting it.
    cp -a "$source_dir/." "$broken_dir/"
    python3 - <<'PY'
from pathlib import Path
import re
index = Path('apps/desktop/frontend/broken-dist/index.html')
html = index.read_text()
corrupted, count = re.subn(
    r'(<script\b[^>]*\btype="module"[^>]*\bsrc=")[^"]+(")',
    r'\g<1>/assets/required-entry-deliberately-missing.js\g<2>',
    html, count=1,
)
assert count == 1 and 'bootstrap.js' in html and 'bootstrap.js' in corrupted
index.write_text(corrupted)
assert Path('apps/desktop/frontend/broken-dist/bootstrap.js').is_file()
PY
    printf '%s\n' 'Build the separate broken candidate using frontend/tauri.candidate.broken.conf.json; do not change shipping configuration.'
    exit 0
fi

if [[ "$#" != 4 || "$1" != verify ]]; then
    echo "Usage: $0 prepare-broken | verify /path/to/extracted/positive/swarmcraft-desktop /path/to/svelte-broken-installed/usr/bin/swarmcraft-desktop /path/to/new-evidence" >&2
    exit 2
fi
positive="$(realpath "$2")"
broken="$(realpath "$3")"
test "$positive" != "$broken"
for binary in "$positive" "$broken"; do
    test -x "$binary"
    for sidecar in swarmcraft swarmcraft-host swarmcraft-runtime swarmcraft-import; do
        test -x "$(dirname "$binary")/$sidecar"
    done
done
artifacts="$(realpath -m "$4")"
mkdir "$artifacts"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
export WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1 GDK_BACKEND=x11

run_gui() {
    local scenario="$1" binary="$2" script="$3" seconds="$4" profile="${5:-$1}"
    mkdir -p "$scratch/$profile/home" "$scratch/$profile/xdg" "$scratch/$profile/data"
    HOME="$scratch/$profile/home" XDG_DATA_HOME="$scratch/$profile/xdg" \
        SWARMCRAFT_DATA_DIR="$scratch/$profile/data" \
        xvfb-run -a -s '-screen 0 1280x1024x24' timeout "${seconds}s" python3 "$script" \
            --binary "$binary" --artifacts "$artifacts/$scenario"
}
verify_visible_failure() {
    python3 - "$1" <<'PY'
import json
from pathlib import Path
import sys
state = json.loads(Path(sys.argv[1]).read_text())
assert state['startup'] == 'failed'
assert state['error']
assert state['status'] == 'Startup failed'
assert state['retryVisible']
PY
}
python3 -m unittest discover -s scripts/acceptance -p 'test_packaged_desktop_webdriver.py'
python3 -m unittest discover -s apps/desktop/frontend/scripts -p 'test_visual_failure_evidence.py'
run_gui positive "$positive" scripts/acceptance/packaged-desktop-webdriver.py 240
run_gui visual "$positive" apps/desktop/frontend/scripts/visual-window-acceptance.py 240 positive
# Remove a sidecar only from our private disposable copy. The original
# extracted application and the daily installed app remain unchanged.
mkdir "$scratch/missing-sidecar"
cp -a "$(dirname "$positive")/." "$scratch/missing-sidecar/"
rm -- "$scratch/missing-sidecar/swarmcraft"
if run_gui negative-control "$scratch/missing-sidecar/swarmcraft-desktop" scripts/acceptance/packaged-desktop-webdriver.py 120; then
    echo 'BROKEN: missing-sidecar negative control unexpectedly passed' >&2
    exit 1
fi
verify_visible_failure "$artifacts/negative-control/last-state.json"
if run_gui broken-bootstrap "$broken" scripts/acceptance/packaged-desktop-webdriver.py 140; then
    echo 'BROKEN: broken-module negative control unexpectedly passed' >&2
    exit 1
fi
verify_visible_failure "$artifacts/broken-bootstrap/last-state.json"
