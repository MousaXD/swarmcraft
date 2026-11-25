# Unconfirmed local save-and-stop recovery

Withdrawing local hosting consent immediately denies new game route proofs
and hosting eligibility. A durable preference write is not proof that a
Minecraft process has exited, saved or transferred authority.

Before requesting a Fabric shutdown barrier, the controller publishes and
syncs a world- and generation-bound local pending-checkpoint record. A
successful request requires the expected authenticated acknowledgement and a
successful child exit. The record is retired only after the final fenced
snapshot and durable signed sleep or prepared-transfer record are published.
A timeout, malformed acknowledgement, failed child exit, or interruption
does not certify a save. A subsequent clean exit alone cannot repair a
missing acknowledgement. An ambiguous marker prevents reset of the local
runtime even after a restart or renewed consent.

The Desktop contribution panel polls the local state while stopping or
blocked. "Stopping at checkpoint" is progress, not a shutdown authorization.
"Checkpoint not confirmed" is a denial-only diagnostic: leave a still-live
controller running, inspect its local logs, and do not force-stop the JVM or
remove the marker. If Fabric recovers and the same authenticated barrier
can finish, the normal checkpoint path can complete. A lost IPC session
cannot certify a late response from a different connection.

## Explicit local archival (Linux/macOS)

After the process has definitely exited, keep local hosting disabled
(0 MiB), and explicitly run:

```bash
swarmcraft world archive-unconfirmed-runtime <world-id> --confirm-unconfirmed-data
```

The CLI refuses without confirmation, a trusted pending marker, an
exclusive world runtime lock, verified process quiescence and an exclusive
Minecraft-compatible session.lock. It syncs the retained runtime's files
and directories, atomically moves the complete runtime into
`runtime-archives/<world>/<archive-id>/runtime` on the same filesystem,
and publishes a durable local receipt. The pending marker itself records
the exact archive transaction before the move, so restarting the command
after an interruption resumes the same archive rather than deleting or
duplicating it. A kernel no-replace rename is mandatory; symlinks,
multiply-linked or special files, conflicting directories and different
filesystems are denied. The journal is cleared only after the separate
archive is durable. As with other local control records, trusted
same-account filesystem ownership is assumed. The operation retires
pending local stop/wake/transfer requests and never changes signed
world history, creates a sleep/transfer proof, or enables hosting.

The output includes the retained Minecraft world directory. The existing
import workflow can import a **copy** of that quiescent save as a distinct
private world after explicit compatibility/identity review. Do not treat
such data as signed history of the original world. To resume the original
world, independently establish its latest verified canonical checkpoint,
current authority and explicit consent. Archival itself does not authorize
that launch. On Windows the archival command fails closed without moving
or deleting the retained files; a verified platform-specific durable
directory transaction is still required. Manually deleting the pending
marker is not a supported recovery action.

Other peers may continue their independent signed quorum recovery;
this local record cannot grant or override canonical authority.

The process-level regression deliberately withholds the real production
30-second Fabric acknowledgement, observes checkpointing and blocked
states, verifies the live subprocess and save bytes remain intact,
then permits that test process to exit without acknowledgement. It
verifies no new canonical checkpoint, sleep or local reset even after
renewed consent. The real-Minecraft and exact-head cross-platform gates
remain separate.
