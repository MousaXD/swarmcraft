//! Local runtime intent and isolated Java qualification. No Minecraft
//! launch, save mutation or canonical authority is reachable here.
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
    use swarm_cli::runtime_process_guard::ProbeRecoveryPin;
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
    }

    impl IntentKind {
        fn name(&self) -> &'static str {
            match self {
                Self::UnlaunchedNoProcessV1 => "unlaunched_no_process_v1",
                Self::JavaProbeMayHaveSpawnedV1 => "java_probe_may_have_spawned_v1",
                Self::JavaProbeMayHaveSpawnedV2(_) => "java_probe_may_have_spawned_v2",
            }
        }

        fn is_probe(&self) -> bool {
            !matches!(self, Self::UnlaunchedNoProcessV1)
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
                if let IntentKind::JavaProbeMayHaveSpawnedV2(pin) = &intent.kind {
                    pin.verify_budget(intent.consent.heap_mib, intent.consent.cpu_millicores, intent.consent.revision)?;
                }
                ensure!(intent.consent.revision < self.revision, "runtime intent precedes its consent");
                ensure!(
                    self.consent.as_ref().is_none_or(|consent| consent == &intent.consent),
                    "runtime intent and consent disagree"
                );
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
            if let IntentKind::JavaProbeMayHaveSpawnedV2(pin) = &kind {
                pin.verify_budget(consent.heap_mib, consent.cpu_millicores, consent.revision)?;
            }
            self.next(expected)?;
            let intent = self.intent.as_mut().context("runtime intent disappeared")?;
            intent.kind = kind;
            intent.id()
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
                IntentKind::JavaProbeMayHaveSpawnedV2(pin) => Ok(pin),
                IntentKind::JavaProbeMayHaveSpawnedV1 => {
                    anyhow::bail!("legacy possibly-spawned probe lacks a signed kernel recovery pin; recovery refused")
                }
                IntentKind::UnlaunchedNoProcessV1 => anyhow::bail!("unlaunched intent is not a recoverable Java probe"),
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
            let probe = state
                .begin_probe(&scope, 2, unlaunched, IntentKind::JavaProbeMayHaveSpawnedV2(recovery.clone()))
                .unwrap();
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
            "process_spawned": if intent.is_some_and(|i| i.kind.is_probe()) { serde_json::Value::Null } else { false.into() },
            "unresolved_possibly_spawned_intent": intent.is_some_and(|i| i.kind.is_probe()),
            "kernel_recovery_pin_available": intent.is_some_and(|i| matches!(i.kind, IntentKind::JavaProbeMayHaveSpawnedV2(_))),
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
                    IntentKind::JavaProbeMayHaveSpawnedV2(recovery_pin.clone()),
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
