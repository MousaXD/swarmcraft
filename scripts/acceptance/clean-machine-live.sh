#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${SWARMCRAFT_CLI:-$ROOT/target/debug/swarmcraft}"
RUNTIME="${SWARMCRAFT_RUNTIME:-$ROOT/target/debug/swarmcraft-runtime}"
DATA="${SWARMCRAFT_ACCEPTANCE_DATA_DIR:-$(mktemp -d)}"
LOG_DIR="${SWARMCRAFT_ACCEPTANCE_LOG_DIR:-$(mktemp -d)}"
KEEP_DATA="${SWARMCRAFT_ACCEPTANCE_KEEP_DATA:-0}"
LAUNCH_PID=""
FIRST_GATEWAY=""

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

latest_snapshot_number() {
  "$CLI" --data-dir "$DATA" world status "$WORLD" | sed -n 's/^Latest snapshot: //p' | tail -n1
}

assert_snapshot_advanced() {
  local previous="$1"
  local current="$2"
  local label="$3"
  [[ "$previous" =~ ^[0-9]+$ && "$current" =~ ^[0-9]+$ ]] || {
    printf '%s snapshot sequence is not numeric: previous=%s current=%s\n' "$label" "$previous" "$current" >&2
    return 1
  }
  (( current > previous )) || {
    printf '%s did not advance the canonical snapshot sequence: previous=%s current=%s\n' "$label" "$previous" "$current" >&2
    return 1
  }
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

# Query actual Minecraft through the daemon-owned stable gateway. Status
# is deliberately distinct from authenticated account login or gameplay.
probe_minecraft_status() {
  python3 - "$1" <<'PYTHON'
import json
import socket
import struct
import sys

host, port = sys.argv[1].rsplit(":", 1)
assert host == "127.0.0.1"
port = int(port)
assert 0 < port <= 65535

def varint(value):
    encoded = bytearray()
    while True:
        part = value & 127
        value >>= 7
        encoded.append(part | (128 if value else 0))
        if not value:
            return bytes(encoded)

def read_varint(stream):
    value = 0
    for shift in range(0, 35, 7):
        part = stream.read(1)
        if not part:
            raise EOFError("truncated Minecraft status response")
        value |= (part[0] & 127) << shift
        if not part[0] & 128:
            return value
    raise ValueError("invalid Minecraft status VarInt")

with socket.create_connection((host, port), timeout=5) as sock:
    sock.settimeout(25)
    name = b"localhost"
    handshake = (b"\x00" + varint(0xffffffff) + varint(len(name))
                 + name + struct.pack(">H", port) + b"\x01")
    sock.sendall(varint(len(handshake)) + handshake + b"\x01\x00")
    with sock.makefile("rb") as stream:
        length = read_varint(stream)
        assert 1 <= length <= 1048576
        import io
        packet = io.BytesIO(stream.read(length))
        assert len(packet.getbuffer()) == length
        assert read_varint(packet) == 0
        body_length = read_varint(packet)
        body = packet.read()
        assert len(body) == body_length
        status = json.loads(body)
        assert "26.1.2" in status["version"]["name"], status
        print(json.dumps(status))
PYTHON
}

verify_live_gateway() {
  local label="$1" expected_epoch="$2" raw endpoint selected=false
  local deadline=$((SECONDS + 180))
  while (( SECONDS < deadline )); do
    kill -0 "$LAUNCH_PID" 2>/dev/null || return 1
    if [[ "$selected" == false ]] && "$CLI" --data-dir "$DATA" world game-route-select "$WORLD" >/dev/null 2>&1; then
      selected=true
    fi
    if [[ "$selected" == true ]]; then
      raw="$("$CLI" --data-dir "$DATA" world game-route-status "$WORLD" --json 2>/dev/null || true)"
      if [[ -n "$raw" ]] && endpoint="$(python3 -c '
import json, sys
s = json.load(sys.stdin)
assert s["world_id"] == sys.argv[1]
assert s["state"] == "verified"
assert all(s[key] is True for key in (
    "quorum_verified", "hosting_consent_verified", "server_ready", "route_verified"))
assert s["epoch"] == int(sys.argv[2])
assert s["fencing_token"] == int(sys.argv[2])
assert s["authority_peer_id"] and s["snapshot_hash"]
assert s["valid_until_unix_ms"] > s["verified_at_unix_ms"]
assert s["local_gateway"].startswith("127.0.0.1:")
print(s["local_gateway"])
' "$WORLD" "$expected_epoch" <<<"$raw" 2>/dev/null)"; then
        printf '%s\n' "$raw" >"$LOG_DIR/$label.route.json"
        # Once a route is verified, a failed real status exchange is a
        # product failure, not a reason to retry until a green sample.
        probe_minecraft_status "$endpoint" >"$LOG_DIR/$label.minecraft-status.json" || return 1
        if [[ -z "$FIRST_GATEWAY" ]]; then
          FIRST_GATEWAY="$endpoint"
        else
          [[ "$FIRST_GATEWAY" == "$endpoint" ]] || {
            printf 'Gateway address changed after restart: %s -> %s\n' "$FIRST_GATEWAY" "$endpoint" >&2
            return 1
          }
        fi
        printf 'Verified signed gateway and real Minecraft status: %s epoch=%s\n' "$endpoint" "$expected_epoch"
        return 0
      fi
    fi
    sleep 1
  done
  printf 'No verified live gateway for %s at epoch %s\n' "$WORLD" "$expected_epoch" >&2
  return 1
}

start_runtime_and_wait_ready() {
  local label="$1" expected_epoch="$2"
  # A signed quorum-of-one is still a quorum. The actual owner must be the
  # daemon, not a detached standalone Java launcher with no live registry.
  if [[ "$expected_epoch" != "1" ]]; then
    "$CLI" --data-dir "$DATA" world wake "$WORLD"
  fi
  "$CLI" --data-dir "$DATA" daemon --listen /ip4/127.0.0.1/udp/0/quic-v1 >"$LOG_DIR/$label.stdout" 2>"$LOG_DIR/$label.stderr" &
  LAUNCH_PID=$!
  if ! wait_phase ready 240 || ! verify_live_gateway "$label" "$expected_epoch"; then
    cat "$LOG_DIR/$label.stdout" >&2 || true
    cat "$LOG_DIR/$label.stderr" >&2 || true
    return 1
  fi
}

stop_runtime_safely() {
  "$CLI" --data-dir "$DATA" world stop "$WORLD"
  wait_phase sleeping 180
  if [[ -n "$LAUNCH_PID" ]]; then
    # Sleeping is published only after Java exits and the signed
    # checkpoint is committed. The daemon itself is intentionally
    # long-lived; stop it now to exercise a real process restart.
    kill -0 "$LAUNCH_PID" 2>/dev/null
    kill "$LAUNCH_PID"
    wait "$LAUNCH_PID" 2>/dev/null || true
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
INITIAL_SNAPSHOT="$(latest_snapshot_number)"
[[ "$INITIAL_SNAPSHOT" =~ ^[0-9]+$ ]] || {
  printf 'Initial canonical snapshot sequence is not numeric: %s\n' "$INITIAL_SNAPSHOT" >&2
  exit 1
}

INITIAL_HOSTING="$("$CLI" --data-dir "$DATA" world hosting status "$WORLD" --json)"
[[ "$(json_field "$INITIAL_HOSTING" consented)" == "false" ]]
[[ "$(json_field "$INITIAL_HOSTING" selected_heap_mib)" == "0" ]]
[[ "$(json_field "$INITIAL_HOSTING" may_host_now)" == "false" ]]
[[ "$(json_field "$INITIAL_HOSTING" hard_process_memory_cap_guaranteed)" == "false" ]]

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

printf '== explicit per-device Minecraft hosting opt-in (not world authority) ==\n'
HOST_HEAP="${SWARMCRAFT_ACCEPTANCE_HEAP_MIB:-512}"
[[ "$HOST_HEAP" =~ ^[0-9]+$ ]] && (( HOST_HEAP >= 512 && HOST_HEAP <= 65536 )) || {
  printf "Invalid explicit acceptance heap MiB: %s\n" "$HOST_HEAP" >&2
  exit 1
}
"$CLI" --data-dir "$DATA" world hosting set "$WORLD" --heap-mib "$HOST_HEAP" --json >"$LOG_DIR/hosting-consent.json"
HOSTING_CURRENT="$("$CLI" --data-dir "$DATA" world hosting status "$WORLD" --json)"
[[ "$(json_field "$HOSTING_CURRENT" consented)" == "true" ]]
[[ "$(json_field "$HOSTING_CURRENT" selected_heap_mib)" == "$HOST_HEAP" ]]
[[ "$(json_field "$HOSTING_CURRENT" hard_process_memory_cap_guaranteed)" == "false" ]]
if [[ "$(json_field "$HOSTING_CURRENT" may_host_now)" != "true" ]]; then
  printf "Explicit opt-in saved but current memory admission DENIED; cannot launch real Minecraft: %s\n" "$(json_field "$HOSTING_CURRENT" reason)" >&2
  exit 1
fi
printf 'Explicit opt-in verified for real Java/Fabric launch: %s MiB (memory eligibility is rechecked at spawn)\n' "$HOST_HEAP"
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
start_runtime_and_wait_ready first-launch 1
WORLD_DIR="$(find "$DATA/runtime" -mindepth 2 -maxdepth 2 -type d -name world -print -quit)"
[[ -n "$WORLD_DIR" && -s "$WORLD_DIR/level.dat" ]] || {
  printf 'Minecraft reached Ready but did not create a usable level.dat\n' >&2
  exit 1
}
printf 'persisted through canonical checkpoint\n' >"$WORLD_DIR/swarmcraft-clean-machine-marker.txt"
stop_runtime_safely
"$CLI" --data-dir "$DATA" world verify "$WORLD"
FIRST_STOP_STATUS="$("$CLI" --data-dir "$DATA" world status "$WORLD")"
FIRST_STOP_SNAPSHOT="$(latest_snapshot_number)"
assert_snapshot_advanced "$INITIAL_SNAPSHOT" "$FIRST_STOP_SNAPSHOT" "First safe stop"
grep -q '^Migration: Sleeping$' <<<"$FIRST_STOP_STATUS"

printf '== backend/process restart with persisted runtime configuration ==\n'
RESTART_STATUS="$(runtime_status)"
[[ "$(json_field "$RESTART_STATUS" ready)" == "true" ]]
[[ "$(json_field "$RESTART_STATUS" eula_accepted)" == "true" ]]
[[ "$(json_field "$RESTART_STATUS" launch_configured)" == "true" ]]
start_runtime_and_wait_ready second-launch 2
WORLD_DIR="$(find "$DATA/runtime" -mindepth 2 -maxdepth 2 -type d -name world -print -quit)"
[[ -s "$WORLD_DIR/level.dat" ]]
grep -q '^persisted through canonical checkpoint$' "$WORLD_DIR/swarmcraft-clean-machine-marker.txt"
stop_runtime_safely
"$CLI" --data-dir "$DATA" world verify "$WORLD"
SECOND_STOP_STATUS="$("$CLI" --data-dir "$DATA" world status "$WORLD")"
SECOND_STOP_SNAPSHOT="$(latest_snapshot_number)"
assert_snapshot_advanced "$FIRST_STOP_SNAPSHOT" "$SECOND_STOP_SNAPSHOT" "Second safe stop"
grep -q '^Migration: Sleeping$' <<<"$SECOND_STOP_STATUS"

echo "CLEAN_MACHINE_LIVE_PASS world=$WORLD snapshots=$INITIAL_SNAPSHOT,$FIRST_STOP_SNAPSHOT,$SECOND_STOP_SNAPSHOT"
