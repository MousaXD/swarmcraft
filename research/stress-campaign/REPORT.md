# #53 bounded local stress and recoverability campaign

Date: 2026-11-25. Scope: Linux, isolated disposable profiles/filesystems,
native process and kernel tests. No production data, hosted Actions,
host network qdisc, or host-wide cgroup changes. This is not a production
release attestation, real multi-machine certification, or a claim that a
physical power failure was tested.

## Results matrix

| Scenario | Repetitions and actual limits | Observed failure / verification | Outcome |
| --- | --- | --- | --- |
| Full filesystem, atomic metadata update | 3 independent worlds × 3 denied writes; 96 MiB tmpfs, 64 MiB reserve, 0 bytes free | Kernel errno 28 (ENOSPC); prior policy unchanged; payload admission refused | 9/9 denied; no new atomic scratch; old signed save restored even while full |
| Process file-size cap | 3; RLIMIT_FSIZE soft/hard 1 MiB; SIGXFSZ ignored only in child to expose errno | Actual EFBIG (errno 27), not a simulated world quota | 3/3 denied; no handled-failure blob scratch; old checkpoint restored |
| Interrupted snapshot capture | 3; 2 MiB complete uncommitted blob plus 24 MiB next input | SIGSTOP after observing an actual partial, then SIGKILL; 128–768 KiB of unfinished compression remained | 3/3 old signed checkpoint survived and was restored |
| Retention and retry | 3; same limited filesystems, retain every committed snapshot | Reclaim exactly one unreferenced complete blob per run (2,097,209 bytes each); preserve unproven crash scratch | 3/3 new snapshot #2 extends #1; both restored byte-for-byte, no signed manifest removed |
| Impaired authenticated replica resume A | 1; kernel netem 15 ± 3 ms configured delay, 0.25% configured loss, 30 Mbit/s, 1000-packet queue; application cap 512 KiB/s | 7,570 transmitted packets, 4 kernel drops; SIGKILL receiver and original source after 131,072-byte durable partial | All three signed snapshots preserved; receiver resumed from surviving authorized seeder; 11.755 s from restart to verified new head |
| Impaired authenticated replica resume B | 1; 30 ± 5 ms configured delay, 0.75% configured loss, 15 Mbit/s, 1000-packet queue; application cap 512 KiB/s | 7,782 transmitted packets, 21 kernel drops; same deliberate hard losses | All three signed snapshots preserved; durable resume; 21.227 s from restart to verified new head |
| Delegated cgroup-v2 hard memory cap | 2 independent transient scopes; actual 64 MiB memory.max; zero swap, group OOM | Combined native parent/descendant charges observed at 30,932,992 and 30,695,424 bytes; 128 MiB pressure allocation in leaf | 2/2 kernel OOM group kill and empty leaf verified |
| Residual runtime descendant | 1; real 64 MiB cap | Parent exited, surviving descendant held recorded ownership | Same-root reset and account-wide readmission denied until exact group empty |
| Host launcher under delegated cap | 1; 512 MiB configured heap, actual 1,536 MiB kernel limit | Actual launcher attached mock Java before exec; verified saved marker and signed sleep/checkpoint boundary | Initial and final signatures verified, checkpoint #2 restored |

