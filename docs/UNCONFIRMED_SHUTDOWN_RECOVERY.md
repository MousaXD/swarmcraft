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

If the process has definitely exited and the barrier was unconfirmed,
preserve the runtime directory and the previous canonical checkpoint
separately. The existing import workflow can import a **copy** of a
quiescent Minecraft save as a distinct private world after explicit
compatibility/identity review. Do not treat such data as signed history
of the original world. The original local hosting path remains blocked
until a deliberate, supported archival/recovery operation can release
the retained runtime. That operator archival workflow is still open;
manually deleting the pending marker is not a supported recovery action.
Other peers may continue their independent signed quorum recovery;
this local record cannot grant or override canonical authority.

The process-level regression deliberately withholds the real production
30-second Fabric acknowledgement, observes checkpointing and blocked
states, verifies the live subprocess and save bytes remain intact,
then permits that test process to exit without acknowledgement. It
verifies no new canonical checkpoint, sleep or local reset even after
renewed consent. The real-Minecraft and exact-head cross-platform gates
remain separate.
