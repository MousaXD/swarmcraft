#!/usr/bin/env bash
# Agent 10 - OFFLINE signed join-request regression for player-only peers.
# This cannot prove a network connection, authorized membership or Minecraft
# gameplay. A separate real multi-peer acceptance gate must prove those.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${SWARMCRAFT_CLI:-$ROOT/target/debug/swarmcraft}"
SCRATCH="$(mktemp -d)"
KEEP="${SWARMCRAFT_PLAYER_ONLY_KEEP_DATA:-0}"
cleanup() {
  if [[ "$KEEP" == 1 ]]; then
    printf 'Preserved offline player-only evidence: %s\n' "$SCRATCH" >&2
  else
    rm -rf "$SCRATCH"
  fi
}
trap cleanup EXIT
fail() { printf 'PLAYER_ONLY_JOIN_FAIL: %s\n' "$*" >&2; exit 1; }
[[ -x "$CLI" ]] || fail "real public CLI binary required"
command -v python3 >/dev/null || fail "python3 required to inspect typed public CLI status"
mkdir -p "$SCRATCH/authority" "$SCRATCH/player"
"$CLI" --data-dir "$SCRATCH/authority" init >"$SCRATCH/authority-init.txt"
"$CLI" --data-dir "$SCRATCH/player" init >"$SCRATCH/player-init.txt"
CREATED="$("$CLI" --data-dir "$SCRATCH/authority" world create \
    --name 'Offline player-only join staging' \
    --minecraft 26.1.2 --fabric-loader 0.19.3 \
    --compatibility vanilla-fabric --visibility private)"
WORLD="$(sed -n 's/^World ID: //p' <<<"$CREATED" | tail -n1)"
[[ "$WORLD" == scworld:* ]] || fail "authority world-create did not provide a real world ID"

# Explicit manual address is an OFFLINE fixture, not evidence of reachability:
# No daemon is started, nothing is dialed and this address is not a live host.
INVITE="$("$CLI" --data-dir "$SCRATCH/authority" invite create "$WORLD" \
    --expires-minutes 15 --bootstrap /ip4/1.1.1.1/udp/41000/quic-v1)"
[[ "$INVITE" == scinvite:* ]] || fail "real signed invite was not generated"
STAGED="$("$CLI" --data-dir "$SCRATCH/player" world join "$INVITE")"
grep -q "^Join request staged for:" <<<"$STAGED" ||
  fail "player could not stage a signed join request"
grep -Fqx "World ID: $WORLD" <<<"$STAGED" ||
  fail "player's staged request referred to the wrong world"
"$CLI" --data-dir "$SCRATCH/player" world list >"$SCRATCH/player-worlds.txt"
grep -Fq "$WORLD" "$SCRATCH/player-worlds.txt" ||
  fail "player world identity was not retained after staging"

# Fail, not skip, if the integrated public hosting command is unavailable.
JSON="$("$CLI" --data-dir "$SCRATCH/player" world hosting status "$WORLD" --json)" ||
  fail "player-only hosting status CLI is missing from integration candidate"
python3 -c '
import json, sys
world=sys.argv[1]
report=json.load(sys.stdin)
if report["world_id"] != world:
    sys.exit("PLAYER_ONLY_JOIN_FAIL: incorrect world identity in readiness report")
if report["selected_heap_mib"] != 0 or report["consented"] is not False:
    sys.exit("PLAYER_ONLY_JOIN_FAIL: staging a signed join opted player into hosting")
if report["may_host_now"] is not False:
    sys.exit("PLAYER_ONLY_JOIN_FAIL: nonhosting player falsely eligible as authority")
if report["hard_process_memory_cap_guaranteed"] is not False:
    sys.exit("PLAYER_ONLY_JOIN_FAIL: unsupported hard process memory cap advertised")
' "$WORLD" <<<"$JSON"

echo "PLAYER_ONLY_OFFLINE_STAGE_PASS world=$WORLD hosting=disabled staged_join=true network=false gameplay=false"