The storage campaign uses the quiescent 31-file Java Edition 26.1.2 save
from [#52](../profile-upgrade/REPORT.md), imports it through the actual
current sidecar into fresh independent peer/world identities, and checks
every original file's SHA-256 against each recovery. The retry adds a
deterministic synthetic file; both old and new inventories must match.
Only each disposable tmpfs is filled. The cap is a real kernel per-file
write limit, not a bound on the total world or JVM memory. The complete
immutable signed checkpoint hashes and per-run failure codes are in
[storage-linux.json](evidence/storage-linux.json).

The new ignored signed-replica stress test builds a valid signed joint
membership with three distinct disposable devices, not three hosts
with a fabricated lease. It uses real authenticated daemon processes,
QUIC, production storage/pull code and an independent surviving replica.
The first nonempty checkpoint (#2) is actually replicated and restored
before the 4 MiB encoded successor (#3) is introduced. It kills the
receiver and original source only after an observed durable partial.
It reopens the receiver, verifies/restores #2, resumes #3 from the
surviving seeder, verifies every retained signature/blob and restores #3.
No device has hosting consent or runtime configuration. All processes
exit, and the epoch/membership remain unchanged. The two impairment
profiles are separate runs, not statistical samples of a production WAN.
Configured loss is not the observed loss fraction; raw qdisc counters
and exact checkpoint hashes are in
[network-linux.json](evidence/network-linux.json). This transfers real
synthetic signed payloads, not live gameplay or the multi-GiB release soak.

The existing cgroup tests were explicitly executed, not reported as
successful when ignored. The hard-cap fixture first verifies production
high-throttling settings, then disables high throttling in its own leaf
only for the deliberate OOM experiment. It creates no Minecraft
checkpoint. The separate host process test records exact previous/final
checkpoint hashes and verified pre-exec membership, under the existing
production 512 MiB heap admission. See
[memory-linux.json](evidence/memory-linux.json). This is a real kernel
limit and mock-Java runtime, not installed Minecraft OOM recovery.

## Defect found and corrected

The original durable atomic replacement left a unique staging file
behind after a handled write/fsync/rename error. An isolated, truly
full tmpfs probe produced three ENOSPC errors and three abandoned
.atomic temporary files. The existing canonical destination was
unchanged. The fix closes and removes only the current writer's unique
temporary file on a handled write, sync or publication failure. It
returns the original I/O error and preserves existing canonical records,
unproven crash evidence, commit intent and all fencing checks. Windows
file handle closure precedes cleanup. A unit regression uses an
unreplaceable directory target, preserves its contents and a preexisting
crash artifact, and repeats failure three times. The real full-disk
campaign then confirmed zero new atomic scratch across nine ENOSPC
metadata failures.

The initial tmpfs probe also demonstrated the default 256 MiB reserve
correctly rejects a new world on a 96 MiB mount; the final lab prepares
the isolated seed on ordinary disk before copying into the dedicated
limited mount with the explicit valid 64 MiB policy. A first network
run stopped with no packets because the replica-only startup rejected
unmapped-root ancestor ownership in the user namespace. The retained
[blocked run](evidence/network-initial-blocked.json) is not counted
as a pass. The final lab uses ordinary daemon startup with no consent
or runtime configuration, verifies absent consent and actual signed
replication, and does not weaken the ownership guard.

## Reproduce locally

Build the actual sidecars and test binaries with locked dependencies,
using the shared Cargo lock when other worktrees are active. Use a real
quiescent disposable Java save, not an active save or user profile.
The lab scripts refuse unsafe scope and use fresh artifact directories.
Keep the source, generated private artifacts, keys and raw logs local;
the committed evidence is just bounded test receipts.

```sh
unshare --user --map-root-user --mount --propagation private \
  python3 scripts/acceptance/storage_stress.py \
  --bin target/debug --save /absolute/disposable/java-save \
  --artifacts /absolute/new/storage-run --repetitions 3

# Build with cargo test --locked -p swarm-cli --test parallel_replication --no-run
# then use its reported test executable (not a guessed or stale binary).
unshare --user --map-root-user --net \
  python3 scripts/acceptance/network_stress.py \
  --test /absolute/parallel_replication_test_binary \
  --artifacts /absolute/new/network-run
```

Use the documented [delegated cgroup gate](../../docs/HOST_MEMORY_ENFORCEMENT.md)
for the two kernel OOM runs, residual descendant test, and actual mock
host launcher test. Each invocation uses its own transient scope, with
a bounded runtime and the explicitly enabled delegated memory controller.
Do not alter the daily workstation cgroup or network interface.

## Remaining qualification

The tests do not prove crash behavior at every commit fsync/rename,
power-loss durability, persistent-filesystem writeback exhaustion,
multi-GiB release soak, independent disks, WAN/NAT, relay failure in
the field, native Windows/macOS, or full live Fabric/Minecraft OOM
recovery. Unproven hard-crash scratch is intentionally retained.
The local-first policy still requires exact-candidate final validation;
no hosted run or release was dispatched for this campaign.
