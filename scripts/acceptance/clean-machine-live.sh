#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${SWARMCRAFT_CLI:-$ROOT/target/debug/swarmcraft}"
RUNTIME="${SWARMCRAFT_RUNTIME:-$ROOT/target/debug/swarmcraft-runtime}"
DATA="${SWARMCRAFT_ACCEPTANCE_DATA_DIR:-$(mktemp -d)}"
LOG_DIR="${SWARMCRAFT_ACCEPTANCE_LOG_DIR:-$(mktemp -d)}"
KEEP_DATA="${SWARMCRAFT_ACCEPTANCE_KEEP_DATA:-0}"
LAUNCH_PID=""

cleanup() {
  if [[ -n "$LAUNCH_PID" ]] && kill -0 "$LAUNCH_PID" 2>/dev/null; then
    kill "$LAUNCH_PID" 2>/dev/null || true
    wait "$LAUNCH_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP_DATA" != "1" ]]; then
    rm -rf "$DATA" "$LOG_DIR"
  else
    printf 'Preserved acceptance data: %s\nPreserved acceptance logs: %s\n' "$DATA" "$LOG_DIR"
  fi
}
trap cleanup EXIT

json_field() {
  local json="$1"
  local field="$2"
  python3 -c '
import json, sys
value = json.load(sys.stdin)
for part in sys.argv[1].split("."):
    value = value[part]
if isinstance(value, bool):
    print("true" if value else "false")
elif value is None:
    print("null")
else:
    print(value)
' "$field" <<<"$json"
}

runtime_status() {
  "$RUNTIME" --data-dir "$DATA" status "$WORLD"
}

migration_status() {
  "$CLI" --data-dir "$DATA" world migration-status "$WORLD" --json 2>/dev/null || true
}

wait_phase() {
  local wanted="$1"
  local timeout_seconds="${2:-180}"
  local deadline=$((SECONDS + timeout_seconds))
  while (( SECONDS < deadline )); do
    local raw phase ready
    raw="$(migration_status)"
    if [[ -n "$raw" ]]; then
      phase="$(json_field "$raw" phase 2>/dev/null || true)"
      ready="$(json_field "$raw" runtime_ready 2>/dev/null || true)"
      if [[ "$phase" == "$wanted" ]]; then
        if [[ "$wanted" != "ready" || "$ready" == "true" ]]; then
          return 0
        fi
      fi
      if [[ "$phase" == "failed" || "$phase" == "blocked" ]]; then
        printf 'Migration entered %s:\n%s\n' "$phase" "$raw" >&2
        return 1
      fi
    fi
    sleep 1
  done
  printf 'Timed out waiting for migration phase %s\n' "$wanted" >&2
  migration_status >&2 || true
  return 1
}

start_runtime_and_wait_ready() {
  local label="$1"
  "$RUNTIME" --data-dir "$DATA" launch "$WORLD" >"$LOG_DIR/$label.stdout" 2>"$LOG_DIR/$label.stderr" &
  LAUNCH_PID=$!
  wait_phase ready 240 || {
    cat "$LOG_DIR/$label.stdout" >&2 || true
    cat "$LOG_DIR/$label.stderr" >&2 || true
    return 1
  }
}

stop_runtime_safely() {
  "$CLI" --data-dir "$DATA" world stop "$WORLD"
  wait_phase sleeping 180
  if [[ -n "$LAUNCH_PID" ]]; then
    local deadline=$((SECONDS + 90))
    while kill -0 "$LAUNCH_PID" 2>/dev/null && (( SECONDS < deadline )); do
      sleep 1
    done
    if kill -0 "$LAUNCH_PID" 2>/dev/null; then
      printf 'Managed runtime did not exit after safe sleep\n' >&2
      return 1
    fi
    wait "$LAUNCH_PID"
    LAUNCH_PID=""
  fi
}

mkdir -p "$DATA" "$LOG_DIR"
[[ -x "$CLI" ]] || { printf 'Missing SwarmCraft CLI at %s\n' "$CLI" >&2; exit 2; }
[[ -x "$RUNTIME" ]] || { printf 'Missing SwarmCraft runtime at %s\n' "$RUNTIME" >&2; exit 2; }
[[ -n "${SWARMCRAFT_FABRIC_MOD_JAR:-}" && -f "$SWARMCRAFT_FABRIC_MOD_JAR" ]] || {
  printf 'SWARMCRAFT_FABRIC_MOD_JAR must point at the candidate Fabric artifact\n' >&2
  exit 2
}

printf '== clean-machine initial state ==\n'
[[ -z "$(find "$DATA" -mindepth 1 -print -quit)" ]] || { printf 'Acceptance data directory is not empty\n' >&2; exit 1; }
"$CLI" --data-dir "$DATA" init
CREATE_OUTPUT="$("$CLI" --data-dir "$DATA" world create \
  --name 'Clean Machine Acceptance' \
  --minecraft 26.1.2 \
  --fabric-loader 0.19.3 \
  --compatibility vanilla-fabric \
  --visibility private)"
