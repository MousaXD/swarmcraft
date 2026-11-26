//! Local runtime intent and isolated Java/Fabric qualification. No playable
//! runtime, canonical authority, or player route is reachable here.
//! Ordinary reference operations refuse an unresolved durable intent even
//! after the controller exits or its journal lock is released.
use super::{Pin, Reviewed, Store};
use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;
use swarm_core::DataPaths;
use swarm_protocol::{ExclusiveTurnWorldIdV1, Hash32};

// A local journal envelope version, not a wire format or genesis policy.
// Older binaries only accept local journal format 1 and therefore cannot
// bypass the sidecar once the user has opted in.
pub(super) const FENCED_JOURNAL_FORMAT: u16 = 2;

#[derive(Debug, Subcommand)]
pub enum RuntimeCommand {
    /// Review local consent, revision and any unresolved launch intent.
    Status {
        #[command(flatten)]
        pin: Pin,
    },
    /// Opt in to one exact local reference grant, save and runtime package.
    Consent {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        heap_mib: u64,
        #[arg(long)]
        cpu_millicores: u64,
    },
    /// Durably fence all other reference operations after fresh package preflight. Does not spawn.
    Reserve {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        package: PathBuf,
    },
    /// Qualify the reviewed Java executable under mandatory kernel limits; never opens a world.
    ProbeJava {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expected_intent: Hash32,
        #[arg(long)]
        package: PathBuf,
    },
    /// Retire an interrupted isolated Java probe only with exact kernel quiescence or verified reboot.
    RecoverJavaProbe {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expected_intent: Hash32,
    },
    /// Run a bounded, network-isolated Fabric save/shutdown qualification; no players or hosting permit.
    QualifyFabric {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expected_intent: Hash32,
        #[arg(long)]
        package: PathBuf,
        /// Explicit acceptance of the Minecraft EULA for this isolated execution.
        #[arg(long)]
        accept_minecraft_eula: bool,
    },
    /// Resume only an already signed, quiescent and fully retained checkpoint transaction.
    ResumeFabricCheckpoint {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_target: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expected_intent: Hash32,
    },
    /// Retire an interrupted Fabric qualification without publishing unconfirmed writes.
    DiscardInterruptedFabric {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expected_intent: Hash32,
        /// Acknowledge that only the old signed checkpoint remains authoritative in the reference journal.
        #[arg(long)]
        acknowledge_unconfirmed_save: bool,
    },
    /// Withdraw exact local consent without taking the world journal lock. An intent stays fenced.
    Revoke {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_revision: u64,
    },
    /// Explicitly retire an exact never-launched v1 intent; consume its consent.
    Clear {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expected_intent: Hash32,
    },
}

#[cfg(target_os = "linux")]
mod linux {
    use super::super::{
        check_dir, dir, identity, pin_grant, print_document, private_file, publish, read_bounded, sync_dir, Journal,
        Phase, VerifiedExclusiveTurnHead,
    };
    use super::*;
    use anyhow::{ensure, Context};
    use ed25519_dalek::{Signature, VerifyingKey};
    use fs2::FileExt;
    use serde::{Deserialize, Serialize};
    use std::{
        fs::{self, File, OpenOptions},
        path::Path,
    };
    use swarm_cli::{
        exclusive_turn_process_probe::{
            fabric::{FabricWorkspace, FabricWorkspacePin},
            ProbeAdmission, ProbeRecovery,
        },
        exclusive_turn_runtime_package::StagedPackagePin,
        runtime_process_guard::ProbeRecoveryPin,
    };
    use swarm_core::{random_nonce, PeerIdentity};
    use swarm_protocol::{ExclusiveTurnCheckpointV1, PeerId};

    const LOCK: &str = "runtime-local.lock";
    const STATE: &str = "runtime-local.postcard";
    const LIMIT: usize = 16 * 1024;
    const DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/local-runtime-intent/v1\0";
    const INTENT_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/unlaunched-runtime-intent/v1\0";
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct Scope {
        world: ExclusiveTurnWorldIdV1,
        holder: PeerId,
        grant: Hash32,
        epoch: u64,
        fence: u64,
        checkpoint: ExclusiveTurnCheckpointV1,
        compatibility: Hash32,
    }

    impl Scope {
        fn held(journal: &Journal, head: &VerifiedExclusiveTurnHead, expected: Hash32) -> Result<Self> {
            ensure!(
                head.save_backed() && journal.phase == Phase::Held && head.holder() == journal.identity,
                "runtime intent requires exact held save-backed reference turn"
            );
            ensure!(journal.checkpoint.reference_commitment == expected, "local checkpoint differs from expected pin");
            Ok(Self {
                world: head.world(),
                holder: journal.identity,
                grant: head.grant(),
                epoch: head.epoch(),
                fence: head.fence(),
                checkpoint: journal.checkpoint,
                compatibility: head.compatibility(),
            })
        }

