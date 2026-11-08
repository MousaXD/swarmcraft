#!/usr/bin/env bash
# Agent 10 - explicit USER-AUTHORIZED test opt-in for clean-machine acceptance.
# The release script must call this from its world-creation step ONLY after
# integration of the real public CLI API. This helper does not start a JVM.
set -euo pipefail
if (( $# != 2 )); then
  printf 'Usage: %s <acceptance-data-root> <created-scworld-id>\n' "$0" >&2
  exit 2
fi
DATA="$1"
WORLD="$2"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${SWARMCRAFT_CLI:-$ROOT/target/debug/swarmcraft}"
HEAP="${SWARMCRAFT_ACCEPTANCE_HEAP_MIB:-512}"

fail() { printf 'HOSTING_CONSENT_OPT_IN_FAIL: %s\n' "$*" >&2; exit 1; }
[[ "$WORLD" == scworld:* ]] || fail "world identity must come from the real world-create command"
[[ -x "$CLI" ]] || fail "public swarmcraft CLI binary not found"
[[ "$HEAP" =~ ^[0-9]+$ ]] || fail "heap must be a nonnegative integer MiB"
(( HEAP >= 512 && HEAP <= 65536 )) || fail "positive-hosting heap must be 512..65536 MiB"
command -v python3 >/dev/null || fail "python3 required for strict typed JSON verification"

field() {
  python3 -c '
import json,sys
v=json.load(sys.stdin)
for key in sys.argv[1].split("."):
    v=v[key]
if isinstance(v,bool): print("true" if v else "false")
elif v is None: print("null")
else: print(v)
' "$1"
}
BEFORE="$("$CLI" --data-dir "$DATA" world hosting status "$WORLD" --json)" ||
  fail "public hosting status unavailable (requires approved API integration)"
[[ "$(field selected_heap_mib <<<"$BEFORE")" == 0 ]] ||
  fail "clean-machine positive world was not initially player-only"
[[ "$(field consented <<<"$BEFORE")" == false ]] ||
  fail "world creation unexpectedly authorized hosting"
[[ "$(field may_host_now <<<"$BEFORE")" == false ]] ||
  fail "default-deny readiness violation"

# This is the explicit, auditable test-operator action. Never edit consent JSON.
"$CLI" --data-dir "$DATA" world hosting set "$WORLD" --heap-mib "$HEAP" ||
  fail "user-authorized local opt-in was rejected"
AFTER="$("$CLI" --data-dir "$DATA" world hosting status "$WORLD" --json)"
[[ "$(field selected_heap_mib <<<"$AFTER")" == "$HEAP" ]] ||
  fail "requested heap was not persisted by Rust API"
[[ "$(field consented <<<"$AFTER")" == true ]] ||
  fail "Rust API did not record explicit device consent"
[[ "$(field may_host_now <<<"$AFTER")" == true ]] || {
  REASON="$(field reason <<<"$AFTER")"
  fail "device cannot safely host selected $HEAP MiB heap: $REASON"
}
CAP="$(field hard_process_memory_cap_guaranteed <<<"$AFTER")"
[[ "$CAP" == true || "$CAP" == false ]] ||
  fail "backend did not state whole-process hard-cap support"
printf 'HOSTING_CONSENT_OPT_IN_PASS world=%s heap_mib=%s current_eligibility=true hard_process_memory_cap_guaranteed=%s\n' \
  "$WORLD" "$HEAP" "$CAP"