printf '%s\n' "$CREATE_OUTPUT"
WORLD="$(sed -n 's/^World ID: //p' <<<"$CREATE_OUTPUT" | tail -n1)"
[[ "$WORLD" == scworld:* ]] || { printf 'Could not parse created world ID\n' >&2; exit 1; }

INITIAL_STATUS="$(runtime_status)"
[[ "$(json_field "$INITIAL_STATUS" eula_accepted)" == "false" ]]
[[ "$(json_field "$INITIAL_STATUS" launch_configured)" == "false" ]]
[[ ! -e "$DATA/runtime-locks" ]] || [[ -z "$(find "$DATA/runtime-locks" -type f -print -quit 2>/dev/null)" ]]

BEFORE_STATE="$("$CLI" --data-dir "$DATA" world status "$WORLD" | sed -n 's/^State root: //p' | tail -n1)"

printf '== explicit EULA refusal / retry-safe preparation ==\n'
"$RUNTIME" --data-dir "$DATA" plan "$WORLD" >"$LOG_DIR/plan.json"
"$RUNTIME" --data-dir "$DATA" install "$WORLD" >"$LOG_DIR/install-without-eula.json"
REFUSED_STATUS="$(runtime_status)"
[[ "$(json_field "$REFUSED_STATUS" eula_accepted)" == "false" ]]
[[ "$(json_field "$REFUSED_STATUS" launch_configured)" == "false" ]]
AFTER_REFUSAL_STATE="$("$CLI" --data-dir "$DATA" world status "$WORLD" | sed -n 's/^State root: //p' | tail -n1)"
[[ "$BEFORE_STATE" == "$AFTER_REFUSAL_STATE" ]]
if migration_status | grep -q '"runtime_ready": true'; then
  printf 'Minecraft became ready despite EULA refusal\n' >&2
  exit 1
fi

printf '== explicit EULA acceptance and managed installation ==\n'
"$RUNTIME" --data-dir "$DATA" install "$WORLD" --accept-eula >"$LOG_DIR/install-accepted.json"
VERIFIED_STATUS="$("$RUNTIME" --data-dir "$DATA" verify "$WORLD")"
[[ "$(json_field "$VERIFIED_STATUS" ready)" == "true" ]]
[[ "$(json_field "$VERIFIED_STATUS" eula_accepted)" == "true" ]]
[[ "$(json_field "$VERIFIED_STATUS" launch_configured)" == "true" ]]

# The release workflow deliberately runs this script with JAVA_HOME removed and
# a base PATH. Assert that the test really exercised managed Java rather than
# silently succeeding on a pre-installed compatible JVM.
MANAGED_JAVA="$(python3 -c '
import json, sys
status = json.load(sys.stdin)
java = next(component for component in status["components"] if component["kind"] == "java")
print("true" if java.get("managed") else "false")
' <<<"$VERIFIED_STATUS")"
[[ "$MANAGED_JAVA" == "true" ]] || {
  printf 'Clean-machine live gate did not exercise managed Java installation\n' >&2
  exit 1
}

printf '== first real Minecraft/Fabric launch ==\n'
start_runtime_and_wait_ready first-launch
WORLD_DIR="$(find "$DATA/runtime" -mindepth 2 -maxdepth 2 -type d -name world -print -quit)"
[[ -n "$WORLD_DIR" && -s "$WORLD_DIR/level.dat" ]] || {
  printf 'Minecraft reached Ready but did not create a usable level.dat\n' >&2
  exit 1
}
printf 'persisted through canonical checkpoint\n' >"$WORLD_DIR/swarmcraft-clean-machine-marker.txt"
stop_runtime_safely
"$CLI" --data-dir "$DATA" world verify "$WORLD"
FIRST_STOP_STATUS="$("$CLI" --data-dir "$DATA" world status "$WORLD")"
grep -q '^Latest snapshot: 2$' <<<"$FIRST_STOP_STATUS"
grep -q '^Migration: Sleeping$' <<<"$FIRST_STOP_STATUS"

printf '== backend/process restart with persisted runtime configuration ==\n'
RESTART_STATUS="$(runtime_status)"
[[ "$(json_field "$RESTART_STATUS" ready)" == "true" ]]
[[ "$(json_field "$RESTART_STATUS" eula_accepted)" == "true" ]]
[[ "$(json_field "$RESTART_STATUS" launch_configured)" == "true" ]]
start_runtime_and_wait_ready second-launch
WORLD_DIR="$(find "$DATA/runtime" -mindepth 2 -maxdepth 2 -type d -name world -print -quit)"
[[ -s "$WORLD_DIR/level.dat" ]]
grep -q '^persisted through canonical checkpoint$' "$WORLD_DIR/swarmcraft-clean-machine-marker.txt"
stop_runtime_safely
"$CLI" --data-dir "$DATA" world verify "$WORLD"
SECOND_STOP_STATUS="$("$CLI" --data-dir "$DATA" world status "$WORLD")"
grep -q '^Latest snapshot: 3$' <<<"$SECOND_STOP_STATUS"

echo "CLEAN_MACHINE_LIVE_PASS world=$WORLD"
