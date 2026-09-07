#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SWARM="$ROOT/target/debug/swarmcraft"
RUNTIME="$ROOT/target/debug/swarmcraft-runtime"
FABRIC_JAR="${SWARMCRAFT_FABRIC_MOD_JAR:-}"
if [[ -z "$FABRIC_JAR" ]]; then
  FABRIC_JAR="$(find "$ROOT/minecraft/fabric/build/libs" -maxdepth 1 -type f -name 'swarmcraft-fabric-*.jar' ! -name '*-sources.jar' | head -n 1)"
fi
[[ -x "$SWARM" && -x "$RUNTIME" ]] || { echo "SwarmCraft debug binaries are missing" >&2; exit 1; }
[[ -f "$FABRIC_JAR" ]] || { echo "built SwarmCraft Fabric bridge is missing" >&2; exit 1; }

JAVA_MAJOR="$(java -XshowSettings:properties -version 2>&1 | awk -F= '/java.specification.version/ {gsub(/[[:space:]]/, "", $2); print $2; exit}')"
[[ "$JAVA_MAJOR" =~ ^[0-9]+$ ]] || { echo "cannot determine Java major" >&2; exit 1; }
(( JAVA_MAJOR >= 25 )) || { echo "Java $JAVA_MAJOR is below shipped minimum 25" >&2; exit 1; }