        fn validate(&self, world: ExclusiveTurnWorldIdV1, peer: PeerId) -> Result<()> {
            ensure!(
                self.world == world
                    && self.holder == peer
                    && self.grant != Hash32::default()
                    && self.compatibility != Hash32::default()
                    && self.epoch > 0
                    && self.fence > 0,
                "runtime intent scope or identity differs"
            );
            self.checkpoint.validate()?;
            Ok(())
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct Consent {
        scope: Scope,
        revision: u64,
        heap_mib: u64,
        cpu_millicores: u64,
    }

    impl Consent {
        fn validate(&self, world: ExclusiveTurnWorldIdV1, peer: PeerId) -> Result<()> {
            self.scope.validate(world, peer)?;
            ensure!(self.revision > 0, "invalid runtime consent revision");
            validate_budget(self.heap_mib, self.cpu_millicores)
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    enum IntentKind {
        // Do not reuse this variant for a process which could have been spawned.
        UnlaunchedNoProcessV1,
        // Distinct persistent variant: never accepted by the unlaunched
        // clear path, including by an older decoder.
        JavaProbeMayHaveSpawnedV1,
        JavaProbeMayHaveSpawnedV2(ProbeRecoveryPin),
        JavaProbeMayHaveSpawnedV3 {
            recovery: ProbeRecoveryPin,
            stage: StagedPackagePin,
        },
        // Append-only discriminants: older decoders must fail closed.
        FabricMayHaveSpawnedV1 {
            recovery: ProbeRecoveryPin,
            stage: StagedPackagePin,
            workspace: FabricWorkspacePin,
        },
        FabricCheckpointPendingV1 {
            stage: StagedPackagePin,
            workspace: FabricWorkspacePin,
            target: ExclusiveTurnCheckpointV1,
            evidence: Hash32,
        },
    }

    impl IntentKind {
        fn name(&self) -> &'static str {
            match self {
                Self::UnlaunchedNoProcessV1 => "unlaunched_no_process_v1",
                Self::JavaProbeMayHaveSpawnedV1 => "java_probe_may_have_spawned_v1",
                Self::JavaProbeMayHaveSpawnedV2(_) => "java_probe_may_have_spawned_v2",
                Self::JavaProbeMayHaveSpawnedV3 { .. } => "java_probe_may_have_spawned_v3",
                Self::FabricMayHaveSpawnedV1 { .. } => "fabric_may_have_spawned_v1",
                Self::FabricCheckpointPendingV1 { .. } => "fabric_checkpoint_pending_v1",
            }
        }

        fn is_probe(&self) -> bool {
            matches!(
                self,
                Self::JavaProbeMayHaveSpawnedV1
                    | Self::JavaProbeMayHaveSpawnedV2(_)
                    | Self::JavaProbeMayHaveSpawnedV3 { .. }
            )
        }

        fn may_have_spawned(&self) -> bool {
            self.is_probe() || matches!(self, Self::FabricMayHaveSpawnedV1 { .. })
        }

        fn recovery(&self) -> Option<&ProbeRecoveryPin> {
            match self {
                Self::JavaProbeMayHaveSpawnedV2(pin) | Self::JavaProbeMayHaveSpawnedV3 { recovery: pin, .. } => {
                    Some(pin)
                }
                _ => None,
            }
        }

        fn stage(&self) -> Option<&StagedPackagePin> {
            match self {
                Self::JavaProbeMayHaveSpawnedV3 { stage, .. }
                | Self::FabricMayHaveSpawnedV1 { stage, .. }
                | Self::FabricCheckpointPendingV1 { stage, .. } => Some(stage),
                _ => None,
            }
        }

        fn workspace(&self) -> Option<&FabricWorkspacePin> {
            match self {
                Self::FabricMayHaveSpawnedV1 { workspace, .. } | Self::FabricCheckpointPendingV1 { workspace, .. } => {
                    Some(workspace)
                }
                _ => None,
            }
        }

        fn validate(&self, consent: &Consent) -> Result<()> {
            if let Some(pin) = self.recovery() {
                pin.verify_budget(consent.heap_mib, consent.cpu_millicores, consent.revision)?;
            }
            if let Self::FabricMayHaveSpawnedV1 { recovery, .. } = self {
                recovery.verify_budget(consent.heap_mib, consent.cpu_millicores, consent.revision)?;
            }
            if let Some(stage) = self.stage() {
                stage.verify_compatibility(consent.scope.compatibility)?;
            }
            if let Some(workspace) = self.workspace() {
                workspace.validate()?;
            }
            if let Self::FabricCheckpointPendingV1 { target, evidence, .. } = self {
                target.validate()?;
                ensure!(*evidence != Hash32::default(), "missing isolated Fabric lifecycle evidence digest");
                ensure!(
                    target.reference_commitment != consent.scope.checkpoint.reference_commitment
                        || *target == consent.scope.checkpoint,
                    "checkpoint claims differ for the same retained bytes"
                );
            }
            Ok(())
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct Intent {
        kind: IntentKind,
        nonce: [u8; 32],
        consent: Consent,
    }

    impl Intent {
        fn id(&self) -> Result<Hash32> {
            Ok(Hash32::from_domain_bytes(INTENT_DOMAIN, &postcard::to_allocvec(self)?))
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct State {
        version: u16,
        world: ExclusiveTurnWorldIdV1,
        peer: PeerId,
        revision: u64,
        consent: Option<Consent>,
        intent: Option<Intent>,
    }

    impl State {
        fn empty(world: ExclusiveTurnWorldIdV1, peer: PeerId) -> Self {
            Self { version: 1, world, peer, revision: 0, consent: None, intent: None }
        }

        fn validate(&self, world: ExclusiveTurnWorldIdV1, peer: PeerId) -> Result<()> {
            ensure!(
                self.version == 1 && self.world == world && self.peer == peer && self.revision > 0,
                "invalid local runtime intent state version, identity or revision"
            );
            if let Some(consent) = &self.consent {
                consent.validate(world, peer)?;
                ensure!(consent.revision <= self.revision, "runtime consent revision exceeds local state");
            }
            if let Some(intent) = &self.intent {
                ensure!(intent.nonce != [0; 32], "invalid runtime intent nonce");
                intent.consent.validate(world, peer)?;
                intent.kind.validate(&intent.consent)?;
                ensure!(intent.consent.revision < self.revision, "runtime intent precedes its consent");
                ensure!(
                    self.consent.as_ref().is_none_or(|consent| consent == &intent.consent),
                    "runtime intent and consent disagree"
                );
                if matches!(intent.kind, IntentKind::FabricCheckpointPendingV1 { .. }) {
                    ensure!(self.consent.is_none(), "pending Fabric checkpoint must consume runtime consent");
                }
            } else if let Some(consent) = &self.consent {
                ensure!(consent.revision == self.revision, "unreserved consent revision differs from local state");
            }
            Ok(())
        }

        fn next(&mut self, expected: u64) -> Result<()> {
            ensure!(self.revision == expected, "runtime local revision differs from expected pin");
            self.revision = self.revision.checked_add(1).context("runtime local revision exhausted")?;
            Ok(())
        }

        fn opt_in(&mut self, scope: Scope, expected: u64, heap_mib: u64, cpu_millicores: u64) -> Result<()> {
            ensure!(self.intent.is_none(), "unresolved runtime intent must be cleared first");
            validate_budget(heap_mib, cpu_millicores)?;
            self.next(expected)?;
            self.consent = Some(Consent { scope, revision: self.revision, heap_mib, cpu_millicores });
            Ok(())
        }

        fn exact_consent(&self, scope: &Scope, expected: u64) -> Result<&Consent> {
            ensure!(self.intent.is_none(), "unresolved runtime intent must be cleared first");
            ensure!(self.revision == expected, "runtime local revision differs from expected pin");
            let consent = self.consent.as_ref().context("explicit local runtime consent is absent or withdrawn")?;
            ensure!(
                &consent.scope == scope && consent.revision == self.revision,
                "runtime consent does not match exact held grant, checkpoint and package"
            );
            Ok(consent)
        }

        fn reserve(&mut self, scope: &Scope, expected: u64) -> Result<()> {
            let consent = self.exact_consent(scope, expected)?.clone();
            self.next(expected)?;
            self.intent = Some(Intent { kind: IntentKind::UnlaunchedNoProcessV1, nonce: random_nonce(), consent });
            Ok(())
        }

        fn unlaunched_consent(&self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<&Consent> {
            ensure!(self.revision == expected, "runtime local revision differs from expected pin");
            let intent = self.intent.as_ref().context("no unresolved runtime intent")?;
            ensure!(
                intent.kind == IntentKind::UnlaunchedNoProcessV1
                    && intent.id()? == expected_intent
                    && &intent.consent.scope == scope,
                "runtime intent differs from exact unlaunched pin"
            );
            ensure!(self.consent.as_ref() == Some(&intent.consent), "runtime consent was withdrawn");
            Ok(&intent.consent)
        }

        fn begin_probe(
            &mut self,
            scope: &Scope,
            expected: u64,
            expected_intent: Hash32,
            kind: IntentKind,
        ) -> Result<Hash32> {
            let consent = self.unlaunched_consent(scope, expected, expected_intent)?;
            ensure!(kind.is_probe(), "cannot begin a non-probe intent");
            kind.validate(consent)?;
            self.next(expected)?;
            let intent = self.intent.as_mut().context("runtime intent disappeared")?;
            intent.kind = kind;
            intent.id()
        }

        fn begin_fabric(
            &mut self,
            scope: &Scope,
            expected: u64,
            expected_intent: Hash32,
            kind: IntentKind,
        ) -> Result<Hash32> {
            let consent = self.unlaunched_consent(scope, expected, expected_intent)?;
            ensure!(matches!(kind, IntentKind::FabricMayHaveSpawnedV1 { .. }), "invalid Fabric process intent kind");
            kind.validate(consent)?;
            self.next(expected)?;
            let intent = self.intent.as_mut().context("runtime intent disappeared")?;
            intent.kind = kind;
            intent.id()
        }

        fn exact_intent(&self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<&Intent> {
            ensure!(self.revision == expected, "runtime local revision differs from expected pin");
            let intent = self.intent.as_ref().context("no unresolved runtime intent")?;
            ensure!(
                intent.id()? == expected_intent && &intent.consent.scope == scope,
                "runtime intent differs from exact pin"
            );
            Ok(intent)
        }

        fn check_fabric(&self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<()> {
            let intent = self.exact_intent(scope, expected, expected_intent)?;
            ensure!(
                matches!(intent.kind, IntentKind::FabricMayHaveSpawnedV1 { .. })
                    && self.consent.as_ref() == Some(&intent.consent),
                "isolated Fabric lost exact local consent or process intent"
            );
            Ok(())
        }

        // Only the in-process supervisor may call after actual authenticated
        // shutdown, exact empty cgroup retirement and durable capture.
        fn prepare_fabric_checkpoint(
            &mut self,
            scope: &Scope,
            expected: u64,
            expected_intent: Hash32,
            target: ExclusiveTurnCheckpointV1,
            evidence: Hash32,
        ) -> Result<Hash32> {
            self.check_fabric(scope, expected, expected_intent)?;
            let intent = self.intent.as_ref().context("Fabric process intent disappeared")?;
            let IntentKind::FabricMayHaveSpawnedV1 { stage, workspace, .. } = &intent.kind else { unreachable!() };
            let kind = IntentKind::FabricCheckpointPendingV1 {
                stage: stage.clone(),
                workspace: workspace.clone(),
                target,
                evidence,
            };
            kind.validate(&intent.consent)?;
            self.next(expected)?;
            self.consent = None;
            let intent = self.intent.as_mut().context("Fabric process intent disappeared")?;
            intent.kind = kind;
            intent.id()
        }

        fn pending_checkpoint(
            &self,
            current: &Scope,
            expected_base: Hash32,
            expected_target: Hash32,
            expected: u64,
            expected_intent: Hash32,
        ) -> Result<(Scope, ExclusiveTurnCheckpointV1, Hash32)> {
            ensure!(self.revision == expected, "runtime local revision differs from expected pin");
            let intent = self.intent.as_ref().context("no unresolved runtime intent")?;
            let IntentKind::FabricCheckpointPendingV1 { target, evidence, .. } = &intent.kind else {
                anyhow::bail!("no signed quiescent Fabric checkpoint transaction")
            };
            let base = &intent.consent.scope;
            let mut current_generation = current.clone();
            current_generation.checkpoint = base.checkpoint;
            ensure!(
                &current_generation == base
                    && intent.id()? == expected_intent
                    && base.checkpoint.reference_commitment == expected_base
                    && target.reference_commitment == expected_target
                    && (current.checkpoint == base.checkpoint || current.checkpoint == *target)
                    && self.consent.is_none(),
                "pending Fabric checkpoint differs from exact base, target, grant or revision"
            );
            Ok((base.clone(), *target, *evidence))
        }

        fn finish_pending_checkpoint(
            &mut self,
            current: &Scope,
            expected_base: Hash32,
            expected_target: Hash32,
            expected: u64,
            expected_intent: Hash32,
        ) -> Result<()> {
            let (_, target, _) =
                self.pending_checkpoint(current, expected_base, expected_target, expected, expected_intent)?;
            ensure!(current.checkpoint == target, "pending Fabric checkpoint has not been published");
            self.next(expected)?;
            self.intent = None;
            Ok(())
        }

        fn fabric_recovery_pin(
            &self,
            scope: &Scope,
            expected: u64,
            expected_intent: Hash32,
        ) -> Result<&ProbeRecoveryPin> {
            let intent = self.exact_intent(scope, expected, expected_intent)?;
            match &intent.kind {
                IntentKind::FabricMayHaveSpawnedV1 { recovery, .. } => Ok(recovery),
                _ => anyhow::bail!("only an interrupted Fabric process intent may be discarded"),
            }
        }

        fn finish_fabric_discard(&mut self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<()> {
            self.fabric_recovery_pin(scope, expected, expected_intent)?;
            self.next(expected)?;
            self.intent = None;
            self.consent = None;
            Ok(())
        }

        fn check_probe(&self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<()> {
            ensure!(self.revision == expected, "runtime consent or local revision changed during Java qualification");
            let intent = self.intent.as_ref().context("Java probe intent disappeared")?;
            ensure!(
                intent.kind.is_probe()
                    && intent.id()? == expected_intent
                    && &intent.consent.scope == scope
                    && self.consent.as_ref() == Some(&intent.consent),
                "Java qualification lost exact local consent or intent"
            );
            Ok(())
        }

        // Only the successful in-process completion path calls this. The
        // public clear command never retires a possibly-spawned intent.
        fn finish_probe(&mut self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<()> {
            self.check_probe(scope, expected, expected_intent)?;
            self.next(expected)?;
            self.intent = None;
            self.consent = None;
            Ok(())
        }

        fn recovery_pin(&self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<&ProbeRecoveryPin> {
            ensure!(self.revision == expected, "runtime local revision differs from expected pin");
            let intent = self.intent.as_ref().context("no unresolved runtime intent")?;
            ensure!(
                &intent.consent.scope == scope && intent.id()? == expected_intent,
                "runtime intent differs from exact recovery pin"
            );
            match &intent.kind {
                IntentKind::JavaProbeMayHaveSpawnedV2(pin)
                | IntentKind::JavaProbeMayHaveSpawnedV3 { recovery: pin, .. } => Ok(pin),
                IntentKind::JavaProbeMayHaveSpawnedV1 => {
                    anyhow::bail!("legacy possibly-spawned probe lacks a signed kernel recovery pin; recovery refused")
                }
                IntentKind::UnlaunchedNoProcessV1 => anyhow::bail!("unlaunched intent is not a recoverable Java probe"),
                _ => anyhow::bail!("Fabric intent is not a recoverable Java probe"),
            }
        }

        fn finish_recovery(&mut self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<()> {
            self.recovery_pin(scope, expected, expected_intent)?;
            self.next(expected)?;
            self.intent = None;
            self.consent = None;
            Ok(())
        }

        fn revoke(&mut self, grant: Hash32, expected: u64) -> Result<()> {
            let consent = self.consent.as_ref().context("explicit local runtime consent is absent or withdrawn")?;
            ensure!(consent.scope.grant == grant, "runtime consent grant differs from expected pin");
            self.next(expected)?;
            self.consent = None;
            Ok(())
        }

        fn clear(&mut self, scope: &Scope, expected: u64, expected_intent: Hash32) -> Result<()> {
            let intent = self.intent.as_ref().context("no unresolved runtime intent")?;
            ensure!(
                intent.kind == IntentKind::UnlaunchedNoProcessV1
                    && &intent.consent.scope == scope
                    && intent.id()? == expected_intent,
                "runtime intent differs from exact unlaunched pin"
            );
            self.next(expected)?;
            self.intent = None;
            self.consent = None; // One-shot consent; never re-use after an intent.
            Ok(())
        }
    }

    fn validate_budget(heap_mib: u64, cpu_millicores: u64) -> Result<()> {
        ensure!(
            (swarm_cli::hosting_budget::MIN_HEAP_MIB..=swarm_cli::hosting_budget::MAX_HEAP_MIB).contains(&heap_mib),
            "runtime heap consent out of bounds"
        );
        swarm_cli::host_memory_cap::validate_cpu_request(cpu_millicores)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn possibly_spawned_probe_cannot_use_unlaunched_clear_or_stale_consent() {
            let world: ExclusiveTurnWorldIdV1 = format!("scpass:{}", Hash32([1; 32])).parse().unwrap();
            let peer = PeerIdentity::generate().peer_id();
            let scope = Scope {
                world,
                holder: peer,
                grant: Hash32([2; 32]),
                epoch: 1,
                fence: 1,
                checkpoint: ExclusiveTurnCheckpointV1 {
                    manifest_hash: Hash32([3; 32]),
                    state_root: Hash32([4; 32]),
                    reference_commitment: Hash32([5; 32]),
                },
                compatibility: Hash32([6; 32]),
            };
            let mut state = State::empty(world, peer);
            state.opt_in(scope.clone(), 0, 512, 500).unwrap();
            state.reserve(&scope, 1).unwrap();
            let unlaunched = state.intent.as_ref().unwrap().id().unwrap();
            assert!(state.begin_probe(&scope, 1, unlaunched, IntentKind::JavaProbeMayHaveSpawnedV1).is_err());
            let probe = state.begin_probe(&scope, 2, unlaunched, IntentKind::JavaProbeMayHaveSpawnedV1).unwrap();
            assert_ne!(unlaunched, probe);
            state.validate(world, peer).unwrap();
            assert!(state.clear(&scope, 3, probe).is_err());
            assert!(state.clear(&scope, 3, unlaunched).is_err());
            assert!(state.begin_probe(&scope, 3, probe, IntentKind::JavaProbeMayHaveSpawnedV1).is_err());
            assert!(state.recovery_pin(&scope, 3, probe).is_err());
            state.check_probe(&scope, 3, probe).unwrap();
            let mut withdrawn = state.clone();
            withdrawn.revoke(scope.grant, 3).unwrap();
            assert!(withdrawn.check_probe(&scope, 3, probe).is_err());
            assert!(withdrawn.finish_probe(&scope, 4, probe).is_err());
            assert!(withdrawn.clear(&scope, 4, probe).is_err());
            assert!(withdrawn.intent.is_some());
            state.finish_probe(&scope, 3, probe).unwrap();
            assert_eq!(state.revision, 4);
            assert!(state.intent.is_none() && state.consent.is_none());
        }

        #[test]
        fn recovery_v2_is_exact_and_consumes_revoked_consent_after_kernel_proof() {
            // Pure state-machine fixture. The separate delegated Linux test
            // exercises the actual process/cgroup proof, not these values.
            let recovery: ProbeRecoveryPin = serde_json::from_value(serde_json::json!({
                "controller_pid": 123, "cgroup_namespace": 4, "mount_namespace": 5,
                "linux": {"machine_id": "ab".repeat(16), "boot_id": "12345678-1234-1234-1234-123456789abc",
                    "pid_namespace": 6, "controller_start_ticks": 7, "java_start_ticks": null,
                    "cgroup": {"path": "/sys/fs/cgroup/example/swarmcraft-jvm-fixture", "device": 8, "inode": 9,
                        "limit_bytes": 1536u64 * 1024 * 1024, "high_bytes": 1449551462u64,
                        "cpu": {"millicores": 500, "admitted_heap_mib": 512, "consent_revision": 1}}}
            }))
            .unwrap();
            let identity = PeerIdentity::generate();
            let world: ExclusiveTurnWorldIdV1 = format!("scpass:{}", Hash32([1; 32])).parse().unwrap();
            let peer = identity.peer_id();
            let scope = Scope {
                world,
                holder: peer,
                grant: Hash32([2; 32]),
                epoch: 1,
                fence: 1,
                checkpoint: ExclusiveTurnCheckpointV1 {
                    manifest_hash: Hash32([3; 32]),
                    state_root: Hash32([4; 32]),
                    reference_commitment: Hash32([5; 32]),
                },
                compatibility: Hash32([6; 32]),
            };
            let mut state = State::empty(world, peer);
            state.opt_in(scope.clone(), 0, 512, 500).unwrap();
            state.reserve(&scope, 1).unwrap();
            let unlaunched = state.intent.as_ref().unwrap().id().unwrap();
            assert!(state.recovery_pin(&scope, 2, unlaunched).is_err());
            let mut staged_state = state.clone();
            let stage: StagedPackagePin = serde_json::from_value(serde_json::json!({
                "name": format!("runtime-stage-{}", "ab".repeat(32)), "device": 1, "inode": 2,
                "compatibility": scope.compatibility
            }))
            .unwrap();
            let wrong_stage: StagedPackagePin = serde_json::from_value(serde_json::json!({
                "name": format!("runtime-stage-{}", "ab".repeat(32)), "device": 1, "inode": 2,
                "compatibility": Hash32([9;32])
            }))
            .unwrap();
            assert!(staged_state
                .begin_probe(
                    &scope,
                    2,
                    unlaunched,
                    IntentKind::JavaProbeMayHaveSpawnedV3 { recovery: recovery.clone(), stage: wrong_stage }
                )
                .is_err());
            let staged_probe = staged_state
                .begin_probe(
                    &scope,
                    2,
                    unlaunched,
                    IntentKind::JavaProbeMayHaveSpawnedV3 { recovery: recovery.clone(), stage: stage.clone() },
                )
                .unwrap();
            staged_state.validate(world, peer).unwrap();
            let staged_signed = SignedState {
                signature: identity.sign(&signing_bytes(&staged_state).unwrap()),
                state: staged_state.clone(),
            };
            let staged_bytes = postcard::to_allocvec(&staged_signed).unwrap();
            let staged_decoded: SignedState = postcard::from_bytes(&staged_bytes).unwrap();
            assert_eq!(postcard::to_allocvec(&staged_decoded).unwrap(), staged_bytes);
            assert_eq!(staged_decoded.state.intent.as_ref().unwrap().kind.stage(), Some(&stage));
            assert_eq!(staged_decoded.state.recovery_pin(&scope, 3, staged_probe).unwrap(), &recovery);
            assert!(staged_state.clear(&scope, 3, staged_probe).is_err());
            staged_state.revoke(scope.grant, 3).unwrap();
            staged_state.finish_recovery(&scope, 4, staged_probe).unwrap();
            assert!(staged_state.intent.is_none() && staged_state.consent.is_none());
            let probe = state
                .begin_probe(&scope, 2, unlaunched, IntentKind::JavaProbeMayHaveSpawnedV2(recovery.clone()))
                .unwrap();
            assert_ne!(probe, staged_probe);
            state.validate(world, peer).unwrap();
            let signed =
                SignedState { signature: identity.sign(&signing_bytes(&state).unwrap()), state: state.clone() };
            let bytes = postcard::to_allocvec(&signed).unwrap();
            let decoded: SignedState = postcard::from_bytes(&bytes).unwrap();
            assert_eq!(postcard::to_allocvec(&decoded).unwrap(), bytes);
            assert_eq!(decoded.state.recovery_pin(&scope, 3, probe).unwrap(), &recovery);
            assert!(state.clear(&scope, 3, probe).is_err());
            state.revoke(scope.grant, 3).unwrap();
            assert!(state.recovery_pin(&scope, 3, probe).is_err());
            assert!(state.recovery_pin(&scope, 4, unlaunched).is_err());
            assert!(state.finish_probe(&scope, 4, probe).is_err());
            assert_eq!(state.recovery_pin(&scope, 4, probe).unwrap(), &recovery);
            state.finish_recovery(&scope, 4, probe).unwrap();
            assert_eq!(state.revision, 5);
            assert!(state.intent.is_none() && state.consent.is_none());
            assert!(state.finish_recovery(&scope, 5, probe).is_err());
        }

        fn fixture_fabric_kind(scope: &Scope) -> IntentKind {
            // Pure signed-state fixture: no claim of actual kernel proof.
            let recovery:ProbeRecoveryPin=serde_json::from_value(serde_json::json!({
                "controller_pid":123,"cgroup_namespace":4,"mount_namespace":5,
                "linux":{"machine_id":"ab".repeat(16),"boot_id":"12345678-1234-1234-1234-123456789abc","pid_namespace":6,"controller_start_ticks":7,"java_start_ticks":null,
                    "cgroup":{"path":"/sys/fs/cgroup/example/swarmcraft-jvm-fixture","device":8,"inode":9,"limit_bytes":1536u64*1024*1024,"high_bytes":1449551462u64,"cpu":{"millicores":500,"admitted_heap_mib":512,"consent_revision":1}}}
            })).unwrap();
            let stage=serde_json::from_value(serde_json::json!({"name":format!("runtime-stage-{}","ab".repeat(32)),"device":1,"inode":2,"compatibility":scope.compatibility})).unwrap();
            let workspace=serde_json::from_value(serde_json::json!({"name":format!("fabric-work-{}","bc".repeat(32)),"device":1,"inode":3,"world_device":1,"world_inode":4})).unwrap();
            IntentKind::FabricMayHaveSpawnedV1 { recovery, stage, workspace }
        }

        #[test]
        fn signed_fabric_transaction_resumes_on_either_side_of_journal_cas() {
            use super::super::super::{
                create_dir, draft_genesis, replace_checkpoint, save, ExclusiveTurnApprovalV1, ExclusiveTurnCreationV1,
                ExclusiveTurnHistoryV1,
            };
            for after_cas in [false, true] {
                let tmp = tempfile::tempdir().unwrap();
                let paths = DataPaths::from_root(tmp.path().join("profile"));
                let local = PeerIdentity::load_or_create(&paths).unwrap();
                let remote = PeerIdentity::generate();
                let compatibility = Hash32([6; 32]);
                let source = tmp.path().join("world");
                create_dir(&source).unwrap();
                // Minimal valid gzip NBT, Data/DataVersion = 4790.
                let level = hex::decode(
                    "1f8b0800000000000203e36260e0626071492c496466e0065161a945c599f9790c0c42db181800d16c62f11e000000",
                )
                .unwrap();
                publish(&source.join("level.dat"), &level, false).unwrap();
                publish(&source.join("session.lock"), b"", false).unwrap();
                publish(&source.join("marker"), b"old", false).unwrap();
                let initial_bundle = tmp.path().join("initial-bundle");
                let (initial, bytes) = save::capture_test_bundle(&source, &initial_bundle, compatibility).unwrap();
                let genesis = draft_genesis(
                    swarm_protocol::EXCLUSIVE_TURN_SAVE_FORMAT_V2,
                    vec![hex::encode(local.public_key()), hex::encode(remote.public_key())],
                    local.peer_id(),
                    compatibility,
                    initial,
                )
                .unwrap();
                let world = genesis.world_id().unwrap();
                let grant = genesis.initial_grant_hash().unwrap();
                let mut approvals = [&local, &remote].map(|key| ExclusiveTurnApprovalV1 {
                    world_id: world,
                    device: key.peer_id(),
                    signature: key.sign(&genesis.approval_bytes(key.peer_id()).unwrap()),
                });
                approvals.sort_by_key(|a| a.device);
                let history = ExclusiveTurnHistoryV1 {
                    creation: ExclusiveTurnCreationV1 { genesis, approvals },
                    releases: Vec::new(),
                };
                let peer = local.peer_id();
                let store = Store::create(&paths, world, local).unwrap();
                create_dir(&store.dir.join("save-bundles")).unwrap();
                save::retain_bundle(&store, &initial_bundle, initial, compatibility, &bytes).unwrap();
                store.retain(initial, &bytes).unwrap();
                let mut journal = Journal {
                    format: FENCED_JOURNAL_FORMAT,
                    identity: peer,
                    history,
                    phase: Phase::Held,
                    attempt: 0,
                    checkpoint: initial,
                    staged: None,
                };
                let head = store.save(world, &journal, false).unwrap();
                let scope = Scope::held(&journal, &head, initial.reference_commitment).unwrap();
                let mut state = State::empty(world, peer);
                state.opt_in(scope.clone(), 0, 512, 500).unwrap();
                let consent_lock = create_lock(&store.dir).unwrap();
                persist(&store.dir, &store.identity, &state, false).unwrap();
                state.reserve(&scope, 1).unwrap();
                let reserved = state.intent.as_ref().unwrap().id().unwrap();
                let running = state.begin_fabric(&scope, 2, reserved, fixture_fabric_kind(&scope)).unwrap();
                persist(&store.dir, &store.identity, &state, true).unwrap();
                assert!(state.finish_probe(&scope, 3, running).is_err());
                assert!(state.recovery_pin(&scope, 3, running).is_err());
                assert!(state.clear(&scope, 3, running).is_err());
                let mut revoked = state.clone();
                revoked.revoke(grant, 3).unwrap();
                assert!(revoked.check_fabric(&scope, 4, running).is_err());
                assert!(revoked.prepare_fabric_checkpoint(&scope, 4, running, initial, Hash32([8; 32])).is_err());
                assert!(revoked.fabric_recovery_pin(&scope, 4, running).is_ok());
                publish(&source.join("marker"), b"new", true).unwrap();
                let target =
                    save::capture_runtime_checkpoint(&store, &source, &tmp.path().join("target-bundle"), compatibility)
                        .unwrap();
                assert_ne!(initial, target);
                assert!(state.prepare_fabric_checkpoint(&scope, 3, running, target, Hash32::default()).is_err());
                let pending = state.prepare_fabric_checkpoint(&scope, 3, running, target, Hash32([8; 32])).unwrap();
                assert_ne!(pending, running);
                persist(&store.dir, &store.identity, &state, true).unwrap();
                assert!(state.consent.is_none());
                assert!(state
                    .finish_pending_checkpoint(
                        &scope,
                        initial.reference_commitment,
                        target.reference_commitment,
                        4,
                        pending
                    )
                    .is_err());
                assert!(state.fabric_recovery_pin(&scope, 4, pending).is_err());
                assert!(state.clear(&scope, 4, pending).is_err());
                assert!(state.opt_in(scope.clone(), 4, 512, 500).is_err());
                if after_cas {
                    let bytes = store.verify_checkpoint(&head, target).unwrap();
                    replace_checkpoint(&store, &mut journal, &head, initial.reference_commitment, target, &bytes)
                        .unwrap();
                }
                drop(consent_lock);
                drop(store);
                // Simulate a fresh process after either durable boundary.
                assert!(Store::open(&paths, world).is_err(), "pending sidecar failed to fence ordinary commands");
                let store = Store::open_for_runtime_intent(&paths, world).unwrap();
                let (mut journal, head) = store.load(world).unwrap();
                let _lock = lock_existing(&store.dir).unwrap();
                let mut state = load(&store.dir, world, &store.identity).unwrap().unwrap();
                assert_eq!(journal.checkpoint, if after_cas { target } else { initial });
                assert!(publish_pending(
                    &store,
                    &mut journal,
                    &head,
                    &mut state,
                    initial.reference_commitment,
                    target.reference_commitment,
                    3,
                    pending
                )
                .is_err());
                assert!(publish_pending(
                    &store,
                    &mut journal,
                    &head,
                    &mut state,
                    initial.reference_commitment,
                    Hash32([9; 32]),
                    4,
                    pending
                )
                .is_err());
                let record = store.dir.join("fabric-process.json");
                publish(&record, b"unresolved", false).unwrap();
                assert!(publish_pending(
                    &store,
                    &mut journal,
                    &head,
                    &mut state,
                    initial.reference_commitment,
                    target.reference_commitment,
                    4,
                    pending
                )
                .is_err());
                fs::remove_file(record).unwrap();
                // Even a correctly signed pending transaction cannot publish
                // if one of its retained target blobs has disappeared.
                let blobs = store.dir.join("save-bundles").join(target.reference_commitment.to_string()).join("blobs");
                let blob = fs::read_dir(blobs).unwrap().next().unwrap().unwrap().path();
                let removed = tmp.path().join("removed-blob");
                fs::rename(&blob, &removed).unwrap();
                assert!(publish_pending(
                    &store,
                    &mut journal,
                    &head,
                    &mut state,
                    initial.reference_commitment,
                    target.reference_commitment,
                    4,
                    pending
                )
                .is_err());
                fs::rename(&removed, &blob).unwrap();
                assert_eq!(
                    publish_pending(
                        &store,
                        &mut journal,
                        &head,
                        &mut state,
                        initial.reference_commitment,
                        target.reference_commitment,
                        4,
                        pending
                    )
                    .unwrap(),
                    Hash32([8; 32])
                );
                assert_eq!(state.revision, 5);
                assert!(state.intent.is_none() && state.consent.is_none());
                drop(_lock);
                drop(store);
                let store = Store::open(&paths, world).unwrap();
                let (journal, head) = store.load(world).unwrap();
                assert_eq!(journal.checkpoint, target);
                assert_eq!(head.grant(), grant);
                assert_eq!(head.epoch(), 1);
                assert_eq!(head.fence(), 1);
                assert_eq!(head.checkpoint(), initial);
            }
        }
    }

    #[derive(Serialize, Deserialize)]
    struct SignedState {
        state: State,
        signature: Vec<u8>,
    }

    fn signing_bytes(state: &State) -> Result<Vec<u8>> {
        let mut bytes = DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(state)?);
        Ok(bytes)
    }

    fn exists(path: &Path) -> Result<bool> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn load(directory: &Path, world: ExclusiveTurnWorldIdV1, identity: &PeerIdentity) -> Result<Option<State>> {
        let lock = exists(&directory.join(LOCK))?;
        let record = exists(&directory.join(STATE))?;
        ensure!(lock == record, "incomplete durable runtime intent state; reference operations refused");
        if !lock {
            return Ok(None);
        }
        private_file(&directory.join(LOCK))?;
        let file = private_file(&directory.join(STATE))?;
        file.sync_all()?;
        sync_dir(directory)?;
        let bytes = read_bounded(file, LIMIT)?;
        let signed: SignedState = postcard::from_bytes(&bytes).context("invalid signed runtime intent state")?;
        ensure!(postcard::to_allocvec(&signed)? == bytes, "noncanonical runtime intent state");
        VerifyingKey::from_bytes(&identity.public_key())?
            .verify_strict(&signing_bytes(&signed.state)?, &Signature::from_slice(&signed.signature)?)
            .context("runtime intent state signature invalid")?;
        signed.state.validate(world, identity.peer_id())?;
        Ok(Some(signed.state))
    }

    fn persist(directory: &Path, identity: &PeerIdentity, state: &State, replace: bool) -> Result<()> {
        state.validate(state.world, identity.peer_id())?;
        let signed = SignedState { state: state.clone(), signature: identity.sign(&signing_bytes(state)?) };
        let bytes = postcard::to_allocvec(&signed)?;
        ensure!(bytes.len() <= LIMIT, "runtime intent state exceeds byte limit");
        publish(&directory.join(STATE), &bytes, replace)
    }

    fn lock_existing(directory: &Path) -> Result<File> {
        let lock =
            private_file(&directory.join(LOCK)).context("local runtime consent state is absent or unreadable")?;
        FileExt::try_lock_exclusive(&lock).context("runtime local state busy; operation refused")?;
        Ok(lock)
    }

    fn create_lock(directory: &Path) -> Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(directory.join(LOCK))?;
        FileExt::try_lock_exclusive(&lock)?;
        lock.sync_all()?;
        sync_dir(directory)?;
        Ok(lock)
    }

    pub(in super::super) fn refuse_unresolved(store: &Store, world: ExclusiveTurnWorldIdV1) -> Result<()> {
        if let Some(state) = load(&store.dir, world, &store.identity)? {
            ensure!(state.intent.is_none(), "unresolved durable runtime intent; reference operations refused");
        }
        Ok(())
    }

    pub(in super::super) fn check_journal_format(
        store: &Store,
        world: ExclusiveTurnWorldIdV1,
        format: u16,
    ) -> Result<()> {
        let state = load(&store.dir, world, &store.identity)?;
        ensure!(
            (format == swarm_protocol::EXCLUSIVE_TURN_FORMAT_V1 && state.is_none())
                || (format == FENCED_JOURNAL_FORMAT && state.is_some()),
            "local runtime intent journal format and sidecar disagree; refusing unsafe downgrade or reset"
        );
        Ok(())
    }

    fn report(state: &State, current: Option<&Scope>) -> Result<serde_json::Value> {
        let intent = state.intent.as_ref();
        let consent = state.consent.as_ref();
        let scope_report = |scope: &Scope| {
            serde_json::json!({
                "world_id": scope.world.to_string(), "holder": scope.holder.to_string(), "grant": scope.grant.to_string(),
                "epoch": scope.epoch, "fence": scope.fence, "checkpoint": scope.checkpoint.reference_commitment.to_string(),
                "compatibility": scope.compatibility.to_string(),
            })
        };
        Ok(serde_json::json!({
            "classification": "exclusive_turn_local_runtime_intent_only",
            "world_id": state.world.to_string(), "local_identity": state.peer.to_string(), "revision": state.revision.to_string(),
            "consent": consent.map(|c| serde_json::json!({"scope":scope_report(&c.scope),"revision":c.revision.to_string(),"heap_mib":c.heap_mib,"cpu_millicores":c.cpu_millicores})),
            "consent_matches_current_held_scope": current.is_some_and(|scope| consent.is_some_and(|c| &c.scope == scope)),
            "intent_id": intent.map(Intent::id).transpose()?.map(|id|id.to_string()),
            "intent_scope": intent.map(|i|scope_report(&i.consent.scope)),
        "intent_kind": intent.map(|i|i.kind.name()),
            "reference_operations_fenced": intent.is_some(),
            "capacity_verified": false, "resource_enforcement_verified": false,
            "process_spawned": if intent.is_some_and(|i| i.kind.may_have_spawned()) { serde_json::Value::Null } else { intent.is_some_and(|i| matches!(i.kind,IntentKind::FabricCheckpointPendingV1{..})).into() },
            "unresolved_possibly_spawned_intent": intent.is_some_and(|i| i.kind.may_have_spawned()),
            "kernel_recovery_pin_available": intent.is_some_and(|i| i.kind.recovery().is_some() || matches!(i.kind,IntentKind::FabricMayHaveSpawnedV1{..})),
            "staged_generation": intent.and_then(|i| i.kind.stage()).map(StagedPackagePin::name),
            "fabric_workspace":intent.and_then(|i|i.kind.workspace()).map(FabricWorkspacePin::name),
            "pending_fabric_checkpoint": intent.and_then(|i| match &i.kind { IntentKind::FabricCheckpointPendingV1{target,..}=>Some(*target),_=>None }),
            "save_barrier_verified": false, "canonical_authority": false, "write_authority": false,
            "runtime_activation": false, "player_admission": false, "minecraft_gameplay_verified": false,
        }))
    }

    fn opened(paths: &DataPaths, pin: &Pin) -> Result<(Store, Journal, VerifiedExclusiveTurnHead)> {
        let store = Store::open_for_runtime_intent(paths, pin.expected_world)?;
        let (journal, head) = store.load(pin.expected_world)?;
        pin_grant(&head, pin.expected_grant)?;
        Ok((store, journal, head))
    }

    /// Crash-safe two-record publication. The signed pending state remains
    /// fenced across the journal CAS and retirement, even if either fsync
    /// reports an ambiguous failure. Resumption never executes Java again.
    // Keep every independently reviewed transaction pin explicit at the call site.
    #[allow(clippy::too_many_arguments)]
    fn publish_pending(
        store: &Store,
        journal: &mut Journal,
        head: &VerifiedExclusiveTurnHead,
        state: &mut State,
        expected_base: Hash32,
        expected_target: Hash32,
        expected_revision: u64,
        expected_intent: Hash32,
    ) -> Result<Hash32> {
        let current = Scope::held(journal, head, journal.checkpoint.reference_commitment)?;
        let (base, target, evidence) =
            state.pending_checkpoint(&current, expected_base, expected_target, expected_revision, expected_intent)?;
        ensure!(
            !exists(&store.dir.join("fabric-process.json"))?,
            "Fabric process ownership record remains; checkpoint publication refused"
        );
        // Never infer target retention from the signed state alone.
        let bytes = store.verify_checkpoint(head, target)?;
        if journal.checkpoint == base.checkpoint {
            super::super::replace_checkpoint(store, journal, head, expected_base, target, &bytes)?;
        } else {
            ensure!(journal.checkpoint == target, "pending Fabric journal differs from exact target");
            // load() already verified the current record and synchronized
            // its directory; re-verify the target bytes before retirement.
            store.verify_checkpoint(head, target)?;
        }
        let published = Scope::held(journal, head, expected_target)?;
        state.finish_pending_checkpoint(
            &published,
            expected_base,
            expected_target,
            expected_revision,
            expected_intent,
        )?;
        persist(&store.dir, &store.identity, state, true)?;
        Ok(evidence)
    }

    pub(in super::super) fn run(paths: &DataPaths, command: RuntimeCommand) -> Result<()> {
        match command {
            RuntimeCommand::Status { pin } => {
                let (store, journal, head) = opened(paths, &pin)?;
                let state = load(&store.dir, head.world(), &store.identity)?
                    .unwrap_or_else(|| State::empty(head.world(), journal.identity));
                let scope = Scope::held(&journal, &head, journal.checkpoint.reference_commitment).ok();
                print_document(&report(&state, scope.as_ref())?)
            }
            RuntimeCommand::Consent { review, expected_checkpoint, expected_revision, heap_mib, cpu_millicores } => {
                review.confirm()?;
                let (store, mut journal, head) = opened(paths, &review.pin)?;
                let scope = Scope::held(&journal, &head, expected_checkpoint)?;
                let prior = load(&store.dir, head.world(), &store.identity)?;
                let (mut state, replace, lock) = if prior.is_some() {
                    let lock = lock_existing(&store.dir)?;
                    (load(&store.dir, head.world(), &store.identity)?.context("runtime state disappeared")?, true, lock)
                } else {
                    // Validate before creating the durable pair. An interrupted
                    // first publication is deliberately not silently recreated.
                    let mut state = State::empty(head.world(), journal.identity);
                    state.opt_in(scope.clone(), expected_revision, heap_mib, cpu_millicores)?;
                    // First persist the incompatible local journal format. A
                    // crash during sidecar initialization denies both old and
                    // new binaries; no launch intent can have been issued.
                    journal.format = FENCED_JOURNAL_FORMAT;
                    store.save(head.world(), &journal, true)?;
                    let lock = create_lock(&store.dir)?;
                    persist(&store.dir, &store.identity, &state, false)?;
                    drop(lock);
                    return print_document(&report(&state, Some(&scope))?);
                };
                state.opt_in(scope.clone(), expected_revision, heap_mib, cpu_millicores)?;
                persist(&store.dir, &store.identity, &state, replace)?;
                drop(lock);
                print_document(&report(&state, Some(&scope))?)
            }
            RuntimeCommand::Reserve { review, expected_checkpoint, expected_revision, package } => {
                review.confirm()?;
                let (store, journal, head) = opened(paths, &review.pin)?;
                let scope = Scope::held(&journal, &head, expected_checkpoint)?;
                {
                    let _lock = lock_existing(&store.dir)?;
                    let state = load(&store.dir, head.world(), &store.identity)?
                        .context("explicit local runtime consent is absent")?;
                    state.exact_consent(&scope, expected_revision)?;
                }
                // Do not hold the consent lock across expensive hashing. A local
                // opt-out can win while the world lock fences reference changes.
                super::super::save::verify_held_package(&store, &journal, &head, &package)?;
                let lock = lock_existing(&store.dir)?;
                let mut state =
                    load(&store.dir, head.world(), &store.identity)?.context("runtime consent disappeared")?;
                state.reserve(&scope, expected_revision)?;
                persist(&store.dir, &store.identity, &state, true)?;
                // A blocked/lost acknowledgment must not prevent local opt-out.
                drop(lock);
                let mut result = report(&state, Some(&scope))?;
                result["package_preflight_verified"] = true.into();
                print_document(&result)
            }
            RuntimeCommand::Revoke { review, expected_revision } => {
                review.confirm()?;
                // Denial-only: no journal acquisition, no history change, no
                // new record creation, no intent clearance. A live supervisor
                // can observe this independently durable local withdrawal.
                let peer = identity(paths)?;
                let directory = dir(paths, review.pin.expected_world)?;
                check_dir(&paths.root.join(super::super::STORE))?;
                check_dir(&directory)?;
                let lock = lock_existing(&directory)?;
                let mut state =
                    load(&directory, review.pin.expected_world, &peer)?.context("local runtime consent is absent")?;
                state.revoke(review.pin.expected_grant, expected_revision)?;
                persist(&directory, &peer, &state, true)?;
                drop(lock);
                print_document(&report(&state, None)?)
            }
            RuntimeCommand::ProbeJava { review, expected_checkpoint, expected_revision, expected_intent, package } => {
                review.confirm()?;
                let (store, journal, head) = opened(paths, &review.pin)?;
                let scope = Scope::held(&journal, &head, expected_checkpoint)?;
                let consent = {
                    let _lock = lock_existing(&store.dir)?;
                    let state =
                        load(&store.dir, head.world(), &store.identity)?.context("runtime local state is absent")?;
                    state.unlaunched_consent(&scope, expected_revision, expected_intent)?.clone()
                };
                // Fresh save and package verification, then account-wide
                // resource admission. No launch-capable intent yet.
                super::super::save::verify_held_package(&store, &journal, &head, &package)?;
                let mut admission = swarm_cli::exclusive_turn_process_probe::ProbeAdmission::prepare(
                    paths,
                    &package,
                    &store.dir,
                    scope.compatibility,
                    consent.heap_mib,
                    consent.cpu_millicores,
                    consent.revision,
                )?;
                let lock = lock_existing(&store.dir)?;
                let mut state =
                    load(&store.dir, head.world(), &store.identity)?.context("runtime local state disappeared")?;
                state.unlaunched_consent(&scope, expected_revision, expected_intent)?;
                let recovery_pin = admission.prepare_recovery_pin()?;
                let probe = state.begin_probe(
                    &scope,
                    expected_revision,
                    expected_intent,
                    IntentKind::JavaProbeMayHaveSpawnedV3 {
                        recovery: recovery_pin.clone(),
                        stage: admission.staged_pin().clone(),
                    },
                )?;
                // Publish and fsync the possibly-spawned state BEFORE any
                // child or its ownership record. Hold consent lock across
                // the synchronous spawn and exact child identity capture.
                persist(&store.dir, &store.identity, &state, true)?;
                let revision = state.revision;
                let running = admission.start(&store.dir)?;
                drop(lock);
                let check = || {
                    let current =
                        load(&store.dir, head.world(), &store.identity)?.context("runtime local state disappeared")?;
                    current.check_probe(&scope, revision, probe)
                };
                let evidence = running.complete(check)?;
                let lock = lock_existing(&store.dir)?;
                let mut current =
                    load(&store.dir, head.world(), &store.identity)?.context("runtime local state disappeared")?;
                current.finish_probe(&scope, revision, probe)?;
                persist(&store.dir, &store.identity, &current, true)?;
                let cleanup = admission.cleanup_completed_leaf(&recovery_pin);
                let staged_cleanup = admission.cleanup_completed_stage();
                drop(lock);
                drop(admission);
                let mut result = report(&current, Some(&scope))?;
                result["classification"] = "exclusive_turn_isolated_java_probe_completed".into();
                result["probe_intent_id"] = probe.to_string().into();
                result["capacity_verified"] = true.into();
                result["resource_enforcement_verified"] = true.into();
                result["process_spawned"] = true.into();
                result["empty_cgroup_retired"] = cleanup.is_ok().into();
                if let Err(error) = cleanup {
                    result["empty_cgroup_cleanup_error"] = format!("{error:#}").into();
                }
                result["staged_generation_retired"] = staged_cleanup.is_ok().into();
                if let Err(error) = staged_cleanup {
                    result["staged_generation_cleanup_error"] = format!("{error:#}").into();
                }
                result["java_probe"] = serde_json::to_value(evidence)?;
                print_document(&result)
            }
            RuntimeCommand::RecoverJavaProbe { review, expected_checkpoint, expected_revision, expected_intent } => {
                review.confirm()?;
                let (store, journal, head) = opened(paths, &review.pin)?;
                let scope = Scope::held(&journal, &head, expected_checkpoint)?;
                let lock = lock_existing(&store.dir)?;
                let mut state =
                    load(&store.dir, head.world(), &store.identity)?.context("local runtime intent state is absent")?;
                let pin = state.recovery_pin(&scope, expected_revision, expected_intent)?.clone();
                let retained_stage =
                    state.intent.as_ref().and_then(|i| i.kind.stage()).map(|pin| pin.name().to_owned());
                let recovery = swarm_cli::exclusive_turn_process_probe::ProbeRecovery::retire(paths, &store.dir, &pin)?;
                state.finish_recovery(&scope, expected_revision, expected_intent)?;
                persist(&store.dir, &store.identity, &state, true)?;
                let method = recovery.method();
                drop(recovery);
                drop(lock);
                let mut result = report(&state, Some(&scope))?;
                result["classification"] = "exclusive_turn_interrupted_java_probe_recovered".into();
                result["retired_probe_intent_id"] = expected_intent.to_string().into();
                result["recovery_method"] = method.into();
                result["process_and_descendants_quiescent"] = true.into();
                result["prior_process_spawn_verified"] = false.into();
                result["qualification_succeeded"] = false.into();
                // A recovered stage may have been mutated or substituted.
                // Never delete/reuse it solely because its process is gone.
                result["retained_stage_for_review"] = serde_json::to_value(retained_stage)?;
                print_document(&result)
            }
            RuntimeCommand::QualifyFabric {
                review,
                expected_checkpoint,
                expected_revision,
                expected_intent,
                package,
                accept_minecraft_eula,
            } => {
                review.confirm()?;
                ensure!(
                    accept_minecraft_eula,
                    "explicit --accept-minecraft-eula required for isolated Fabric execution"
                );
                let (store, mut journal, head) = opened(paths, &review.pin)?;
                let scope = Scope::held(&journal, &head, expected_checkpoint)?;
                let consent = {
                    let _lock = lock_existing(&store.dir)?;
                    let state =
                        load(&store.dir, head.world(), &store.identity)?.context("runtime local state is absent")?;
                    state.unlaunched_consent(&scope, expected_revision, expected_intent)?.clone()
                };
                super::super::save::verify_held_package(&store, &journal, &head, &package)?;
                let mut admission = ProbeAdmission::prepare(
                    paths,
                    &package,
                    &store.dir,
                    scope.compatibility,
                    consent.heap_mib,
                    consent.cpu_millicores,
                    consent.revision,
                )?;
                admission.verify_fabric_launch_package()?;
                let workspace = FabricWorkspace::create(&store.dir)?;
                super::super::save::restore_runtime_checkpoint(&store, &journal, &head, &workspace.world())?;
                let workspace_pin = workspace.pin()?;
                let lock = lock_existing(&store.dir)?;
                let mut state =
                    load(&store.dir, head.world(), &store.identity)?.context("runtime local state disappeared")?;
                state.unlaunched_consent(&scope, expected_revision, expected_intent)?;
                let recovery = admission.prepare_recovery_pin()?;
                let running_intent = state.begin_fabric(
                    &scope,
                    expected_revision,
                    expected_intent,
                    IntentKind::FabricMayHaveSpawnedV1 {
                        recovery: recovery.clone(),
                        stage: admission.staged_pin().clone(),
                        workspace: workspace_pin.clone(),
                    },
                )?;
                persist(&store.dir, &store.identity, &state, true)?;
                let running_revision = state.revision;
                let running = admission.start_fabric(&store.dir, &workspace, &workspace_pin, scope.compatibility)?;
                drop(lock);
                let check = || {
                    load(&store.dir, head.world(), &store.identity)?
                        .context("runtime local state disappeared")?
                        .check_fabric(&scope, running_revision, running_intent)
                };
                let completed = running.complete(check)?;
                workspace.verify(&workspace_pin)?;
                // No publication before the authenticated shutdown, process
                // exit and exact saved-tree capture. Keep all fences held.
                let target = super::super::save::capture_runtime_checkpoint(
                    &store,
                    &workspace.world(),
                    &workspace.bundle(),
                    scope.compatibility,
                )?;
                workspace.verify(&workspace_pin)?;
                let lock = lock_existing(&store.dir)?;
                let mut current =
                    load(&store.dir, head.world(), &store.identity)?.context("runtime local state disappeared")?;
                current.check_fabric(&scope, running_revision, running_intent)?;
                // Retire the exact empty kernel leaf BEFORE signing the
                // pending checkpoint. No delayed child can attach later.
                admission.cleanup_completed_leaf(&recovery)?;
                let pending = current.prepare_fabric_checkpoint(
                    &scope,
                    running_revision,
                    running_intent,
                    target,
                    completed.evidence_digest,
                )?;
                persist(&store.dir, &store.identity, &current, true)?;
                let pending_revision = current.revision;
                publish_pending(
                    &store,
                    &mut journal,
                    &head,
                    &mut current,
                    expected_checkpoint,
                    target.reference_commitment,
                    pending_revision,
                    pending,
                )?;
                let stage_cleanup = admission.cleanup_completed_stage();
                let work_cleanup = workspace.cleanup(&workspace_pin);
                drop(lock);
                drop(admission);
                let mut result = report(&current, Some(&Scope::held(&journal, &head, target.reference_commitment)?))?;
                result["classification"] = "exclusive_turn_isolated_fabric_checkpoint_published".into();
                result["prior_checkpoint"] = expected_checkpoint.to_string().into();
                result["published_checkpoint"] = serde_json::to_value(target)?;
                result["fabric_intent_id"] = running_intent.to_string().into();
                result["checkpoint_transaction_id"] = pending.to_string().into();
                result["save_barrier_verified"] = true.into();
                result["resource_enforcement_verified"] = true.into();
                result["capacity_verified"] = true.into();
                result["process_spawned"] = true.into();
                result["process_and_descendants_quiescent"] = true.into();
                result["empty_cgroup_retired"] = true.into();
                result["fabric_lifecycle"] = serde_json::to_value(completed)?;
                result["staged_generation_retired"] = stage_cleanup.is_ok().into();
                result["fabric_workspace_retired"] = work_cleanup.is_ok().into();
                if let Err(e) = stage_cleanup {
                    result["stage_cleanup_error"] = format!("{e:#}").into();
                }
                if let Err(e) = work_cleanup {
                    result["workspace_cleanup_error"] = format!("{e:#}").into();
                }
                print_document(&result)
            }
            RuntimeCommand::ResumeFabricCheckpoint {
                review,
                expected_checkpoint,
                expected_target,
                expected_revision,
                expected_intent,
            } => {
                review.confirm()?;
                let (store, mut journal, head) = opened(paths, &review.pin)?;
                let lock = lock_existing(&store.dir)?;
                let mut state =
                    load(&store.dir, head.world(), &store.identity)?.context("runtime local state is absent")?;
                let stage = state.intent.as_ref().and_then(|i| i.kind.stage()).map(|s| s.name().to_owned());
                let workspace = state.intent.as_ref().and_then(|i| i.kind.workspace()).map(|s| s.name().to_owned());
                let evidence = publish_pending(
                    &store,
                    &mut journal,
                    &head,
                    &mut state,
                    expected_checkpoint,
                    expected_target,
                    expected_revision,
                    expected_intent,
                )?;
                drop(lock);
                let mut result = report(&state, Some(&Scope::held(&journal, &head, expected_target)?))?;
                result["classification"] = "exclusive_turn_pending_fabric_checkpoint_published".into();
                result["published_checkpoint"] = serde_json::to_value(journal.checkpoint)?;
                result["prior_lifecycle_evidence_digest"] = evidence.to_string().into();
                result["retired_checkpoint_transaction_id"] = expected_intent.to_string().into();
                result["retained_stage_for_review"] = serde_json::to_value(stage)?;
                result["retained_workspace_for_review"] = serde_json::to_value(workspace)?;
                print_document(&result)
            }
            RuntimeCommand::DiscardInterruptedFabric {
                review,
                expected_checkpoint,
                expected_revision,
                expected_intent,
                acknowledge_unconfirmed_save,
            } => {
                review.confirm()?;
                ensure!(
                    acknowledge_unconfirmed_save,
                    "explicit --acknowledge-unconfirmed-save required; uncertain writes are not published"
                );
                let (store, journal, head) = opened(paths, &review.pin)?;
                let scope = Scope::held(&journal, &head, expected_checkpoint)?;
                let lock = lock_existing(&store.dir)?;
                let mut state =
                    load(&store.dir, head.world(), &store.identity)?.context("runtime local state is absent")?;
                let pin = state.fabric_recovery_pin(&scope, expected_revision, expected_intent)?.clone();
                let stage = state.intent.as_ref().and_then(|i| i.kind.stage()).map(|s| s.name().to_owned());
                let workspace = state.intent.as_ref().and_then(|i| i.kind.workspace()).map(|s| s.name().to_owned());
                let recovery = ProbeRecovery::retire_fabric(paths, &store.dir, &pin)?;
                state.finish_fabric_discard(&scope, expected_revision, expected_intent)?;
                persist(&store.dir, &store.identity, &state, true)?;
                let method = recovery.method();
                drop(recovery);
                drop(lock);
                let mut result = report(&state, Some(&scope))?;
                result["classification"] = "exclusive_turn_interrupted_fabric_discarded".into();
                result["recovery_method"] = method.into();
                result["process_and_descendants_quiescent"] = true.into();
                result["unconfirmed_writes_published"] = false.into();
                result["old_checkpoint_preserved"] = true.into();
                result["retained_stage_for_review"] = serde_json::to_value(stage)?;
                result["retained_workspace_for_review"] = serde_json::to_value(workspace)?;
                print_document(&result)
            }
            RuntimeCommand::Clear { review, expected_checkpoint, expected_revision, expected_intent } => {
                review.confirm()?;
                let (store, journal, head) = opened(paths, &review.pin)?;
                let scope = Scope::held(&journal, &head, expected_checkpoint)?;
                let lock = lock_existing(&store.dir)?;
                let mut state =
                    load(&store.dir, head.world(), &store.identity)?.context("local runtime intent state is absent")?;
                state.clear(&scope, expected_revision, expected_intent)?;
                persist(&store.dir, &store.identity, &state, true)?;
                drop(lock);
                print_document(&report(&state, Some(&scope))?)
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) use linux::{check_journal_format, refuse_unresolved, run};

#[cfg(not(target_os = "linux"))]
pub(super) fn refuse_unresolved(_store: &Store, _world: ExclusiveTurnWorldIdV1) -> Result<()> {
    anyhow::bail!("runtime intent is qualified on Linux only")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn check_journal_format(_store: &Store, _world: ExclusiveTurnWorldIdV1, _format: u16) -> Result<()> {
    anyhow::bail!("runtime intent is qualified on Linux only")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn run(_paths: &DataPaths, _command: RuntimeCommand) -> Result<()> {
    anyhow::bail!("runtime intent is qualified on Linux only")
}
