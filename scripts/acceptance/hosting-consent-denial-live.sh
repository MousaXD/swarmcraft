#!/usr/bin/env bash
# Agent 10 - real runtime negative hosting-consent acceptance.
# Runs ONLY when explicitly invoked against an integrated CLI; no CI skip or fake JVM.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${SWARMCRAFT_CLI:-$ROOT/target/debug/swarmcraft}"
RUNTIME="${SWARMCRAFT_RUNTIME:-$ROOT/target/debug/swarmcraft-runtime}"
SCRATCH="$(mktemp -d)"
DATA="${SWARMCRAFT_HOSTING_DENIAL_DATA_DIR:-$SCRATCH/data}"
LOG="$SCRATCH/log"
KEEP="${SWARMCRAFT_HOSTING_DENIAL_KEEP_DATA:-0}"

cleanup() {
  if [[ "$KEEP" == "1" ]]; then
    printf 'Preserved logs: %s\nPreserved acceptance data: %s\n' "$LOG" "$DATA" >&2
    # The caller owns external data directories; never delete them.
    return
  fi
  rm -rf "$SCRATCH"
}
trap cleanup EXIT
fail() { printf 'HOSTING_DENIAL_FAIL: %s\n' "$*" >&2; exit 1; }
[[ -x "$CLI" && -x "$RUNTIME" ]] || fail "real CLI and runtime binaries are required"
[[ -n "${SWARMCRAFT_FABRIC_MOD_JAR:-}" && -f "$SWARMCRAFT_FABRIC_MOD_JAR" ]] ||
  fail "provide the actual candidate Fabric artifact in SWARMCRAFT_FABRIC_MOD_JAR"
command -v python3 >/dev/null || fail "python3 is required for JSON and process checks"
mkdir -p "$DATA" "$LOG"
[[ -z "$(find "$DATA" -mindepth 1 -print -quit)" ]] ||
  fail "acceptance data root must be empty; refusing to modify existing worlds"

field() {
  python3 -c '
import json,sys
value=json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value=value[key]
if isinstance(value,bool):
    print("true" if value else "false")
elif value is None:
    print("null")
else:
    print(value)
' "$1"
}

"$CLI" --data-dir "$DATA" init
CREATE="$("$CLI" --data-dir "$DATA" world create \
  --name 'Explicit Hosting Denial' --minecraft 26.1.2 \
  --fabric-loader 0.19.3 --compatibility vanilla-fabric --visibility private)"
WORLD="$(sed -n 's/^World ID: //p' <<<"$CREATE" | tail -n1)"
[[ "$WORLD" == scworld:* ]] || fail "no real world identity was produced"
INITIAL_SNAPSHOT="$("$CLI" --data-dir "$DATA" world status "$WORLD" |
  sed -n 's/^Latest snapshot: //p' | tail -n1)"
[[ "$INITIAL_SNAPSHOT" =~ ^[0-9]+$ ]] ||
  fail "world-create did not establish an initial canonical snapshot"

# Missing public command is a genuine failing dependency, never a test skip.
DEFAULT="$("$CLI" --data-dir "$DATA" world hosting status "$WORLD" --json)" ||
  fail "hosting CLI unavailable: integrate Agent 03 API first"
[[ "$(field selected_heap_mib <<<"$DEFAULT")" == 0 ]] || fail "unexpected nonzero default heap"
[[ "$(field consented <<<"$DEFAULT")" == false ]] || fail "world creation implicitly opted in"
[[ "$(field may_host_now <<<"$DEFAULT")" == false ]] || fail "default-deny readiness violated"