TMP="$(mktemp -d)"
DATA="$TMP/data"
LAUNCH_LOG="$TMP/launch.log"
CHAOS_LOG="$TMP/chaos.log"
DAEMON_LOG="$TMP/daemon.log"
SUP_PID=""
DAEMON_PID=""
OLD_JAVA_PID=""
cleanup() {
  set +e
  [[ -n "$OLD_JAVA_PID" ]] && kill -CONT "$OLD_JAVA_PID" 2>/dev/null || true
  [[ -n "$SUP_PID" ]] && kill "$SUP_PID" 2>/dev/null || true
  [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

phase() {
  "$SWARM" --data-dir "$DATA" world migration-status "$WORLD" --json 2>/dev/null |
    python3 -c 'import json,sys; print(json.load(sys.stdin).get("phase", ""))' 2>/dev/null || true
}
runtime_ready() {
  "$SWARM" --data-dir "$DATA" world migration-status "$WORLD" --json 2>/dev/null |
    python3 -c 'import json,sys; print(str(json.load(sys.stdin).get("runtime_ready", False)).lower())' 2>/dev/null || true
}
wait_phase() {
  local expected="$1" limit="${2:-240}"
  for ((i=0; i<limit; i++)); do
    [[ "$(phase)" == "$expected" ]] && return 0
    sleep 1
  done
  echo "timed out waiting for migration phase $expected; current=$(phase)" >&2
  "$SWARM" --data-dir "$DATA" world migration-status "$WORLD" --json >&2 || true
  return 1
}
wait_pid_dead() {
  local pid="$1" limit="${2:-60}"
  for ((i=0; i<limit*10; i++)); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  return 1
}
record_java_pid() {
  python3 - "$PROCESS_RECORD" <<'PY'
import json,sys
with open(sys.argv[1], encoding='utf-8') as f:
    value=json.load(f)
pid=value.get('java_pid')
if not isinstance(pid, int) or pid <= 0:
    raise SystemExit('runtime process record has no live java_pid')
print(pid)
PY
}

CREATE_OUT="$($SWARM --data-dir "$DATA" world create \
  --name 'Agent 6 Real Runtime' \
  --minecraft 26.1.2 \
  --fabric-loader 0.19.3 \
  --visibility private)"
WORLD="$(printf '%s\n' "$CREATE_OUT" | awk '/^World ID:/ {print $3; exit}')"
[[ "$WORLD" == scworld:* ]] || { echo "could not parse created world ID" >&2; printf '%s\n' "$CREATE_OUT" >&2; exit 1; }
WORLD_HEX="${WORLD#scworld:}"
PROCESS_RECORD="$DATA/control/$WORLD_HEX/runtime-process.json"
RUNTIME_DIR="$DATA/runtime/$WORLD_HEX"

export SWARMCRAFT_FABRIC_MOD_JAR="$FABRIC_JAR"
"$RUNTIME" --data-dir "$DATA" install "$WORLD" --accept-eula >"$TMP/install.json"
"$RUNTIME" --data-dir "$DATA" verify "$WORLD" >"$TMP/verify.json"
python3 - "$TMP/verify.json" <<'PY'
import json,sys
value=json.load(open(sys.argv[1], encoding='utf-8'))
assert value['minecraft_version'] == '26.1.2', value
assert tuple(map(int, value['fabric_loader_version'].split('.'))) >= (0,19,3), value
assert value['required_java_major'] >= 25, value
assert value['ready'] is True, value
PY

echo "AGENT6_RUNTIME_INSTALL=PASS world=$WORLD java=$JAVA_MAJOR fabric_bridge=$FABRIC_JAR"

# Normal real supported launch: authenticated Fabric handshake -> Ready, then safe stop/checkpoint.
"$RUNTIME" --data-dir "$DATA" launch "$WORLD" >"$LAUNCH_LOG" 2>&1 &
SUP_PID=$!
wait_phase ready 300
[[ "$(runtime_ready)" == "true" ]] || { echo "runtime never became authenticated-ready" >&2; exit 1; }
[[ -f "$PROCESS_RECORD" ]] || { echo "persistent Java ownership record missing at Ready" >&2; exit 1; }
JAVA_PID="$(record_java_pid)"
kill -0 "$JAVA_PID"
grep -q "SwarmCraft lifecycle bridge connected to local controller" "$RUNTIME_DIR/logs/latest.log"
"$SWARM" --data-dir "$DATA" world stop "$WORLD"
wait "$SUP_PID"
SUP_PID=""
wait_phase sleeping 120
[[ ! -e "$PROCESS_RECORD" ]] || { echo "clean shutdown left a live runtime process fence" >&2; cat "$PROCESS_RECORD" >&2; exit 1; }
echo "AGENT6_SUPPORTED_RUNTIME=PASS world=$WORLD java=$JAVA_MAJOR phase=sleeping"

# Hard-death chaos: freeze the real Java process so a replacement controller definitely races a live orphan.
"$RUNTIME" --data-dir "$DATA" launch "$WORLD" >"$CHAOS_LOG" 2>&1 &
SUP_PID=$!
wait_phase ready 300
[[ -f "$PROCESS_RECORD" ]]
OLD_JAVA_PID="$(record_java_pid)"
kill -0 "$OLD_JAVA_PID"
printf 'must-survive-old-java\n' > "$RUNTIME_DIR/agent6-no-reset-sentinel"

# The daemon remains alive while the independent runtime controller is destroyed.
"$SWARM" --data-dir "$DATA" daemon --listen /ip4/127.0.0.1/udp/0/quic-v1 >"$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
sleep 2
kill -0 "$DAEMON_PID"

kill -STOP "$OLD_JAVA_PID"
kill -KILL "$SUP_PID"
wait "$SUP_PID" 2>/dev/null || true
SUP_PID=""

# Keep Java frozen long enough for the daemon's replacement supervisor to attempt recovery.
sleep 4
kill -0 "$DAEMON_PID"
kill -0 "$OLD_JAVA_PID"
[[ -f "$PROCESS_RECORD" ]] || { echo "persistent ownership record disappeared while orphan Java lived" >&2; exit 1; }
[[ -f "$RUNTIME_DIR/agent6-no-reset-sentinel" ]] || { echo "replacement controller reset runtime while old Java PID was alive" >&2; exit 1; }
RECORDED="$(record_java_pid)"
[[ "$RECORDED" == "$OLD_JAVA_PID" ]] || { echo "runtime fence was reused by a different Java PID while old process lived" >&2; exit 1; }
STATUS_JSON="$($SWARM --data-dir "$DATA" world migration-status "$WORLD" --json 2>/dev/null || true)"
if [[ -n "$STATUS_JSON" ]]; then
  printf '%s' "$STATUS_JSON" | python3 -c 'import json,sys; v=json.load(sys.stdin); r=(v.get("failure_reason") or "").lower(); assert ("still alive" in r) or (v.get("phase") in ("ready","failed","preparing_runtime")), v'
fi
echo "AGENT6_ORPHAN_FENCE=PASS old_java_pid=$OLD_JAVA_PID daemon_pid=$DAEMON_PID sentinel=preserved"

# Resume the orphan. Fabric must observe controller loss, save, and stop itself.
kill -CONT "$OLD_JAVA_PID"
BRIDGE_LOSS=0
for ((i=0; i<300; i++)); do
  if [[ -f "$RUNTIME_DIR/logs/latest.log" ]] && grep -q "SwarmCraft controller ownership lost:.*Saving and stopping server" "$RUNTIME_DIR/logs/latest.log"; then
    BRIDGE_LOSS=1
  fi
  kill -0 "$OLD_JAVA_PID" 2>/dev/null || break
  sleep 0.1
done
[[ "$BRIDGE_LOSS" == 1 ]] || { echo "Fabric controller-loss save/stop evidence not observed" >&2; exit 1; }
wait_pid_dead "$OLD_JAVA_PID" 30 || { echo "orphan Java did not terminate after Fabric fail-closed controller loss" >&2; exit 1; }
OLD_JAVA_PID=""

# Daemon may now clear the stale dead-PID fence, reset safely, and relaunch a new Java runtime.
wait_phase ready 300
[[ -f "$PROCESS_RECORD" ]]
NEW_JAVA_PID="$(record_java_pid)"
kill -0 "$NEW_JAVA_PID"
[[ ! -f "$RUNTIME_DIR/agent6-no-reset-sentinel" ]] || { echo "runtime was not safely rebuilt after old Java death" >&2; exit 1; }
echo "AGENT6_SAFE_RELAUNCH=PASS new_java_pid=$NEW_JAVA_PID daemon_pid=$DAEMON_PID"

"$SWARM" --data-dir "$DATA" world stop "$WORLD"
wait_phase sleeping 120
kill "$DAEMON_PID" 2>/dev/null || true
wait "$DAEMON_PID" 2>/dev/null || true
DAEMON_PID=""

echo "AGENT6_REAL_RUNTIME_ACCEPTANCE=PASS world=$WORLD"
