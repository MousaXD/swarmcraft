#!/usr/bin/env bash
# Run ONLY inside a newly allocated disposable systemd scope/service.
set -euo pipefail
[[ $# -gt 0 ]]
member="$(sed -n 's/^0:://p' /proc/self/cgroup)"
root="/sys/fs/cgroup${member}"
[[ "$member" = /* && "$member" != / ]]
[[ "$(basename "$root")" =~ ^swarmcraft-memory-cap-test-[a-zA-Z0-9_-]+\.(scope|service)$ ]]
[[ "$(realpath "$root")" = "$root" ]]
[[ -w "$root/cgroup.subtree_control" ]]
grep -qw memory "$root/cgroup.controllers"
mapfile -t tasks < "$root/cgroup.procs"
[[ "${#tasks[@]}" -eq 1 && "${tasks[0]}" = "$$" ]]
mkdir "$root/controller"
printf '%s\n' "$$" > "$root/controller/cgroup.procs"
printf '+memory\n' > "$root/cgroup.subtree_control"
export SWARMCRAFT_HOST_CGROUP_ROOT="$root"
exec "$@"