# Make EULA and Java/loader genuinely ready FIRST, so a subsequent launch
# error cannot be falsely attributed to missing runtime components or EULA.
"$RUNTIME" --data-dir "$DATA" plan "$WORLD" >"$LOG/plan.json"
"$RUNTIME" --data-dir "$DATA" install "$WORLD" --accept-eula >"$LOG/install.json"
VERIFY="$("$RUNTIME" --data-dir "$DATA" verify "$WORLD")"
[[ "$(field ready <<<"$VERIFY")" == true ]] || fail "managed runtime not ready"
[[ "$(field eula_accepted <<<"$VERIFY")" == true ]] || fail "EULA not accepted explicitly"
[[ "$(field launch_configured <<<"$VERIFY")" == true ]] || fail "launch not configured"
MANAGED="$(python3 -c '
import json,sys
components=json.load(sys.stdin)["components"]
java=next(component for component in components if component["kind"]=="java")
print("true" if java.get("managed") else "false")
' <<<"$VERIFY")"
[[ "$MANAGED" == true ]] || fail "did not use managed Java"

# Execute real public runtime in an isolated process group. Timeouts fail,
# with cleanup; no synthetic JVM, mocked admission, or silently ignored errors.
denied_launch() {
  local case_name="$1"
  local target_world="$2"
  python3 - "$RUNTIME" "$DATA" "$target_world" "$LOG/$case_name" <<'PY'
import os, signal, subprocess, sys, time
runtime, data, world, log = sys.argv[1:]
p = subprocess.Popen([runtime, "--data-dir", data, "launch", world],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)

def signal_group(sig):
    # A launch may spawn Java and exit early. Clean the whole detached group,
    # not just the runtime parent, or a bad regression can leave Java running.
    try:
        os.killpg(p.pid, sig)
        return True
    except ProcessLookupError:
        return False

timed_out = False
try:
    out, err = p.communicate(timeout=45)
except subprocess.TimeoutExpired:
    timed_out = True
    signal_group(signal.SIGTERM)
    try:
        out, err = p.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        signal_group(signal.SIGKILL)
        out, err = p.communicate()

# Reap and reject any still-running child JVM despite a parent error exit.
orphan_group = signal_group(signal.SIGTERM)
if orphan_group:
    time.sleep(1)
    signal_group(signal.SIGKILL)

with open(log+".stdout", "w") as f:
    f.write(out)
with open(log+".stderr", "w") as f:
    f.write(err)
if timed_out:
    sys.exit("HOSTING_DENIAL_FAIL: launch did not promptly reject missing/zero consent")
if orphan_group:
    sys.exit("HOSTING_DENIAL_FAIL: runtime left a background process group alive")
if p.returncode == 0:
    sys.exit("HOSTING_DENIAL_FAIL: real runtime returned success without consent")
reason = err.lower()
if "hosting consent" not in reason or not any(
    word in reason for word in ("absent", "player-only", "disabled", "withdrawn")
):
    sys.exit("HOSTING_DENIAL_FAIL: rejected for unrelated reason: " + err[-1200:])
PY
  if [[ -d "$DATA/runtime" ]] &&
    find "$DATA/runtime" -type f -name level.dat -print -quit | grep -q .; then
    fail "Minecraft wrote a world despite hosting denial"
  fi
}

printf '== missing consent must deny actual runtime launch ==\n'
denied_launch absent "$WORLD"
AFTER_INSTALL="$("$CLI" --data-dir "$DATA" world hosting status "$WORLD" --json)"
[[ "$(field selected_heap_mib <<<"$AFTER_INSTALL")" == 0 ]] ||
  fail "EULA or runtime installation implicitly opted into hosting"

# Complete the first case before using a distinct world. This ensures that
# any failed-state bookkeeping cannot affect the explicit-zero negative case.
"$CLI" --data-dir "$DATA" world verify "$WORLD" >"$LOG/absent-world-verify.txt"
ABSENT_AFTER_SNAPSHOT="$("$CLI" --data-dir "$DATA" world status "$WORLD" |
  sed -n 's/^Latest snapshot: //p' | tail -n1)"
[[ "$ABSENT_AFTER_SNAPSHOT" == "$INITIAL_SNAPSHOT" ]] ||
  fail "absent-consent denial unexpectedly advanced canonical snapshot history"

printf '== explicit zero heap must deny a separately prepared real runtime ==\n'
ZERO_CREATE="$("$CLI" --data-dir "$DATA" world create \
  --name 'Explicit Zero-MiB Hosting Denial' --minecraft 26.1.2 \
  --fabric-loader 0.19.3 --compatibility vanilla-fabric --visibility private)"
ZERO_WORLD="$(sed -n 's/^World ID: //p' <<<"$ZERO_CREATE" | tail -n1)"
[[ "$ZERO_WORLD" == scworld:* && "$ZERO_WORLD" != "$WORLD" ]] ||
  fail "zero-consent fixture must have a distinct real world identity"
ZERO_INITIAL_SNAPSHOT="$("$CLI" --data-dir "$DATA" world status "$ZERO_WORLD" |
  sed -n 's/^Latest snapshot: //p' | tail -n1)"
[[ "$ZERO_INITIAL_SNAPSHOT" =~ ^[0-9]+$ ]] ||
  fail "zero-consent world did not establish an initial canonical snapshot"
ZERO_DEFAULT="$("$CLI" --data-dir "$DATA" world hosting status "$ZERO_WORLD" --json)"
[[ "$(field selected_heap_mib <<<"$ZERO_DEFAULT")" == 0 ]] ||
  fail "second world unexpectedly received a nonzero initial heap"
[[ "$(field consented <<<"$ZERO_DEFAULT")" == false ]] ||
  fail "second world was implicitly opted into hosting"

# Zero is a deliberate public CLI action; never edit consent JSON directly.
"$CLI" --data-dir "$DATA" world hosting set "$ZERO_WORLD" --heap-mib 0 >"$LOG/set-zero.txt"
ZERO="$("$CLI" --data-dir "$DATA" world hosting status "$ZERO_WORLD" --json)"
[[ "$(field selected_heap_mib <<<"$ZERO")" == 0 ]] || fail "zero MiB did not persist"
[[ "$(field consented <<<"$ZERO")" == false ]] || fail "zero MiB still consented"
[[ "$(field may_host_now <<<"$ZERO")" == false ]] || fail "zero MiB reported eligible"

# Prepare real Java/Fabric plus explicit EULA for this second world, so a
# denied launch cannot be excused by missing components or EULA refusal.
"$RUNTIME" --data-dir "$DATA" plan "$ZERO_WORLD" >"$LOG/zero-plan.json"
"$RUNTIME" --data-dir "$DATA" install "$ZERO_WORLD" --accept-eula >"$LOG/zero-install.json"
ZERO_VERIFY="$("$RUNTIME" --data-dir "$DATA" verify "$ZERO_WORLD")"
[[ "$(field ready <<<"$ZERO_VERIFY")" == true ]] || fail "zero-consent runtime not ready"
[[ "$(field eula_accepted <<<"$ZERO_VERIFY")" == true ]] ||
  fail "zero-consent runtime EULA not explicitly accepted"
[[ "$(field launch_configured <<<"$ZERO_VERIFY")" == true ]] ||
  fail "zero-consent launch not configured"
ZERO_AFTER_INSTALL="$("$CLI" --data-dir "$DATA" world hosting status "$ZERO_WORLD" --json)"
[[ "$(field selected_heap_mib <<<"$ZERO_AFTER_INSTALL")" == 0 ]] ||
  fail "zero heap changed during managed runtime installation"
[[ "$(field consented <<<"$ZERO_AFTER_INSTALL")" == false ]] ||
  fail "managed runtime installation implicitly opted in zero-consent world"
denied_launch zero "$ZERO_WORLD"

"$CLI" --data-dir "$DATA" world verify "$ZERO_WORLD" >"$LOG/zero-world-verify.txt"
ZERO_AFTER_SNAPSHOT="$("$CLI" --data-dir "$DATA" world status "$ZERO_WORLD" |
  sed -n 's/^Latest snapshot: //p' | tail -n1)"
[[ "$ZERO_AFTER_SNAPSHOT" == "$ZERO_INITIAL_SNAPSHOT" ]] ||
  fail "zero-consent denial unexpectedly advanced canonical snapshot history"
echo "HOSTING_CONSENT_NEGATIVE_PASS worlds=$WORLD,$ZERO_WORLD cases=absent,zero real_runtime=true managed_java=true snapshots_unchanged=true"
