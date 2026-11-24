use crate::{
    authority_permit::PermitWatch,
    host_readiness, hosting_budget, launch_guard,
    live_fabric_readiness::{challenge_authenticated_session, ExpectedFabricRuntime},
    live_runtime_registry::{LiveFabricRuntimeRegistry, LiveRuntimeIdentity, RuntimeChallengeCommand},
    runtime_process_guard::RuntimeProcessGuard,
    server_mods,
};
#[path = "managed_server_properties.rs"]
mod managed_server_properties;
use anyhow::{anyhow, bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use swarm_consensus::AuthorityGeneration;
use swarm_core::{
    lifecycle::verify_sleep_record_signature, verify_signature, verify_snapshot_signature, verify_transfer_signature,
    DataPaths, PeerIdentity,
};
use swarm_ipc::FabricBridgeListener;
use swarm_protocol::{
    validate_runtime_selection, AuthorityTransferV1, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, PeerId,
    SleepRecordV1, SnapshotManifestV1, TransferPhase, WorldId, PROTOCOL_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, StorageError};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tracing::{info, warn};

const FABRIC_START_TIMEOUT: Duration = Duration::from_secs(60);
const FABRIC_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const SUPERVISOR_POLL: Duration = Duration::from_millis(250);
const STOP_INTENT_SCHEMA_VERSION: u16 = 1;
// serde encodes WorldId/PeerId as byte arrays; pretty JSON can be larger
// than a compact machine token. Keep an explicit, small read ceiling.
const MAX_STOP_INTENT_BYTES: u64 = 4 * 1024;
const TRANSFER_INTENT_SCHEMA_VERSION: u16 = 1;
const MAX_TRANSFER_INTENT_BYTES: u64 = 4 * 1024;
static CONTROL_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Machine-local stop requests must be tied to the exact authority generation.
/// An old UI request must never stop a newly recovered or transferred host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StopIntentV1 {
    schema_version: u16,
    world_id: WorldId,
    epoch: u64,
    fencing_token: u64,
    authority_peer_id: PeerId,
}

impl StopIntentV1 {
    fn from_epoch(epoch: &EpochRecordV1) -> Self {
        Self {
            schema_version: STOP_INTENT_SCHEMA_VERSION,
            world_id: epoch.world_id,
            epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            authority_peer_id: epoch.authority_peer_id,
        }
    }

    fn matches_epoch(&self, epoch: &EpochRecordV1) -> bool {
        self == &Self::from_epoch(epoch)
    }
}

/// A local transfer request is scoped to the exact authority generation.
/// This is not signed transfer authorization: that is created only after
/// Fabric has checkpointed and the canonical generation is revalidated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TransferIntentV1 {
    schema_version: u16,
    world_id: WorldId,
    epoch: u64,
    fencing_token: u64,
    authority_peer_id: PeerId,
    target_peer_id: PeerId,
}

impl TransferIntentV1 {
    fn from_epoch(epoch: &EpochRecordV1, target_peer_id: PeerId) -> Self {
        Self {
            schema_version: TRANSFER_INTENT_SCHEMA_VERSION,
            world_id: epoch.world_id,
            epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            authority_peer_id: epoch.authority_peer_id,
            target_peer_id,
        }
    }

    fn matches_epoch(&self, epoch: &EpochRecordV1) -> bool {
        self.schema_version == TRANSFER_INTENT_SCHEMA_VERSION
            && self.world_id == epoch.world_id
            && self.epoch == epoch.epoch_number
            && self.fencing_token == epoch.fencing_token
            && self.authority_peer_id == epoch.authority_peer_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeLaunchConfig {
    pub java: PathBuf,
    pub server_jar: PathBuf,
    pub mod_jar: PathBuf,
    pub accept_eula: bool,
    pub game_endpoint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HostOptions {
    pub world: WorldId,
    pub java: PathBuf,
    pub server_jar: PathBuf,
    pub mod_jar: PathBuf,
    pub accept_eula: bool,
}

impl RuntimeLaunchConfig {
    fn host_options(&self, world: WorldId) -> HostOptions {
        HostOptions {
            world,
            java: self.java.clone(),
            server_jar: self.server_jar.clone(),
            mod_jar: self.mod_jar.clone(),
            accept_eula: self.accept_eula,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationTrigger {
    AutomaticRecovery,
    ManualTransfer,
    WorldWake,
    DirectHost,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationPhase {
    Dormant,
    WaitingForAuthority,
    WaitingForQuorum,
    SelectingSnapshot,
    PreparingRuntime,
    RestoringWorld,
    LaunchingRuntime,
    VerifyingFabric,
    Ready,
    Checkpointing,
    AwaitingTransferAcceptance,
    Sleeping,
    Blocked,
    Failed,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationStatus {
    pub world_id: String,
    pub authority_peer_id: Option<String>,
    pub epoch: Option<u64>,
    pub fencing_token: Option<u64>,
    pub trigger: Option<MigrationTrigger>,
    pub phase: MigrationPhase,
    pub runtime_ready: bool,
    pub game_endpoint: Option<String>,
    pub snapshot_hash: Option<String>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferPrepareResult {
    CheckpointRequested,
    Prepared(String),
}

/// A local durable progress report, NOT a current quorum, route, or shutdown
/// proof. Read after restart without carrying signed tokens in the UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferProgressPhase {
    Idle,
    CheckpointRequested,
    Prepared,
    Accepted,
    Committed,
    SuccessorObserved,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferProgress {
    pub world_id: String,
    pub phase: TransferProgressPhase,
    pub current_authority: String,
    pub current_epoch: u64,
    pub source_peer: Option<String>,
    pub target_peer: Option<String>,
    pub source_epoch: Option<u64>,
    pub checkpoint_hash: Option<String>,
}

pub fn transfer_progress(paths: &DataPaths, storage: &Storage, world: WorldId) -> Result<TransferProgress> {
    let epoch = storage.load_epoch_record(world)?;
    verify_signature(epoch.authority_peer_id, epoch.authority_public_key, &epoch.signing_bytes()?, &epoch.signature)?;
    let mut progress = TransferProgress {
        world_id: world.to_string(),
        phase: TransferProgressPhase::Idle,
        current_authority: epoch.authority_peer_id.to_string(),
        current_epoch: epoch.epoch_number,
        source_peer: None,
        target_peer: None,
        source_epoch: None,
        checkpoint_hash: None,
    };
    let record = match storage.load_transfer_record(world) {
        Ok(record) => Some(record),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(record) = record {
        verify_transfer_signature(&record)?;
        let source_generation = transfer_is_source_generation(&record, &epoch);
        let successor_generation = transfer_is_successor_generation(&record, &epoch, epoch.authority_peer_id);
        if source_generation || successor_generation {
            let descriptor = storage.load_world_descriptor(world)?;
            let from = descriptor.member(record.from_peer_id).context("transfer source is not a member")?;
            let to = descriptor.member(record.to_peer_id).context("transfer target is not a member")?;
            let signer = if record.phase == TransferPhase::Accepted { to } else { from };
            if from.banned
                || to.banned
                || !to.authority_eligible
                || signer.peer_id != record.signer_peer_id
                || signer.public_key != record.signer_public_key
                || (source_generation && from.public_key != epoch.authority_public_key)
                || (successor_generation && to.public_key != epoch.authority_public_key)
            {
                bail!("transfer progress does not match accepted membership");
            }
        }
        progress.phase = if source_generation {
            match record.phase {
                TransferPhase::Prepared => TransferProgressPhase::Prepared,
                TransferPhase::Accepted => TransferProgressPhase::Accepted,
                TransferPhase::Committed => TransferProgressPhase::Committed,
            }
        } else if successor_generation && epoch.mode == EpochMode::Quorum && epoch.previous_epoch_hash.is_some() {
            TransferProgressPhase::SuccessorObserved
        } else {
            TransferProgressPhase::Superseded
        };
        progress.source_peer = Some(record.from_peer_id.to_string());
        progress.target_peer = Some(record.to_peer_id.to_string());
        progress.source_epoch = record.next_epoch.checked_sub(1);
        progress.checkpoint_hash = Some(record.base_snapshot_hash.to_string());
        if source_generation {
            return Ok(progress);
        }
    }
    // A new local intent may follow a completed or superseded older handoff.
    if let Some(target) = load_transfer_intent(paths, &epoch)? {
        progress.phase = TransferProgressPhase::CheckpointRequested;
        progress.source_peer = Some(epoch.authority_peer_id.to_string());
        progress.target_peer = Some(target.to_string());
        progress.source_epoch = Some(epoch.epoch_number);
        progress.checkpoint_hash = None;
    }
    Ok(progress)
}

/// Begin a daemon-driven handoff on the exact epoch displayed to the user.
/// Reopening the wizard reads progress; no UI token is required to resume.
pub fn request_automatic_transfer(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    target: PeerId,
    source_epoch: u64,
) -> Result<TransferProgress> {
    prepare_transfer(paths, storage, world, target, Some(source_epoch))?;
    transfer_progress(paths, storage, world)
}

pub fn save_runtime_config(paths: &DataPaths, world: WorldId, config: &RuntimeLaunchConfig) -> Result<()> {
    // Any launch-path change invalidates the prior machine-local runtime proof.
    host_readiness::invalidate_runtime_verification(paths, world)?;
    let path = runtime_config_path(paths, world);
    atomic_json(&path, config)?;
    Ok(())
}

pub fn load_runtime_config(paths: &DataPaths, world: WorldId) -> Result<RuntimeLaunchConfig> {
    let path = runtime_config_path(paths, world);
    let bytes =
        fs::read(&path).with_context(|| format!("runtime launch configuration is missing at {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn load_migration_status(paths: &DataPaths, world: WorldId) -> Result<MigrationStatus> {
    let path = migration_status_path(paths, world);
    let bytes = fs::read(&path).with_context(|| format!("migration status is unavailable at {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Returns only the locally managed Minecraft listener port assignment.
/// This is not runtime-readiness or signed route authorization: consumers
/// must independently verify live authenticated Fabric IPC, current canonical
/// quorum, membership, consent and fencing before forwarding any game data.
pub fn managed_minecraft_port(paths: &DataPaths, world: WorldId) -> Result<Option<u16>> {
    managed_server_properties::recorded_port(paths, world)
}

pub fn request_world_stop(paths: &DataPaths, storage: &Storage, world: WorldId) -> Result<()> {
    storage.load_world(world)?;
    if launch_guard::load_sleep_record_fail_closed(storage, world)?.is_some() {
        return Ok(());
    }
    let identity = PeerIdentity::load_or_create(paths)?;
    let epoch = storage.load_epoch_record(world)?;
    if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {
        bail!("only the current authority may request a safe world stop");
    }
    // The user requested a checkpoint: immediately retract the previous
    // machine-local runtime proof and any green readiness cache, even before
    // Java reaches its save barrier. It must not accept new game sessions
    // during this transition.
    host_readiness::invalidate_runtime_verification(paths, world)?;
    host_readiness::invalidate_cached_readiness(paths, world)?;
    publish_status(
        paths,
        storage,
        world,
        Some(MigrationTrigger::DirectHost),
        MigrationPhase::Checkpointing,
        false,
        None,
        storage.latest_snapshot(world)?.and_then(|snapshot| snapshot.manifest_hash().ok()),
        None,
    )?;
    atomic_json(&stop_intent_path(paths, world), &StopIntentV1::from_epoch(&epoch))?;
    Ok(())
}

pub fn request_world_wake(paths: &DataPaths, storage: &Storage, world: WorldId) -> Result<()> {
    let consent = hosting_budget::require_contribution(paths, world)?;
    let identity = PeerIdentity::load_or_create(paths)?;
    let sleep_record = storage.load_sleep_record(world).context("world is not durably sleeping")?;
    verify_sleep_record_signature(&sleep_record)?;
    let latest = storage.latest_snapshot(world)?.context("sleeping world has no canonical snapshot")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;
    if latest.manifest_hash()? != sleep_record.latest_snapshot_hash {
        bail!("sleep record does not reference the exact latest verified snapshot");
    }
    let descriptor = storage.load_world_descriptor(world)?;
    let local = descriptor.member(identity.peer_id()).context("local peer is not a world member")?;
    if local.banned || !local.authority_eligible || local.public_key != identity.public_key() {
        bail!("local peer is not eligible to wake this world");
    }
    hosting_budget::with_consent_guarded_operation(paths, world, &consent, || {
        atomic_bytes(&wake_intent_path(paths, world), b"wake\n")
    })?;
    publish_status(
        paths,
        storage,
        world,
        Some(MigrationTrigger::WorldWake),
        MigrationPhase::WaitingForAuthority,
        false,
        None,
        Some(latest.manifest_hash()?),
        None,
    )?;
    Ok(())
}

pub async fn supervise(paths: DataPaths, runtime_registry: LiveFabricRuntimeRegistry) -> Result<()> {
    let storage = Storage::open(paths.root.clone())?;
    let mut tasks: HashMap<WorldId, JoinHandle<()>> = HashMap::new();
    loop {
        tasks.retain(|_, task| !task.is_finished());
        for metadata in storage.list_worlds()? {
            if tasks.contains_key(&metadata.world_id) {
                continue;
            }
            let world = metadata.world_id;
            let task_paths = paths.clone();
            let task_registry = runtime_registry.clone();
            let task = tokio::spawn(async move {
                if let Err(error) = supervise_world(task_paths.clone(), world, task_registry).await {
                    warn!(%world, %error, "migration runtime supervisor stopped");
                    if let Ok(storage) = Storage::open(task_paths.root.clone()) {
                        let _ = publish_status(
                            &task_paths,
                            &storage,
                            world,
                            None,
                            MigrationPhase::Failed,
                            false,
                            None,
                            None,
                            Some(error.to_string()),
                        );
                    }
                }
            });
            tasks.insert(world, task);
        }
        sleep(Duration::from_secs(1)).await;
    }
}

async fn supervise_world(paths: DataPaths, world: WorldId, runtime_registry: LiveFabricRuntimeRegistry) -> Result<()> {
    let storage = Storage::open(paths.root.clone())?;
    let identity = PeerIdentity::load_or_create(&paths)?;
    loop {
        let config = match load_runtime_config(&paths, world) {
            Ok(config) => config,
            Err(error) => {
                publish_status(
                    &paths,
                    &storage,
                    world,
                    None,
                    MigrationPhase::Blocked,
                    false,
                    None,
                    None,
                    Some(error.to_string()),
                )?;
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        if let Some(transfer) = active_outbound_transfer(&storage, world, identity.peer_id())? {
            publish_status(
                &paths,
                &storage,
                world,
                Some(MigrationTrigger::ManualTransfer),
                MigrationPhase::AwaitingTransferAcceptance,
                false,
                config.game_endpoint.clone(),
                Some(transfer.base_snapshot_hash),
                None,
            )?;
            sleep(SUPERVISOR_POLL).await;
            continue;
        }

        match launch_guard::load_sleep_record_fail_closed(&storage, world) {
            Ok(Some(_)) => {
                if !wake_intent_path(&paths, world).exists() {
                    publish_status(
                        &paths,
                        &storage,
                        world,
                        None,
                        MigrationPhase::Sleeping,
                        false,
                        config.game_endpoint.clone(),
                        storage.latest_snapshot(world)?.and_then(|snapshot| snapshot.manifest_hash().ok()),
                        None,
                    )?;
                    sleep(SUPERVISOR_POLL).await;
                    continue;
                }
                // A sleeping world stays Sleeping for player-only peers.
                // But an explicit wake must re-check admission before any
                // authority generation is installed.
                if let Err(error) = hosting_budget::require_contribution(&paths, world) {
                    publish_status(
                        &paths,
                        &storage,
                        world,
                        Some(MigrationTrigger::WorldWake),
                        MigrationPhase::Blocked,
                        false,
                        config.game_endpoint.clone(),
                        None,
                        Some(format!("wake denied by local hosting budget: {error}")),
                    )?;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                let descriptor = storage.load_world_descriptor(world)?;
                let members = descriptor.members.iter().filter(|member| !member.banned).count();
                if members > 1 {
                    publish_status(
                        &paths,
                        &storage,
                        world,
                        Some(MigrationTrigger::WorldWake),
                        MigrationPhase::Blocked,
                        false,
                        config.game_endpoint.clone(),
                        storage.latest_snapshot(world)?.and_then(|snapshot| snapshot.manifest_hash().ok()),
                        Some("multi-member wake requires a quorum-backed authority transition; unsafe solo wake is not automatic".into()),
                    )?;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                // Another process may already own hosting (Desktop Play). Stay
                // silent and retry later instead of racing its runtime reset.
                let Some(authority_slot) = AuthorityRuntimeGuard::try_acquire(&paths, world)? else {
                    sleep(SUPERVISOR_POLL).await;
                    continue;
                };
                let result = run_authority_runtime_inner(
                    &paths,
                    &storage,
                    config.host_options(world),
                    MigrationTrigger::WorldWake,
                    config.game_endpoint.clone(),
                    false,
                    Some(&runtime_registry),
                )
                .await;
                drop(authority_slot);
                if let Err(error) = result {
                    publish_failure(&paths, &storage, world, MigrationTrigger::WorldWake, &config, &error)?;
                    sleep(Duration::from_secs(1)).await;
                }
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                publish_status(
                    &paths,
                    &storage,
                    world,
                    None,
                    MigrationPhase::Blocked,
                    false,
                    config.game_endpoint.clone(),
                    storage.latest_snapshot(world)?.and_then(|snapshot| snapshot.manifest_hash().ok()),
                    Some(format!("sleep state is unreadable or corrupt; authority launch is blocked: {error}")),
                )?;
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        }

        // Player-only peers continue to replicate, but may never start a JVM.
        // A missing/withdrawn or currently unsafe budget is an explicit block.
        if let Err(error) = hosting_budget::require_contribution(&paths, world) {
            publish_status(
                &paths,
                &storage,
                world,
                None,
                MigrationPhase::Blocked,
                false,
                config.game_endpoint.clone(),
                None,
                Some(format!("local hosting contribution is not admitted: {error}")),
            )?;
            sleep(Duration::from_secs(1)).await;
            continue;
        }

        let descriptor = storage.load_world_descriptor(world)?;
        let Some(local) = descriptor.member(identity.peer_id()) else {
            publish_status(
                &paths,
                &storage,
                world,
                None,
                MigrationPhase::WaitingForAuthority,
                false,
                config.game_endpoint.clone(),
                None,
                Some("local peer is no longer a world member".into()),
            )?;
            sleep(Duration::from_secs(1)).await;
            continue;
        };
        if local.banned || !local.authority_eligible || local.public_key != identity.public_key() {
            publish_status(
                &paths,
                &storage,
                world,
                None,
                MigrationPhase::Blocked,
                false,
                config.game_endpoint.clone(),
                None,
                Some("local peer is not authority eligible".into()),
            )?;
            sleep(Duration::from_secs(1)).await;
            continue;
        }

        let epoch = match storage.load_epoch_record(world) {
            Ok(epoch) => epoch,
            Err(_) => {
                publish_status(
                    &paths,
                    &storage,
                    world,
                    None,
                    MigrationPhase::WaitingForAuthority,
                    false,
                    config.game_endpoint.clone(),
                    None,
                    Some("no accepted authority epoch is available".into()),
                )?;
                sleep(SUPERVISOR_POLL).await;
                continue;
            }
        };
        if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {
            publish_status(
                &paths,
                &storage,
                world,
                None,
                MigrationPhase::WaitingForAuthority,
                false,
                config.game_endpoint.clone(),
                storage.latest_snapshot(world)?.and_then(|snapshot| snapshot.manifest_hash().ok()),
                None,
            )?;
            sleep(SUPERVISOR_POLL).await;
            continue;
        }

        if !wait_until_launch_safe(&paths, &storage, &identity, world, &descriptor, &epoch, &config).await? {
            continue;
        }
        let trigger = infer_trigger(&storage, &epoch, identity.peer_id());
        // Another process may already own hosting (Desktop Play or a CLI
        // launch). Skip this tick silently instead of resetting its runtime
        // directory underneath a live Minecraft process.
        let Some(authority_slot) = AuthorityRuntimeGuard::try_acquire(&paths, world)? else {
            sleep(SUPERVISOR_POLL).await;
            continue;
        };
        let result = run_authority_runtime_inner(
            &paths,
            &storage,
            config.host_options(world),
            trigger,
            config.game_endpoint.clone(),
            false,
            Some(&runtime_registry),
        )
        .await;
        drop(authority_slot);
        match result {
            Ok(()) => {}
            Err(error) => {
                publish_failure(&paths, &storage, world, trigger, &config, &error)?;
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn wait_until_launch_safe(
    paths: &DataPaths,
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
    descriptor: &swarm_protocol::WorldDescriptorV1,
    epoch: &EpochRecordV1,
    config: &RuntimeLaunchConfig,
) -> Result<bool> {
    let member_count = descriptor.members.iter().filter(|member| !member.banned).count();
    if member_count <= 1 {
        return Ok(true);
    }
    let generation = AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token };
    let mut watch = PermitWatch::new(generation);
    publish_status(
        paths,
        storage,
        world,
        Some(infer_trigger(storage, epoch, identity.peer_id())),
        MigrationPhase::WaitingForQuorum,
        false,
        config.game_endpoint.clone(),
        storage.latest_snapshot(world)?.and_then(|snapshot| snapshot.manifest_hash().ok()),
        None,
    )?;
    loop {
        if hosting_budget::require_contribution(paths, world).is_err() {
            return Ok(false);
        }
        let current = storage.load_epoch_record(world)?;
        if current.epoch_number != epoch.epoch_number
            || current.fencing_token != epoch.fencing_token
            || current.authority_peer_id != identity.peer_id()
            || current.authority_public_key != identity.public_key()
        {
            return Ok(false);
        }
        if active_outbound_transfer(storage, world, identity.peer_id())?.is_some() {
            return Ok(false);
        }
        if watch.observe(paths, world, Instant::now()).unwrap_or(false) {
            return Ok(true);
        }
        sleep(SUPERVISOR_POLL).await;
    }
}

fn infer_trigger(storage: &Storage, epoch: &EpochRecordV1, local_peer: PeerId) -> MigrationTrigger {
    if epoch.mode == EpochMode::Recovery {
        return if epoch.reason == "quorum wake from exact durable sleep boundary" {
            MigrationTrigger::WorldWake
        } else {
            MigrationTrigger::AutomaticRecovery
        };
    }
    if storage
        .load_transfer_record(epoch.world_id)
        .is_ok_and(|transfer| transfer_is_successor_generation(&transfer, epoch, local_peer))
    {
        return MigrationTrigger::ManualTransfer;
    }
    MigrationTrigger::DirectHost
}

/// Single-flight ownership of one world's authority runtime on this device.
///
/// The Desktop Play action (`swarmcraft-runtime launch`) and the daemon
/// migration supervisor can both try to host the same solo world. A second
/// concurrent authority runtime resets `runtime/<world>/` underneath the live
/// Minecraft process and races its Fabric boot downloads, which surfaces to
/// players as confusing "No such file or directory" setup failures. This
/// advisory whole-file lock makes later attempts fail fast (explicit launches)
/// or stay silent and retry later (supervisor ticks) instead of racing.
struct AuthorityRuntimeGuard {
    file: fs::File,
}

impl AuthorityRuntimeGuard {
    fn lock_path(paths: &DataPaths, world: WorldId) -> PathBuf {
        paths.root.join("control").join(world.to_hex()).join("authority-runtime.lock")
    }

    /// Fail-fast acquisition for explicit launch entry points.
    fn acquire(paths: &DataPaths, world: WorldId) -> Result<Self> {
        Self::try_acquire(paths, world)?.ok_or_else(|| {
            anyhow!(
                "this world already has an active authority runtime on this device; stop it before starting another"
            )
        })
    }

    /// Non-fatal contention probe for the background supervisor. Only a
    /// real held lock becomes `Ok(None)`; filesystem and locking failures
    /// are propagated so recovery cannot silently stall forever.
    fn try_acquire(paths: &DataPaths, world: WorldId) -> Result<Option<Self>> {
        let path = Self::lock_path(paths, world);
        let parent = path.parent().context("authority runtime lock path has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot prepare authority runtime lock directory {}", parent.display()))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("cannot open authority runtime lock {}", path.display()))?;
        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot acquire authority runtime lock {}", path.display()));
            }
        }
        file.set_len(0).with_context(|| format!("cannot reset authority runtime lock {}", path.display()))?;
        write!(file, "pid={}", std::process::id())
            .with_context(|| format!("cannot record owner in authority runtime lock {}", path.display()))?;
        Ok(Some(Self { file }))
    }
}

impl Drop for AuthorityRuntimeGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub async fn run_authority_runtime(paths: &DataPaths, storage: &Storage, options: HostOptions) -> Result<()> {
    // Preserve the explicit EULA rejection even when the machine is under
    // pressure or player-only; consent is never a substitute for EULA.
    if !options.accept_eula {
        bail!("Minecraft server EULA acceptance is required; configure the runtime only after reviewing Mojang's EULA");
    }
    // Explicit hosting must fail immediately instead of waiting indefinitely
    // for quorum when this device is player-only or under RAM pressure.
    hosting_budget::require_contribution(paths, options.world)?;
    let identity = PeerIdentity::load_or_create(paths)?;
    let descriptor = storage.load_world_descriptor(options.world)?;
    let member_count = descriptor.members.iter().filter(|member| !member.banned).count();
    let sleep_record = launch_guard::load_sleep_record_fail_closed(storage, options.world)?;
    if member_count > 1 {
        if sleep_record.is_some() {
            bail!("multi-member sleeping worlds must be woken through the migration supervisor");
        }
        let epoch = storage.load_epoch_record(options.world)?;
        if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {
            bail!("local peer does not hold the accepted authority generation");
        }
        let config = RuntimeLaunchConfig {
            java: options.java.clone(),
            server_jar: options.server_jar.clone(),
            mod_jar: options.mod_jar.clone(),
            accept_eula: options.accept_eula,
            game_endpoint: None,
        };
        if !wait_until_launch_safe(paths, storage, &identity, options.world, &descriptor, &epoch, &config).await? {
            bail!("authority generation changed while waiting for launch safety");
        }
    }
    // Fail fast when another authority runtime (Desktop launch, CLI launch, or
    // the daemon supervisor) already owns this world's hosting slot. Racing
    // here would reset the live runtime directory underneath a running server.
    let _authority_slot = AuthorityRuntimeGuard::acquire(paths, options.world)?;
    run_authority_runtime_inner(paths, storage, options, MigrationTrigger::DirectHost, None, true, None).await
}

async fn run_authority_runtime_inner(
    paths: &DataPaths,
    storage: &Storage,
    options: HostOptions,
    trigger: MigrationTrigger,
    game_endpoint: Option<String>,
    handle_shutdown_signal: bool,
    runtime_registry: Option<&LiveFabricRuntimeRegistry>,
) -> Result<()> {
    if !options.accept_eula {
        bail!("Minecraft server EULA acceptance is required; configure the runtime only after reviewing Mojang's EULA");
    }
    // This admission must happen before authority epoch mutation, runtime reset,
    // snapshot restore, or Java spawn for ALL launch triggers.
    let admission = hosting_budget::admit(paths, options.world)?;
    let identity = PeerIdentity::load_or_create(paths)?;
    let metadata = storage.load_world(options.world)?;
    let initial = storage.latest_snapshot(options.world)?.context("world has no verified snapshot to host")?;
    storage.verify_snapshot(&initial)?;
    verify_snapshot_signature(&initial)?;

    publish_status(
        paths,
        storage,
        options.world,
        Some(trigger),
        MigrationPhase::SelectingSnapshot,
        false,
        game_endpoint.clone(),
        Some(initial.manifest_hash()?),
        None,
    )?;
    // The initial budget admission predates snapshot lookup. Recheck under
    // the same consent-write lock used by local opt-out before mutating the
    // signed authority epoch, then independently recheck before Java spawn.
    let epoch = hosting_budget::with_consent_guarded_epoch_transition(paths, options.world, &admission, || {
        prepare_authority_epoch(storage, &identity, options.world, &initial)
    })?;
    let latest = storage.latest_snapshot(options.world)?.context("authority transition lost its canonical snapshot")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;
    ensure_authority_generation(storage, &identity, &epoch)?;
    let world_config =
        storage.load_world_config(options.world).context("canonical runtime profile is not synchronized")?;
    validate_runtime_selection(
        &world_config.compatibility.minecraft_version,
        &world_config.compatibility.loader_version,
        None,
    )
    .context("canonical runtime profile is outside the shipped SwarmCraft Fabric adapter contract")?;
    let mod_readiness = server_mods::evaluate_world_mods(paths, options.world, &world_config.compatibility)?;
    if !mod_readiness.ready {
        let details = mod_readiness.issues.iter().map(|issue| issue.message.as_str()).collect::<Vec<_>>().join("; ");
        bail!("local device is not server-mod ready for authority runtime: {details}");
    }

    let runtime = paths.root.join("runtime").join(options.world.to_hex());
    let world_dir = runtime.join("world");
    publish_status(
        paths,
        storage,
        options.world,
        Some(trigger),
        MigrationPhase::PreparingRuntime,
        false,
        game_endpoint.clone(),
        Some(latest.manifest_hash()?),
        None,
    )?;
    // This persistent ownership record survives a hard-killed Rust supervisor.
    // A subsequent launcher must prove the old Java PID is gone before any runtime bytes are reset.
    // Every fresh attempt invalidates the preceding machine-local Fabric
    // verification, including when an orphaned Java PID blocks the reset.
    // A prior launch can never certify a newly restoring Minecraft process.
    host_readiness::invalidate_runtime_verification(paths, options.world)?;
    let mut runtime_process = RuntimeProcessGuard::begin(paths, options.world)?;
    reset_runtime_directory(&runtime)?;
    fs::create_dir_all(runtime.join("mods"))?;
    fs::copy(&options.mod_jar, runtime.join("mods/swarmcraft-fabric.jar"))
        .with_context(|| format!("cannot install Fabric bridge from {}", options.mod_jar.display()))?;
    server_mods::install_verified_user_mods(paths, options.world, &world_config.compatibility, &runtime.join("mods"))?;
    fs::write(runtime.join("eula.txt"), "eula=true\n")?;
    // Must be written before Java launch: the default Minecraft listener is
    // 0.0.0.0:25565 and must NEVER be reachable outside SwarmCraft's signed
    // authenticated game-stream route. The saved port is not a live proof.
    let managed_port = managed_server_properties::prepare(paths, options.world, &runtime)?;
    seed_fabric_game_jar(paths, options.world, &metadata.genesis.minecraft_version, &runtime)?;

    publish_status(
        paths,
        storage,
        options.world,
        Some(trigger),
        MigrationPhase::RestoringWorld,
        false,
        game_endpoint.clone(),
        Some(latest.manifest_hash()?),
        None,
    )?;
    storage.restore_snapshot(&latest, &world_dir)?;
    ensure_authority_generation(storage, &identity, &epoch)?;

    let listener = FabricBridgeListener::bind().await?;
    let launch = listener.launch_config()?;
    publish_status(
        paths,
        storage,
        options.world,
        Some(trigger),
        MigrationPhase::LaunchingRuntime,
        false,
        game_endpoint.clone(),
        Some(latest.manifest_hash()?),
        None,
    )?;
    // Restoring a large snapshot can take time; opt-out and resource
    // pressure must be rechecked immediately before Java starts. Serialize
    // consent publication with the synchronous spawn to close that race.
    let mut child = hosting_budget::with_consent_guarded_spawn(paths, options.world, &admission, || {
        // Do not launch a server into an occupied saved address or fall back
        // to a fresh port that would silently redirect an existing world.
        managed_server_properties::ensure_port_free(managed_port)?;
        launch_server(
            &options.java,
            &options.server_jar,
            admission.heap_mib(),
            &runtime,
            &world_dir,
            metadata.genesis.compatibility_fingerprint,
            &launch.environment(),
        )
    })?;
    // Any failed publish, verification, or IPC handshake before Ready must
    // reap this Java process. Dropping std::process::Child alone leaves the
    // unverified server running without its Rust supervisor.
    let startup = StartupJavaGuard::new(&mut child, paths, options.world);
    runtime_process.record_java(startup.pid())?;

    publish_status(
        paths,
        storage,
        options.world,
        Some(trigger),
        MigrationPhase::VerifyingFabric,
        false,
        game_endpoint.clone(),
        Some(latest.manifest_hash()?),
        None,
    )?;
    let mut session = listener.accept(FABRIC_START_TIMEOUT).await?;
    validate_world_info(
        session.world_info(),
        &metadata.genesis.minecraft_version,
        &metadata.genesis.fabric_loader_version,
        &world_dir,
        metadata.genesis.compatibility_fingerprint,
    )?;
    // SERVER_STARTED sends authenticated Fabric world info only after Java's
    // Minecraft listener has initialized. Independently reject an unavailable
    // local port before recording host-ready status. This does not replace
    // Agent 1's nonce-bound runtime/authority proof used by game routing.
    if let Err(error) = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, managed_port).into(),
        Duration::from_secs(2),
    ) {
        return Err(anyhow!("managed Minecraft server listener on 127.0.0.1:{managed_port} is unavailable: {error}"));
    }
    if let Err(error) = ensure_authority_generation(storage, &identity, &epoch) {
        terminate_child(&mut *startup.child);
        publish_status(
            paths,
            storage,
            options.world,
            Some(trigger),
            MigrationPhase::Superseded,
            false,
            game_endpoint.clone(),
            Some(latest.manifest_hash()?),
            Some(error.to_string()),
        )?;
        return Err(error);
    }
    // A runtime becomes host-ready only after the actual Fabric process has
    // launched and reported the exact world compatibility fingerprint. Persist
    // that machine-local proof so another peer can distinguish configured paths
    // from a runtime that has truly passed launch verification.
    let verified_config = RuntimeLaunchConfig {
        java: options.java.clone(),
        server_jar: options.server_jar.clone(),
        mod_jar: options.mod_jar.clone(),
        accept_eula: options.accept_eula,
        game_endpoint: game_endpoint.clone(),
    };
    let admission_state = hosting_budget::runtime_withdrawal(paths, options.world, admission.consent());
    match admission_state {
        Ok(None) => {
            host_readiness::record_runtime_verified(
                paths,
                options.world,
                &verified_config,
                metadata.genesis.compatibility_fingerprint,
            )?;
            publish_status(
                paths,
                storage,
                options.world,
                Some(trigger),
                MigrationPhase::Ready,
                true,
                game_endpoint.clone(),
                Some(latest.manifest_hash()?),
                None,
            )?;
            info!(world = %options.world, pid = startup.pid(), ?trigger, "Minecraft authority runtime is ready");
        }
        state => {
            // Never advertise Ready after consent has changed during Fabric
            // verification. The exit loop requests a safe Fabric checkpoint.
            let detail = match state {
                Ok(Some(reason)) => reason,
                Err(error) => format!("cannot verify hosting contribution: {error}"),
                Ok(None) => unreachable!(),
            };
            if let Err(error) = publish_status(
                paths,
                storage,
                options.world,
                Some(trigger),
                MigrationPhase::Blocked,
                false,
                game_endpoint.clone(),
                Some(latest.manifest_hash()?),
                Some(detail),
            ) {
                // Consent was withdrawn during authenticated Fabric startup.
                // Do not abandon the newly running JVM because telemetry
                // failed; the supervised loop must still request a Fabric
                // save barrier instead of killing uncheckpointed world data.
                warn!(world = %options.world, %error, "could not publish blocked startup state; continuing safe-stop supervision");
            }
        }
    }
    // Only this live supervisor can register the actual authenticated socket
    // and Child handle. Registration is never a proof of signed authority; the
    // challenge handler below rechecks the permit, generation, consent and PID.
    let mut registration = None;
    let mut challenge_commands = None;
    if let Some(registry) = runtime_registry {
        let identity = LiveRuntimeIdentity {
            world: options.world,
            pid: startup.pid(),
            generation: AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token },
            fingerprint: metadata.genesis.compatibility_fingerprint,
            managed_port,
        };
        match registry.register(identity) {
            Ok((guard, receiver)) => {
                registration = Some(guard);
                challenge_commands = Some(receiver);
            }
            Err(error) => {
                // An already-running JVM must retain its save-first owner.
                // Refuse game-route proofs without abandoning the supervisor.
                warn!(world = %options.world, %error, "live Fabric registry refused registration; continuing supervised JVM without game-route evidence");
            }
        }
    }
    // From here the existing supervisor owns shutdown and checkpointing.
    // Loss of consent after live Fabric handshaking must remain save-first.
    startup.disarm();

    let disposition = match wait_for_runtime_exit(
        paths,
        storage,
        &identity,
        &epoch,
        &mut child,
        &mut session,
        &world_dir,
        managed_port,
        challenge_commands.as_mut(),
        RuntimeExitOptions { handle_shutdown_signal, admission: &admission },
    )
    .await
    {
        Ok(disposition) => disposition,
        Err(error) => {
            terminate_child(&mut child);
            return Err(error);
        }
    };
    // From this point onward there is NO owned live game runtime. Revocation
    // precedes checkpointing, transfer, or publishing any final saved state.
    drop(registration);

    publish_status(
        paths,
        storage,
        options.world,
        Some(match disposition {
            RuntimeDisposition::Transfer(_) => MigrationTrigger::ManualTransfer,
            _ => trigger,
        }),
        MigrationPhase::Checkpointing,
        false,
        game_endpoint.clone(),
        Some(latest.manifest_hash()?),
        None,
    )?;
    ensure_authority_generation(storage, &identity, &epoch)?;
    let expected_head = storage.canonical_snapshot_head(options.world)?.head;
    let number = storage.next_snapshot_number(options.world)?;
    let previous_hash = Some(latest.manifest_hash()?);
    let mut final_manifest = storage.snapshot_directory(
        &world_dir,
        SnapshotContext {
            world: options.world,
            snapshot_number: number,
            epoch: epoch.epoch_number,
            sequence: latest.sequence.checked_add(1).context("snapshot sequence counter exhausted")?,
            previous_snapshot_hash: previous_hash,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
        },
    )?;
    ensure_authority_generation(storage, &identity, &epoch)?;
    identity.sign_snapshot(&mut final_manifest)?;
    storage.commit_snapshot_fenced(
        &final_manifest,
        swarm_storage::SnapshotCommitFence {
            expected_epoch: epoch.epoch_number,
            expected_fencing_token: epoch.fencing_token,
            expected_head,
        },
    )?;

    match disposition {
        RuntimeDisposition::Transfer(target) => {
            let transfer =
                create_prepared_transfer(storage, &identity, options.world, target, &epoch, &final_manifest)?;
            storage.save_transfer_record(&transfer)?;
            clear_transfer_intent(paths, options.world)?;
            publish_status(
                paths,
                storage,
                options.world,
                Some(MigrationTrigger::ManualTransfer),
                MigrationPhase::AwaitingTransferAcceptance,
                false,
                game_endpoint,
                Some(final_manifest.manifest_hash()?),
                None,
            )?;
        }
        RuntimeDisposition::Sleep => {
            let mut sleep_record = SleepRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: options.world,
                latest_snapshot_hash: final_manifest.manifest_hash()?,
                epoch: epoch.epoch_number,
                fencing_token: epoch.fencing_token,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            identity.sign_sleep_record(&mut sleep_record)?;
            storage.save_sleep_record(&sleep_record)?;
            clear_wake_intent(paths, options.world)?;
            publish_status(
                paths,
                storage,
                options.world,
                Some(trigger),
                MigrationPhase::Sleeping,
                false,
                game_endpoint,
                Some(final_manifest.manifest_hash()?),
                None,
            )?;
            info!(world = %options.world, snapshot = final_manifest.snapshot_number, "world committed and sleeping");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum RuntimeDisposition {
    Sleep,
    Transfer(PeerId),
}

struct RuntimeExitOptions<'a> {
    handle_shutdown_signal: bool,
    admission: &'a hosting_budget::HostingAdmission,
}

// Each argument is an independently authenticated runtime/lifecycle input;
// do not collapse the Fabric ownership proof into cached status data.
#[allow(clippy::too_many_arguments)]
async fn wait_for_runtime_exit(
    paths: &DataPaths,
    storage: &Storage,
    identity: &PeerIdentity,
    epoch: &EpochRecordV1,
    child: &mut Child,
    session: &mut swarm_ipc::FabricSession,
    world_directory: &Path,
    managed_port: u16,
    mut challenge_commands: Option<&mut mpsc::Receiver<RuntimeChallengeCommand>>,
    options: RuntimeExitOptions<'_>,
) -> Result<RuntimeDisposition> {
    let mut withdrawal_reported = false;
    let mut invalid_stop_intent_reported = false;
    let mut permit_watch =
        PermitWatch::new(AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token });
    loop {
        // Observe the live majority-backed heartbeat continuously. The first
        // observation alone is never authority: only a strictly advancing
        // heartbeat becomes fresh, and every challenge still rechecks it.
        // A corrupt/missing permit resets the watch and makes proof deny.
        let _ = permit_watch.observe(paths, epoch.world_id, Instant::now());
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                if let Err(error) = retire_live_runtime(
                    paths,
                    epoch.world_id,
                    MigrationPhase::Failed,
                    Some(&format!("Minecraft server exited unsuccessfully: {status}")),
                ) {
                    warn!(world = %epoch.world_id, %error, "could not invalidate stale readiness after Java exit");
                }
                bail!("Minecraft server exited unsuccessfully: {status}");
            }
            if let Err(error) = retire_live_runtime(paths, epoch.world_id, MigrationPhase::Checkpointing, None) {
                warn!(world = %epoch.world_id, %error, "could not invalidate readiness after Java exited");
            }
            return Ok(RuntimeDisposition::Sleep);
        }
        if let Err(error) = ensure_authority_generation(storage, identity, epoch) {
            if let Err(report_error) = retire_live_runtime(
                paths,
                epoch.world_id,
                MigrationPhase::Superseded,
                Some("runtime authority generation was superseded"),
            ) {
                warn!(world = %epoch.world_id, %report_error, "could not invalidate superseded runtime status");
            }
            return Err(error);
        }
        let stop_requested = match load_stop_intent(paths, epoch) {
            Ok(requested) => {
                invalid_stop_intent_reported = false;
                requested
            }
            Err(error) => {
                if !invalid_stop_intent_reported {
                    warn!(world = %epoch.world_id, %error, "invalid or stale safe-stop intent; refusing to stop this authority generation");
                    invalid_stop_intent_reported = true;
                }
                false
            }
        };
        if stop_requested {
            if let Err(error) = retire_live_runtime(paths, epoch.world_id, MigrationPhase::Checkpointing, None) {
                warn!(world = %epoch.world_id, %error, "could not publish safe-stop checkpointing state");
            }
            let stop_result = async {
                session.prepare_shutdown(1, FABRIC_SHUTDOWN_TIMEOUT).await?;
                timeout(FABRIC_SHUTDOWN_TIMEOUT, wait_for_child(child))
                    .await
                    .map_err(|_| anyhow!("Minecraft did not stop after the shutdown save barrier"))??;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            if let Err(error) = clear_stop_intent(paths, epoch.world_id) {
                // An inability to clear telemetry/intent must never kill the
                // unfenced Java process or discard an already saved checkpoint.
                warn!(world = %epoch.world_id, %error, "cannot clear safe-stop intent");
            }
            match stop_result {
                Ok(()) => return Ok(RuntimeDisposition::Sleep),
                Err(error) => {
                    if let Err(report_error) = retire_live_runtime(
                        paths,
                        epoch.world_id,
                        MigrationPhase::Blocked,
                        Some("safe-stop save barrier failed; Java remains running until it can be stopped safely"),
                    ) {
                        warn!(world = %epoch.world_id, %report_error, "could not publish failed safe-stop state");
                    }
                    warn!(world = %epoch.world_id, %error, "safe stop save barrier failed; keeping Minecraft running");
                    continue;
                }
            }
        }
        // Prefer any already-requested signed transfer checkpoint. A withdrawn
        // contribution otherwise requests the same Fabric save barrier used by
        // a safe user stop; a failed barrier does NOT kill an unfenced JVM.
        match load_transfer_intent(paths, epoch) {
            Ok(Some(target)) => match validate_transfer_target(storage, identity, epoch.world_id, target) {
                Ok(()) => {
                    if let Err(error) = retire_live_runtime(paths, epoch.world_id, MigrationPhase::Checkpointing, None)
                    {
                        warn!(world = %epoch.world_id, %error, "could not publish transfer checkpointing state");
                    }
                    let outcome = async {
                        session.prepare_shutdown(2, FABRIC_SHUTDOWN_TIMEOUT).await?;
                        timeout(FABRIC_SHUTDOWN_TIMEOUT, wait_for_child(child))
                            .await
                            .map_err(|_| anyhow!("Minecraft did not stop after the transfer save barrier"))??;
                        Ok::<(), anyhow::Error>(())
                    }
                    .await;
                    match outcome {
                        Ok(()) => return Ok(RuntimeDisposition::Transfer(target)),
                        Err(error) => {
                            if let Err(report_error) = retire_live_runtime(
                                paths,
                                epoch.world_id,
                                MigrationPhase::Blocked,
                                Some("transfer save barrier failed; current Java authority remains running"),
                            ) {
                                warn!(world = %epoch.world_id, %report_error, "could not publish failed transfer state");
                            }
                            warn!(
                                world = %epoch.world_id,
                                %error,
                                "transfer save barrier failed; keeping Minecraft running rather than terminating an uncheckpointed authority"
                            );
                        }
                    }
                }
                Err(error) => {
                    warn!(
                        world = %epoch.world_id,
                        %error,
                        "invalid manual transfer target; preserving running authority and removing invalid local intent"
                    );
                    if let Err(clear_error) = clear_transfer_intent(paths, epoch.world_id) {
                        warn!(world = %epoch.world_id, %clear_error, "cannot clear rejected local transfer intent");
                    }
                }
            },
            Ok(None) => {}
            Err(error) => {
                warn!(world = %epoch.world_id, %error, "rejecting stale or malformed transfer intent without stopping Minecraft");
                if let Err(clear_error) = clear_transfer_intent(paths, epoch.world_id) {
                    warn!(world = %epoch.world_id, %clear_error, "cannot clear rejected transfer intent");
                }
            }
        }
        let withdrawal = match hosting_budget::runtime_withdrawal(paths, epoch.world_id, options.admission.consent()) {
            Ok(reason) => reason,
            Err(error) => Some(format!("cannot revalidate local hosting consent or memory: {error}")),
        };
        if let Some(reason) = withdrawal {
            if !withdrawal_reported {
                match retire_live_runtime(paths, epoch.world_id, MigrationPhase::Checkpointing, Some(&reason)) {
                    Ok(()) => withdrawal_reported = true,
                    Err(error) => {
                        warn!(world = %epoch.world_id, %error, "could not invalidate runtime readiness on hosting withdrawal");
                    }
                }
            }
            warn!(world = %epoch.world_id, %reason, "hosting contribution withdrawn or unsafe; requesting durable safe stop");
            let stopped = async {
                session.prepare_shutdown(1, FABRIC_SHUTDOWN_TIMEOUT).await?;
                timeout(FABRIC_SHUTDOWN_TIMEOUT, wait_for_child(child))
                    .await
                    .map_err(|_| anyhow!("Minecraft did not stop after hosting withdrawal save barrier"))??;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            match stopped {
                Ok(()) => return Ok(RuntimeDisposition::Sleep),
                Err(error) => {
                    if let Err(report_error) = retire_live_runtime(
                        paths,
                        epoch.world_id,
                        MigrationPhase::Blocked,
                        Some("hosting withdrawal save barrier failed; Java remains running while shutdown retries"),
                    ) {
                        warn!(world = %epoch.world_id, %report_error, "could not publish failed hosting-withdrawal state");
                    }
                    warn!(world = %epoch.world_id, %error, "cannot safely checkpoint withdrawn host; preserving running authority and retrying")
                }
            }
        }
        if options.handle_shutdown_signal {
            tokio::select! {
                _ = sleep(SUPERVISOR_POLL) => {}
                result = tokio::signal::ctrl_c() => {
                    result.context("failed to listen for Ctrl+C")?;
                    if let Err(error) = retire_live_runtime(paths, epoch.world_id, MigrationPhase::Checkpointing, None) {
                        warn!(world = %epoch.world_id, %error, "could not publish Ctrl+C checkpointing state");
                    }
                    let stop_result = async {
                        session.prepare_shutdown(1, FABRIC_SHUTDOWN_TIMEOUT).await?;
                        timeout(FABRIC_SHUTDOWN_TIMEOUT, wait_for_child(child))
                            .await
                            .map_err(|_| anyhow!("Minecraft did not stop after the shutdown barrier"))??;
                        Ok::<(), anyhow::Error>(())
                    }.await;
                    match stop_result {
                        Ok(()) => return Ok(RuntimeDisposition::Sleep),
                        Err(error) => {
                            if let Err(report_error) = retire_live_runtime(
                                paths,
                                epoch.world_id,
                                MigrationPhase::Blocked,
                                Some("Ctrl+C save barrier failed; Java remains running until it can stop safely"),
                            ) {
                                warn!(world = %epoch.world_id, %report_error, "could not publish failed Ctrl+C stop state");
                            }
                            warn!(world = %epoch.world_id, %error, "Ctrl+C save barrier failed; preserving Java and allowing a later safe-stop retry");
                        }
                    }
                }
            }
        } else if let Some(commands) = challenge_commands.as_mut() {
            tokio::select! {
                request = commands.recv() => {
                    if let Some(request) = request {
                        let reply = challenge_live_supervised_runtime(
                            paths, storage, identity, epoch, child, session,
                            world_directory, managed_port, options.admission,
                            &mut permit_watch, request.request_id, request.nonce,
                        ).await;
                        let _ = request.response.send(reply);
                    }
                }
                _ = sleep(SUPERVISOR_POLL) => {}
            }
        } else {
            sleep(SUPERVISOR_POLL).await;
        }
    }
}

/// Runs ONLY in the task that owns the managed JVM and original authenticated
/// FabricSession. Never use a cached Ready, an unbound PID or a new socket.
#[allow(clippy::too_many_arguments)]
async fn challenge_live_supervised_runtime(
    paths: &DataPaths,
    storage: &Storage,
    identity: &PeerIdentity,
    epoch: &EpochRecordV1,
    child: &mut Child,
    session: &mut swarm_ipc::FabricSession,
    world_directory: &Path,
    managed_port: u16,
    admission: &hosting_budget::HostingAdmission,
    permit_watch: &mut PermitWatch,
    request_id: u64,
    nonce: [u8; 32],
) -> Result<crate::live_fabric_readiness::VerifiedFabricRuntime> {
    if child.try_wait()?.is_some() {
        bail!("managed Minecraft JVM has already exited");
    }
    ensure_authority_generation(storage, identity, epoch)?;
    if hosting_budget::runtime_withdrawal(paths, epoch.world_id, admission.consent())?.is_some() {
        bail!("hosting consent or memory admission was withdrawn");
    }
    if load_stop_intent(paths, epoch)? || load_transfer_intent(paths, epoch)?.is_some() {
        bail!("runtime is stopping or transferring; no new game sessions");
    }
    if !permit_watch.observe(paths, epoch.world_id, Instant::now())? {
        bail!("majority-backed authority permit has not been observed advancing live");
    }
    let expected = ExpectedFabricRuntime {
        world_id: epoch.world_id,
        world_directory,
        compatibility_fingerprint: session.world_info().compatibility_fingerprint,
        managed_loopback_port: managed_port,
        request_id,
        challenge_nonce: nonce,
    };
    let verified = challenge_authenticated_session(session, &expected, Duration::from_millis(1_200)).await?;
    if child.try_wait()?.is_some() {
        bail!("managed Minecraft process exited during Fabric challenge");
    }
    ensure_authority_generation(storage, identity, epoch)?;
    if hosting_budget::runtime_withdrawal(paths, epoch.world_id, admission.consent())?.is_some()
        || load_stop_intent(paths, epoch)?
        || load_transfer_intent(paths, epoch)?.is_some()
        || !permit_watch.observe(paths, epoch.world_id, Instant::now())?
    {
        bail!("hosting consent, permit or stop state changed during Fabric challenge");
    }
    Ok(verified)
}

/// Denial-only last-mile check for responses queued while the signed authority
/// was stopping or preparing a transfer. Never grants permission by itself.
pub fn no_local_route_transition_pending(paths: &DataPaths, epoch: &EpochRecordV1) -> Result<bool> {
    Ok(!load_stop_intent(paths, epoch)? && load_transfer_intent(paths, epoch)?.is_none())
}

pub fn prepare_manual_transfer(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    target: PeerId,
) -> Result<TransferPrepareResult> {
    prepare_transfer(paths, storage, world, target, None)
}

fn prepare_transfer(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    target: PeerId,
    expected_epoch: Option<u64>,
) -> Result<TransferPrepareResult> {
    storage.load_world(world)?;
    // Serialize foreground requests across Desktop/CLI processes. This lock
    // covers target selection, not the long-running save or network transfer.
    let control = control_dir(paths, world);
    prepare_control_parent(&control)?;
    let _request_lock = lock_control_file(&control.join("transfer-request.lock"))?;
    let identity = PeerIdentity::load_or_create(paths)?;
    let epoch = storage.load_epoch_record(world)?;
    if expected_epoch.is_some() {
        verify_signature(
            epoch.authority_peer_id,
            epoch.authority_public_key,
            &epoch.signing_bytes()?,
            &epoch.signature,
        )?;
    }
    ensure_authority_generation(storage, &identity, &epoch)?;
    if expected_epoch.is_some_and(|expected| expected != epoch.epoch_number) {
        bail!("authority generation changed; reopen the transfer wizard");
    }
    validate_transfer_target(storage, &identity, world, target)?;
    let latest = storage.latest_snapshot(world)?.context("cannot transfer authority without a canonical snapshot")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;

    // Read the intent before the record: the supervisor publishes the signed
    // prepared record before clearing the intent. A retry cannot overwrite a
    // checkpoint already in flight for a different target.
    let pending_target = load_transfer_intent(paths, &epoch)?;
    if let Some(existing) = active_transfer_for_current_generation(storage, world)? {
        if existing.from_peer_id == identity.peer_id()
            && existing.to_peer_id == target
            && (existing.phase == TransferPhase::Prepared || expected_epoch.is_some())
        {
            return Ok(TransferPrepareResult::Prepared(encode_transfer(&existing)?));
        }
        bail!("another authority transfer is already active for this authority generation");
    }
    if let Some(pending_target) = pending_target {
        if pending_target == target {
            return Ok(TransferPrepareResult::CheckpointRequested);
        }
        bail!("a checkpoint for another transfer target is already requested");
    }

    if let Some(sleep_record) = launch_guard::load_sleep_record_fail_closed(storage, world)? {
        if sleep_record.latest_snapshot_hash != latest.manifest_hash()?
            || sleep_record.epoch != epoch.epoch_number
            || sleep_record.fencing_token != epoch.fencing_token
        {
            bail!("sleep state is not aligned with the accepted authority generation");
        }
        let prepared = create_prepared_transfer(storage, &identity, world, target, &epoch, &latest)?;
        storage.save_transfer_record(&prepared)?;
        publish_status(
            paths,
            storage,
            world,
            Some(MigrationTrigger::ManualTransfer),
            MigrationPhase::AwaitingTransferAcceptance,
            false,
            None,
            Some(latest.manifest_hash()?),
            None,
        )?;
        return Ok(TransferPrepareResult::Prepared(encode_transfer(&prepared)?));
    }

    // A transfer begins at the current authority, but no cached Ready proof
    // may remain usable once a checkpoint handoff has been requested.
    host_readiness::invalidate_runtime_verification(paths, world)?;
    host_readiness::invalidate_cached_readiness(paths, world)?;
    atomic_json(&transfer_intent_path(paths, world), &TransferIntentV1::from_epoch(&epoch, target))?;
    publish_status(
        paths,
        storage,
        world,
        Some(MigrationTrigger::ManualTransfer),
        MigrationPhase::Checkpointing,
        false,
        None,
        Some(latest.manifest_hash()?),
        None,
    )?;
    Ok(TransferPrepareResult::CheckpointRequested)
}

pub fn export_transfer(storage: &Storage, world: WorldId) -> Result<String> {
    encode_transfer(&storage.load_transfer_record(world)?)
}

pub fn accept_manual_transfer(paths: &DataPaths, storage: &Storage, world: WorldId, token: &str) -> Result<String> {
    let consent = hosting_budget::require_contribution(paths, world)?;
    let identity = PeerIdentity::load_or_create(paths)?;
    let prepared = decode_transfer(token)?;
    if prepared.world_id != world || prepared.phase != TransferPhase::Prepared {
        bail!("transfer token is not a prepared transfer for this world");
    }
    validate_transfer_record(storage, &prepared)?;
    if prepared.to_peer_id != identity.peer_id() {
        bail!("only the prepared target may accept this transfer");
    }
    let (accepted, was_saved) = hosting_budget::with_consent_guarded_operation(paths, world, &consent, || {
        if let Some(existing) = active_transfer_for_current_generation(storage, world)? {
            ensure_same_transfer(&existing, &prepared)?;
            match existing.phase {
                TransferPhase::Prepared => {}
                TransferPhase::Accepted => return Ok((existing, false)),
                TransferPhase::Committed => bail!("transfer is already committed and cannot be accepted again"),
            }
        }
        let mut accepted = prepared.clone();
        accepted.phase = TransferPhase::Accepted;
        identity.sign_transfer(&mut accepted)?;
        storage.save_transfer_record(&prepared)?;
        storage.save_transfer_record(&accepted)?;
        Ok((accepted, true))
    })?;
    if !was_saved {
        return encode_transfer(&accepted);
    }
    publish_status(
        paths,
        storage,
        world,
        Some(MigrationTrigger::ManualTransfer),
        MigrationPhase::WaitingForAuthority,
        false,
        None,
        Some(accepted.base_snapshot_hash),
        None,
    )?;
    encode_transfer(&accepted)
}

pub fn commit_manual_transfer(paths: &DataPaths, storage: &Storage, world: WorldId, token: &str) -> Result<String> {
    let identity = PeerIdentity::load_or_create(paths)?;
    let accepted = decode_transfer(token)?;
    if accepted.world_id != world || accepted.phase != TransferPhase::Accepted {
        bail!("transfer token is not an accepted transfer for this world");
    }
    validate_transfer_record(storage, &accepted)?;
    if accepted.from_peer_id != identity.peer_id() {
        bail!("only the current authority may commit this transfer");
    }
    if let Some(previous) = active_transfer_for_current_generation(storage, world)? {
        ensure_same_transfer(&previous, &accepted)?;
        match previous.phase {
            TransferPhase::Prepared | TransferPhase::Accepted => {}
            TransferPhase::Committed => return encode_transfer(&previous),
        }
    }
    storage.save_transfer_record(&accepted)?;
    let mut committed = accepted.clone();
    committed.phase = TransferPhase::Committed;
    identity.sign_transfer(&mut committed)?;
    storage.save_transfer_record(&committed)?;
    publish_status(
        paths,
        storage,
        world,
        Some(MigrationTrigger::ManualTransfer),
        MigrationPhase::AwaitingTransferAcceptance,
        false,
        None,
        Some(committed.base_snapshot_hash),
        None,
    )?;
    encode_transfer(&committed)
}

pub fn activate_manual_transfer(paths: &DataPaths, storage: &Storage, world: WorldId, token: &str) -> Result<String> {
    let consent = hosting_budget::require_contribution(paths, world)?;
    let identity = PeerIdentity::load_or_create(paths)?;
    let committed = decode_transfer(token)?;
    if committed.world_id != world || committed.phase != TransferPhase::Committed {
        bail!("transfer token is not a committed transfer for this world");
    }
    validate_transfer_record(storage, &committed)?;
    if committed.to_peer_id != identity.peer_id() {
        bail!("only the committed target may activate this transfer");
    }
    if let Some(existing) = active_transfer_for_current_generation(storage, world)? {
        ensure_same_transfer(&existing, &committed)?;
        if existing.phase == TransferPhase::Prepared {
            bail!("target has not durably accepted this transfer");
        }
    }
    let current = storage.load_epoch_record(world)?;
    let expected_generation = AuthorityGeneration { epoch: current.epoch_number, fencing_token: current.fencing_token }
        .checked_next()
        .context("accepted authority generation is exhausted")?;
    if current.authority_peer_id != committed.from_peer_id
        || expected_generation.epoch != committed.next_epoch
        || expected_generation.fencing_token != committed.next_fencing_token
    {
        bail!("committed transfer no longer extends the accepted authority generation");
    }
    let latest = storage.latest_snapshot(world)?.context("transfer target lacks the canonical snapshot")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;
    if latest.manifest_hash()? != committed.base_snapshot_hash {
        bail!("transfer target lacks the exact canonical checkpoint");
    }
    let mut next = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: committed.next_epoch,
        previous_epoch_hash: Some(epoch_record_hash(&current)?),
        base_state_hash: latest.state_root,
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: committed.next_fencing_token,
        reason: "manual authority transfer committed after final canonical checkpoint".into(),
        signature: Vec::new(),
    };
    next.signature = identity.sign(&next.signing_bytes()?);
    hosting_budget::with_consent_guarded_operation(paths, world, &consent, || {
        storage.save_transfer_record(&committed)?;
        storage.save_epoch_record(&next)?;
        storage.clear_sleep_record(world)?;
        Ok(())
    })?;
    publish_status(
        paths,
        storage,
        world,
        Some(MigrationTrigger::ManualTransfer),
        MigrationPhase::WaitingForQuorum,
        false,
        None,
        Some(committed.base_snapshot_hash),
        None,
    )?;
    encode_epoch(&next)
}

pub fn observe_manual_transfer_epoch(paths: &DataPaths, storage: &Storage, world: WorldId, token: &str) -> Result<()> {
    let identity = PeerIdentity::load_or_create(paths)?;
    let next = decode_epoch(token)?;
    if next.world_id != world || next.mode == EpochMode::Recovery {
        bail!("epoch token is not a manual authority transition for this world");
    }
    verify_signature(next.authority_peer_id, next.authority_public_key, &next.signing_bytes()?, &next.signature)?;
    let current = storage.load_epoch_record(world)?;
    let transfer = storage.load_transfer_record(world).context("manual epoch is missing its committed transfer")?;
    validate_transfer_record_against_epoch(storage, &transfer, &current)?;
    let expected_generation = AuthorityGeneration { epoch: current.epoch_number, fencing_token: current.fencing_token }
        .checked_next()
        .context("accepted authority generation is exhausted")?;
    if transfer.phase != TransferPhase::Committed
        || transfer.from_peer_id != current.authority_peer_id
        || transfer.to_peer_id != next.authority_peer_id
        || transfer.next_epoch != next.epoch_number
        || transfer.next_fencing_token != next.fencing_token
        || next.epoch_number != expected_generation.epoch
        || next.fencing_token != expected_generation.fencing_token
        || next.previous_epoch_hash != Some(epoch_record_hash(&current)?)
    {
        bail!("manual epoch does not exactly extend the committed transfer");
    }
    let descriptor = storage.load_world_descriptor(world)?;
    let target = descriptor.member(next.authority_peer_id).context("manual epoch authority is not a member")?;
    if target.banned || !target.authority_eligible || target.public_key != next.authority_public_key {
        bail!("manual epoch authority is not eligible or its key changed");
    }
    let latest = storage.latest_snapshot(world)?.context("manual epoch has no canonical base snapshot")?;
    if latest.state_root != next.base_state_hash || latest.manifest_hash()? != transfer.base_snapshot_hash {
        bail!("manual epoch does not use the committed canonical checkpoint");
    }
    storage.save_epoch_record(&next)?;
    storage.clear_sleep_record(world)?;
    if identity.peer_id() == current.authority_peer_id {
        publish_status(
            paths,
            storage,
            world,
            Some(MigrationTrigger::ManualTransfer),
            MigrationPhase::WaitingForAuthority,
            false,
            None,
            Some(transfer.base_snapshot_hash),
            None,
        )?;
    }
    Ok(())
}

fn active_transfer_for_current_generation(storage: &Storage, world: WorldId) -> Result<Option<AuthorityTransferV1>> {
    let current = match storage.load_epoch_record(world) {
        Ok(current) => current,
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let transfer = match storage.load_transfer_record(world) {
        Ok(transfer) => transfer,
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !transfer_is_source_generation(&transfer, &current) {
        return Ok(None);
    }
    validate_transfer_record_against_epoch(storage, &transfer, &current)?;
    Ok(Some(transfer))
}

fn active_outbound_transfer(
    storage: &Storage,
    world: WorldId,
    local_peer: PeerId,
) -> Result<Option<AuthorityTransferV1>> {
    Ok(active_transfer_for_current_generation(storage, world)?.filter(|transfer| transfer.from_peer_id == local_peer))
}

fn transfer_is_source_generation(transfer: &AuthorityTransferV1, current: &EpochRecordV1) -> bool {
    let Some(source_epoch) = transfer.next_epoch.checked_sub(1) else {
        return false;
    };
    let Some(source_fencing_token) = transfer.next_fencing_token.checked_sub(1) else {
        return false;
    };
    transfer.world_id == current.world_id
        && transfer.from_peer_id == current.authority_peer_id
        && source_epoch == current.epoch_number
        && source_fencing_token == current.fencing_token
}

fn transfer_is_successor_generation(
    transfer: &AuthorityTransferV1,
    current: &EpochRecordV1,
    local_peer: PeerId,
) -> bool {
    transfer.world_id == current.world_id
        && transfer.phase == TransferPhase::Committed
        && transfer.to_peer_id == local_peer
        && transfer.to_peer_id == current.authority_peer_id
        && transfer.next_epoch == current.epoch_number
        && transfer.next_fencing_token == current.fencing_token
}

fn create_prepared_transfer(
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
    target: PeerId,
    epoch: &EpochRecordV1,
    latest: &SnapshotManifestV1,
) -> Result<AuthorityTransferV1> {
    validate_transfer_target(storage, identity, world, target)?;
    let next_epoch = epoch.epoch_number.checked_add(1).context("authority epoch is exhausted")?;
    let next_fencing_token = epoch.fencing_token.checked_add(1).context("authority fencing token is exhausted")?;
    let mut transfer = AuthorityTransferV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        from_peer_id: identity.peer_id(),
        to_peer_id: target,
        base_snapshot_hash: latest.manifest_hash()?,
        next_epoch,
        next_fencing_token,
        phase: TransferPhase::Prepared,
        signer_peer_id: identity.peer_id(),
        signer_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    identity.sign_transfer(&mut transfer)?;
    Ok(transfer)
}

fn validate_transfer_target(storage: &Storage, identity: &PeerIdentity, world: WorldId, target: PeerId) -> Result<()> {
    if target == identity.peer_id() {
        bail!("manual transfer target must be a different peer");
    }
    let descriptor = storage.load_world_descriptor(world)?;
    let target_member = descriptor.member(target).context("manual transfer target is not a world member")?;
    if target_member.banned || !target_member.authority_eligible {
        bail!("manual transfer target is banned or not authority eligible");
    }
    Ok(())
}

fn validate_transfer_record(storage: &Storage, transfer: &AuthorityTransferV1) -> Result<()> {
    let current = storage.load_epoch_record(transfer.world_id)?;
    validate_transfer_record_against_epoch(storage, transfer, &current)
}

fn validate_transfer_record_against_epoch(
    storage: &Storage,
    transfer: &AuthorityTransferV1,
    current: &EpochRecordV1,
) -> Result<()> {
    verify_transfer_signature(transfer)?;
    if transfer.world_id != current.world_id {
        bail!("transfer world does not match the accepted authority generation");
    }
    let descriptor = storage.load_world_descriptor(transfer.world_id)?;
    let from = descriptor.member(transfer.from_peer_id).context("transfer source is not a world member")?;
    let to = descriptor.member(transfer.to_peer_id).context("transfer target is not a world member")?;
    if from.banned || to.banned || !to.authority_eligible {
        bail!("transfer participants are banned or target is not authority eligible");
    }
    if from.public_key != current.authority_public_key {
        bail!("transfer source key does not match the accepted authority generation");
    }
    let expected_signer = match transfer.phase {
        TransferPhase::Prepared | TransferPhase::Committed => transfer.from_peer_id,
        TransferPhase::Accepted => transfer.to_peer_id,
    };
    if transfer.signer_peer_id != expected_signer {
        bail!("transfer phase was signed by the wrong participant");
    }
    let signer = descriptor.member(transfer.signer_peer_id).context("transfer signer is not a member")?;
    if signer.public_key != transfer.signer_public_key {
        bail!("transfer signer key does not match membership");
    }
    let source_epoch =
        transfer.next_epoch.checked_sub(1).context("transfer successor epoch has no source generation")?;
    let source_fencing_token = transfer
        .next_fencing_token
        .checked_sub(1)
        .context("transfer successor fencing token has no source generation")?;
    if source_epoch != current.epoch_number
        || source_fencing_token != current.fencing_token
        || !transfer_is_source_generation(transfer, current)
    {
        bail!("transfer generation does not extend the accepted authority exactly once");
    }
    let latest = storage.latest_snapshot(transfer.world_id)?.context("transfer peer lacks the canonical snapshot")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;
    if latest.manifest_hash()? != transfer.base_snapshot_hash {
        bail!("transfer token does not reference this peer's exact canonical snapshot");
    }
    Ok(())
}

fn ensure_same_transfer(left: &AuthorityTransferV1, right: &AuthorityTransferV1) -> Result<()> {
    if left.world_id != right.world_id
        || left.from_peer_id != right.from_peer_id
        || left.to_peer_id != right.to_peer_id
        || left.base_snapshot_hash != right.base_snapshot_hash
        || left.next_epoch != right.next_epoch
        || left.next_fencing_token != right.next_fencing_token
    {
        bail!("transfer token does not continue the locally durable transfer");
    }
    Ok(())
}

fn encode_transfer(transfer: &AuthorityTransferV1) -> Result<String> {
    Ok(hex::encode(postcard::to_allocvec(transfer)?))
}

fn decode_transfer(token: &str) -> Result<AuthorityTransferV1> {
    let bytes = hex::decode(token.trim()).context("transfer token is not valid hex")?;
    postcard::from_bytes(&bytes).context("transfer token is malformed")
}

fn encode_epoch(epoch: &EpochRecordV1) -> Result<String> {
    Ok(hex::encode(postcard::to_allocvec(epoch)?))
}

fn decode_epoch(token: &str) -> Result<EpochRecordV1> {
    let bytes = hex::decode(token.trim()).context("epoch token is not valid hex")?;
    postcard::from_bytes(&bytes).context("epoch token is malformed")
}

fn prepare_authority_epoch(
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
    latest: &SnapshotManifestV1,
) -> Result<EpochRecordV1> {
    let descriptor = storage.load_world_descriptor(world)?;
    let local = descriptor.member(identity.peer_id()).context("local peer is not a member of this world")?;
    if local.banned || !local.authority_eligible || local.public_key != identity.public_key() {
        bail!("local peer is not eligible to host this world");
    }

    if let Some(sleep_record) = launch_guard::load_sleep_record_fail_closed(storage, world)? {
        if sleep_record.latest_snapshot_hash != latest.manifest_hash()? {
            bail!("local replica is stale and cannot wake the sleeping world");
        }
        let previous = storage.load_epoch_record(world)?;
        let mut next = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: previous.epoch_number.checked_add(1).context("authority epoch exhausted during wake")?,
            previous_epoch_hash: Some(epoch_record_hash(&previous)?),
            base_state_hash: latest.state_root,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            mode: EpochMode::Solo,
            fencing_token: previous
                .fencing_token
                .checked_add(1)
                .context("authority fencing token exhausted during wake")?,
            reason: "wake from durable sleep".into(),
            signature: Vec::new(),
        };
        next.signature = identity.sign(&next.signing_bytes()?);
        storage.save_epoch_record(&next)?;
        storage.clear_sleep_record(world)?;
        ensure_authority_artifacts(storage, identity, &next)?;
        return Ok(next);
    }

    match storage.load_epoch_record(world) {
        Ok(epoch) => {
            if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {
                bail!("local peer does not hold the accepted authority epoch");
            }
            ensure_authority_artifacts(storage, identity, &epoch)?;
            Ok(epoch)
        }
        Err(_)
            if latest.authority_peer_id == identity.peer_id()
                && latest.authority_public_key == identity.public_key() =>
        {
            let mut epoch = EpochRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                epoch_number: latest.epoch,
                previous_epoch_hash: None,
                base_state_hash: latest.state_root,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                mode: EpochMode::Solo,
                fencing_token: 1,
                reason: "initial preview authority".into(),
                signature: Vec::new(),
            };
            epoch.signature = identity.sign(&epoch.signing_bytes()?);
            storage.save_epoch_record(&epoch)?;
            Ok(epoch)
        }
        Err(error) => Err(error.into()),
    }
}

fn ensure_authority_artifacts(storage: &Storage, identity: &PeerIdentity, epoch: &EpochRecordV1) -> Result<()> {
    let latest = storage.latest_snapshot(epoch.world_id)?.context("accepted authority epoch has no base snapshot")?;
    if latest.epoch < epoch.epoch_number {
        if latest.epoch.checked_add(1).context("snapshot epoch counter exhausted during authority promotion")?
            != epoch.epoch_number
            || latest.state_root != epoch.base_state_hash
        {
            bail!("accepted authority epoch does not directly promote the latest canonical snapshot");
        }
        let mut promoted = SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: epoch.world_id,
            snapshot_number: storage.next_snapshot_number(epoch.world_id)?,
            epoch: epoch.epoch_number,
            sequence: latest.sequence.checked_add(1).context("snapshot sequence counter exhausted")?,
            previous_snapshot_hash: Some(latest.manifest_hash()?),
            entries: latest.entries.clone(),
            state_root: latest.state_root,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity.sign_snapshot(&mut promoted)?;
        let promoted_expected_head = storage.canonical_snapshot_head(promoted.world_id)?.head;
        storage.commit_snapshot_fenced(
            &promoted,
            swarm_storage::SnapshotCommitFence {
                expected_epoch: epoch.epoch_number,
                expected_fencing_token: epoch.fencing_token,
                expected_head: promoted_expected_head,
            },
        )?;
    } else if latest.epoch != epoch.epoch_number
        || latest.authority_peer_id != identity.peer_id()
        || latest.authority_public_key != identity.public_key()
        || latest.state_root != epoch.base_state_hash
    {
        bail!("latest snapshot conflicts with the accepted authority epoch");
    }

    if let Ok(membership) = storage.load_membership_record(epoch.world_id) {
        if membership.epoch > epoch.epoch_number {
            bail!("membership is ahead of the accepted authority epoch");
        }
        if membership.epoch != epoch.epoch_number
            || membership.authority_peer_id != identity.peer_id()
            || membership.authority_public_key != identity.public_key()
        {
            let mut promoted = MembershipRecordV1 {
                protocol_version: membership.protocol_version,
                world_id: epoch.world_id,
                epoch: epoch.epoch_number,
                sequence: membership.sequence.checked_add(1).context("membership sequence counter exhausted")?,
                previous_membership_hash: Some(membership.record_hash()?),
                members: membership.members.clone(),
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            identity.sign_membership(&mut promoted)?;
            storage.save_membership_record(&promoted)?;
        }
    }
    Ok(())
}

fn ensure_authority_generation(storage: &Storage, identity: &PeerIdentity, expected: &EpochRecordV1) -> Result<()> {
    let current = storage.load_epoch_record(expected.world_id)?;
    if current.epoch_number != expected.epoch_number
        || current.fencing_token != expected.fencing_token
        || current.authority_peer_id != identity.peer_id()
        || current.authority_public_key != identity.public_key()
    {
        bail!("authority generation changed while runtime migration was in progress");
    }
    Ok(())
}

fn launch_server(
    java: &Path,
    server_jar: &Path,
    heap_mib: u64,
    runtime: &Path,
    world_dir: &Path,
    compatibility: Hash32,
    ipc_environment: &[(String, String); 3],
) -> Result<Child> {
    let mut command = Command::new(java);
    command.arg(format!("-Xmx{heap_mib}m")).arg("-jar").arg(server_jar).arg("nogui").current_dir(runtime);
    for (name, value) in ipc_environment {
        command.env(name, value);
    }
    command
        .env("SWARMCRAFT_WORLD_DIR", world_dir.to_string_lossy().as_ref())
        .env("SWARMCRAFT_COMPAT_FINGERPRINT", compatibility.to_string());
    command.spawn().with_context(|| format!("cannot launch Java runtime {}", java.display()))
}

fn validate_world_info(
    info: &swarm_ipc::FabricWorldInfo,
    minecraft_version: &str,
    loader_version: &str,
    world_dir: &Path,
    compatibility: Hash32,
) -> Result<()> {
    if info.minecraft_version != minecraft_version {
        bail!("Fabric reported Minecraft {}, expected {}", info.minecraft_version, minecraft_version);
    }
    if loader_version != "unknown" && info.fabric_loader_version != loader_version {
        bail!("Fabric loader mismatch: {} != {}", info.fabric_loader_version, loader_version);
    }
    if Path::new(&info.world_directory) != world_dir {
        bail!("Fabric world directory does not match the restored runtime directory");
    }
    if info.compatibility_fingerprint != compatibility {
        bail!("Fabric compatibility fingerprint does not match world metadata");
    }
    Ok(())
}

fn reset_runtime_directory(runtime: &Path) -> Result<()> {
    if runtime.exists() {
        fs::remove_dir_all(runtime).with_context(|| format!("cannot reset runtime directory {}", runtime.display()))?;
    }
    fs::create_dir_all(runtime)?;
    Ok(())
}

fn managed_seed_expected_sha256(paths: &DataPaths, world: WorldId, minecraft_version: &str) -> Result<Option<String>> {
    let lock_path = crate::runtime_layout::runtime_lock_path(paths, world);
    if !lock_path.is_file() {
        return Ok(None);
    }
    let bytes =
        fs::read(&lock_path).with_context(|| format!("cannot read managed runtime lock {}", lock_path.display()))?;
    let lock: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("managed runtime lock is malformed at {}", lock_path.display()))?;
    let schema_version = lock
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .context("managed runtime lock is missing schema_version")?;
    if schema_version != u64::from(crate::runtime_layout::RUNTIME_LOCK_SCHEMA_VERSION) {
        bail!("managed runtime lock schema is incompatible with this build");
    }
    let lock_world =
        lock.get("world_id").and_then(serde_json::Value::as_str).context("managed runtime lock is missing world_id")?;
    if lock_world != world.to_string() {
        bail!("managed runtime lock belongs to a different world");
    }
    let lock_minecraft = lock
        .get("minecraft_version")
        .and_then(serde_json::Value::as_str)
        .context("managed runtime lock is missing minecraft_version")?;
    if lock_minecraft != minecraft_version {
        bail!("managed runtime lock belongs to a different Minecraft version");
    }
    let expected = lock
        .pointer("/artifacts/minecraft_server/sha256")
        .and_then(serde_json::Value::as_str)
        .context("managed runtime lock is missing the Minecraft server SHA-256")?;
    let decoded = hex::decode(expected).context("managed runtime lock contains an invalid Minecraft SHA-256")?;
    if decoded.len() != 32 {
        bail!("managed runtime lock contains an invalid Minecraft SHA-256 length");
    }
    Ok(Some(expected.to_ascii_lowercase()))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("cannot open {} for SHA-256 verification", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("cannot read {} for SHA-256 verification", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Seed the Fabric server launcher's expected game-jar location
/// (`<runtime>/.fabric/server/<minecraft>-server.jar`) only from a
/// managed artifact that still matches its runtime-lock SHA-256.
///
/// Manual/advanced runtime profiles without a managed runtime lock keep
/// their existing behavior and do not consume stray staged artifacts.
fn seed_fabric_game_jar(paths: &DataPaths, world: WorldId, minecraft_version: &str, runtime: &Path) -> Result<()> {
    let Some(expected_sha256) = managed_seed_expected_sha256(paths, world, minecraft_version)? else {
        return Ok(());
    };
    let staged = crate::runtime_layout::managed_world_server_dir(paths, world).join("server.jar");
    if !staged.is_file() {
        bail!("managed runtime lock references a staged Minecraft server jar that is missing at {}", staged.display());
    }
    let actual_sha256 = sha256_file(&staged)?;
    if actual_sha256 != expected_sha256 {
        bail!("staged managed Minecraft server jar failed runtime-lock SHA-256 verification");
    }
    let server_dir = runtime.join(".fabric").join("server");
    fs::create_dir_all(&server_dir).with_context(|| format!("cannot create {}", server_dir.display()))?;
    let game_jar = server_dir.join(format!("{minecraft_version}-server.jar"));
    if game_jar.exists() {
        return Ok(());
    }
    let temporary = server_dir.join(format!("{minecraft_version}-server.jar.tmp-seed"));
    let _ = fs::remove_file(&temporary);
    fs::copy(&staged, &temporary)
        .with_context(|| format!("cannot stage {} into {}", staged.display(), server_dir.display()))?;
    fs::rename(&temporary, &game_jar).with_context(|| format!("cannot publish {}", game_jar.display()))?;
    Ok(())
}

async fn wait_for_child(child: &mut Child) -> Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        sleep(SUPERVISOR_POLL).await;
    }
}

fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// An unverified Minecraft Java launch must not outlive a failed startup.
/// This guard only covers startup, before the runtime is turned over to the
/// Fabric save-first supervisor. After handoff, its Drop never kills Java.
struct StartupJavaGuard<'a> {
    child: &'a mut Child,
    paths: &'a DataPaths,
    world: WorldId,
    armed: bool,
}

impl<'a> StartupJavaGuard<'a> {
    fn new(child: &'a mut Child, paths: &'a DataPaths, world: WorldId) -> Self {
        Self { child, paths, world, armed: true }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for StartupJavaGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        terminate_child(&mut *self.child);
        if let Err(error) = host_readiness::invalidate_runtime_verification(self.paths, self.world) {
            warn!(world = %self.world, %error, "could not retract failed-startup runtime verification");
        }
        if let Err(error) = host_readiness::invalidate_cached_readiness(self.paths, self.world) {
            warn!(world = %self.world, %error, "could not retract failed-startup host readiness");
        }
    }
}

fn publish_failure(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    trigger: MigrationTrigger,
    config: &RuntimeLaunchConfig,
    error: &anyhow::Error,
) -> Result<()> {
    publish_status(
        paths,
        storage,
        world,
        Some(trigger),
        MigrationPhase::Failed,
        false,
        config.game_endpoint.clone(),
        storage.latest_snapshot(world)?.and_then(|snapshot| snapshot.manifest_hash().ok()),
        Some(error.to_string()),
    )
}

#[allow(clippy::too_many_arguments)]
fn publish_status(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    trigger: Option<MigrationTrigger>,
    phase: MigrationPhase,
    runtime_ready: bool,
    game_endpoint: Option<String>,
    snapshot_hash: Option<Hash32>,
    failure_reason: Option<String>,
) -> Result<()> {
    let epoch = storage.load_epoch_record(world).ok();
    let status = MigrationStatus {
        world_id: world.to_string(),
        authority_peer_id: epoch.as_ref().map(|value| value.authority_peer_id.to_string()),
        epoch: epoch.as_ref().map(|value| value.epoch_number),
        fencing_token: epoch.as_ref().map(|value| value.fencing_token),
        trigger,
        phase,
        runtime_ready,
        game_endpoint,
        snapshot_hash: snapshot_hash.map(|value| value.to_string()),
        failure_reason,
    };
    atomic_json(&migration_status_path(paths, world), &status)?;
    Ok(())
}

/// Remove machine-local runtime readiness before awaiting any checkpoint,
/// transfer, fencing or consent-withdrawal barrier. A previously persisted
/// `Ready` report must not remain green while Java is stopping, untrusted, or
/// preserving its last save after a failed barrier. Never treat this status
/// update as a replacement for fresh canonical authority and Fabric checks.
fn retire_live_runtime(paths: &DataPaths, world: WorldId, phase: MigrationPhase, detail: Option<&str>) -> Result<()> {
    // Do this first: even if the status file is corrupt, no older local
    // runtime verification should remain available for optimistic reuse.
    host_readiness::invalidate_runtime_verification(paths, world)?;
    host_readiness::invalidate_cached_readiness(paths, world)?;
    let mut status = load_migration_status(paths, world).context("cannot load live runtime status for retirement")?;
    if status.world_id != world.to_string() {
        bail!("live runtime status belongs to another world; cannot retire readiness safely");
    }
    status.phase = phase;
    status.runtime_ready = false;
    status.game_endpoint = None;
    status.failure_reason = detail.map(str::to_owned);
    atomic_json(&migration_status_path(paths, world), &status)
}

fn runtime_config_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("runtime.json")
}

fn migration_status_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("migration-status.json")
}

fn stop_intent_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("runtime-control").join(world.to_hex()).join("stop.intent")
}

/// A plain `stop\n` legacy intent is not tied to a host generation and must
/// fail closed after an authority change, rather than stopping a successor.
fn load_stop_intent(paths: &DataPaths, epoch: &EpochRecordV1) -> Result<bool> {
    let path = stop_intent_path(paths, epoch.world_id);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("cannot inspect safe-stop intent {}", path.display())),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_STOP_INTENT_BYTES
    {
        bail!("safe-stop intent is not a bounded regular file; refusing unsafe stop");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(&path)
        .with_context(|| format!("cannot open safe-stop intent {}", path.display()))?
        .take(MAX_STOP_INTENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read safe-stop intent {}", path.display()))?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_STOP_INTENT_BYTES {
        bail!("safe-stop intent exceeded size limit; refusing unsafe stop");
    }
    let intent: StopIntentV1 = serde_json::from_slice(&bytes).context("safe-stop intent is malformed or obsolete")?;
    if !intent.matches_epoch(epoch) {
        bail!("safe-stop intent is for a different authority generation, fence or world");
    }
    Ok(true)
}

fn clear_stop_intent(paths: &DataPaths, world: WorldId) -> Result<()> {
    let path = stop_intent_path(paths, world);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("cannot clear safe-stop intent {}", path.display()))?;
    }
    Ok(())
}

fn wake_intent_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("wake.intent")
}

fn transfer_intent_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("transfer.intent")
}

fn control_dir(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("control").join(world.to_hex())
}

fn load_transfer_intent(paths: &DataPaths, epoch: &EpochRecordV1) -> Result<Option<PeerId>> {
    let path = transfer_intent_path(paths, epoch.world_id);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot inspect transfer intent {}", path.display())),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_TRANSFER_INTENT_BYTES
    {
        bail!("transfer intent is not a bounded regular file; refusing unsafe transfer");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(&path)
        .with_context(|| format!("cannot open transfer intent {}", path.display()))?
        .take(MAX_TRANSFER_INTENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read transfer intent {}", path.display()))?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_TRANSFER_INTENT_BYTES {
        bail!("transfer intent exceeded size limit; refusing unsafe transfer");
    }
    let intent: TransferIntentV1 =
        serde_json::from_slice(&bytes).context("transfer intent is malformed or obsolete")?;
    if !intent.matches_epoch(epoch) {
        bail!("transfer intent is for a different authority generation, fence or world");
    }
    Ok(Some(intent.target_peer_id))
}

fn clear_transfer_intent(paths: &DataPaths, world: WorldId) -> Result<()> {
    remove_if_present(&transfer_intent_path(paths, world))
}

pub fn world_wake_requested(paths: &DataPaths, world: WorldId) -> bool {
    wake_intent_path(paths, world).is_file()
}

pub fn clear_world_wake_request(paths: &DataPaths, world: WorldId) -> Result<()> {
    remove_if_present(&wake_intent_path(paths, world))
}

fn clear_wake_intent(paths: &DataPaths, world: WorldId) -> Result<()> {
    clear_world_wake_request(paths, world)
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    atomic_bytes(path, &bytes)
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("control file has no parent directory")?;
    prepare_control_parent(parent)?;
    // Serialize concurrent publishers without removing the previous record.
    let _lock = lock_control_file(&path.with_extension("publish.lock"))?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("control file path is an unsafe symlink or non-regular file: {}", path.display());
        }
    }
    let seq = CONTROL_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), seq));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_control_file_atomically(&tmp, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn prepare_control_parent(parent: &Path) -> Result<()> {
    // Control/status files can be written by the foreground CLI while the
    // supervisor publishes progress. Give every writer a distinct temporary
    // file; never truncate a shared .tmp or remove the old Windows destination
    // before the replacement is committed.
    for directory in [parent.parent(), Some(parent)].into_iter().flatten() {
        if let Ok(metadata) = fs::symlink_metadata(directory) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!("control path has an unsafe symlink or non-directory ancestor: {}", directory.display());
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(parent)?.file_type().is_symlink() {
        bail!("control path directory was replaced by an unsafe symlink");
    }
    Ok(())
}

fn lock_control_file(lock_path: &Path) -> Result<fs::File> {
    if let Ok(metadata) = fs::symlink_metadata(lock_path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("control publication lock is an unsafe symlink or non-regular file");
        }
    }
    let mut lock_options = fs::OpenOptions::new();
    lock_options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600);
    }
    let lock = lock_options.open(lock_path)?;
    FileExt::lock_exclusive(&lock)?;
    let metadata = fs::symlink_metadata(lock_path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("control publication lock became an unsafe symlink");
    }
    Ok(lock)
}

#[cfg(not(windows))]
fn replace_control_file_atomically(tmp: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(tmp, destination)
}

#[cfg(windows)]
fn replace_control_file_atomically(tmp: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }
    let old = tmp.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let new = destination.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    // ReplaceFileW retains an existing destination's visibility under Windows
    // readers, while MoveFileExW creates the first version without an unsafe
    // remove/rename gap. Retry only transient file-sharing errors, bounded.
    for attempt in 0..=40 {
        // SAFETY: all UTF-16 buffers are NUL-terminated and live for the call.
        let result = unsafe {
            if destination.exists() {
                ReplaceFileW(
                    new.as_ptr(),
                    old.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            } else {
                MoveFileExW(old.as_ptr(), new.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
            }
        };
        if result != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if attempt == 40 || !matches!(error.raw_os_error(), Some(2 | 3 | 5 | 32 | 33 | 1176 | 1177)) {
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    unreachable!("bounded Windows control replacement loop returned without a verdict")
}

fn epoch_record_hash(record: &EpochRecordV1) -> Result<Hash32> {
    let encoded = postcard::to_allocvec(record)?;
    Ok(Hash32::from_domain_bytes(b"swarmcraft/epoch-record/v1\0", &encoded))
}

#[cfg(test)]
mod runtime_retirement_tests {
    use super::*;

    #[cfg(unix)]
    fn sleeping_mock_java() -> Child {
        Command::new("/bin/sh").args(["-c", "exec sleep 30"]).spawn().expect("cannot spawn isolated mock Java process")
    }

    #[cfg(unix)]
    #[test]
    fn failed_startup_guard_reaps_java_and_invalidates_both_readiness_proofs() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([34; 32]);
        let proof = control_dir(&paths, world).join("runtime-verified.json");
        let readiness = control_dir(&paths, world).join("host-readiness.json");
        fs::create_dir_all(proof.parent().unwrap()).unwrap();
        fs::write(&proof, b"stale runtime proof").unwrap();
        fs::write(&readiness, b"stale readiness snapshot").unwrap();
        let mut child = sleeping_mock_java();
        {
            let guard = StartupJavaGuard::new(&mut child, &paths, world);
            assert!(guard.pid() != 0);
            // An ordinary fallible startup operation (`?`) unwinds here.
        }
        assert!(child.try_wait().unwrap().is_some(), "failed startup must not orphan a Java process");
        assert!(!proof.exists());
        assert!(!readiness.exists());
    }

    #[cfg(unix)]
    #[test]
    fn disarmed_startup_guard_does_not_kill_a_supervised_java_process() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([35; 32]);
        let mut child = sleeping_mock_java();
        StartupJavaGuard::new(&mut child, &paths, world).disarm();
        assert!(child.try_wait().unwrap().is_none(), "save-first supervisor must retain the live Java child");
        terminate_child(&mut child);
        assert!(child.try_wait().unwrap().is_some());
    }

    fn epoch(world: WorldId) -> EpochRecordV1 {
        EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: 4,
            previous_epoch_hash: None,
            base_state_hash: Hash32([4; 32]),
            authority_peer_id: PeerId([5; 32]),
            authority_public_key: [6; 32],
            mode: EpochMode::Quorum,
            fencing_token: 9,
            reason: "stop-intent-test".into(),
            signature: Vec::new(),
        }
    }

    fn ready_status(world: WorldId) -> MigrationStatus {
        MigrationStatus {
            world_id: world.to_string(),
            authority_peer_id: None,
            epoch: Some(4),
            fencing_token: Some(9),
            trigger: Some(MigrationTrigger::DirectHost),
            phase: MigrationPhase::Ready,
            runtime_ready: true,
            game_endpoint: Some("127.0.0.1:25565".into()),
            snapshot_hash: None,
            failure_reason: None,
        }
    }

    #[test]
    fn checkpointing_or_withdrawal_clears_stale_ready_and_route() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(dir.path());
        let world = WorldId([23; 32]);
        atomic_json(&migration_status_path(&paths, world), &ready_status(world)).unwrap();
        let proof = control_dir(&paths, world).join("runtime-verified.json");
        fs::write(&proof, b"previous runtime proof").unwrap();
        let readiness = control_dir(&paths, world).join("host-readiness.json");
        fs::write(&readiness, b"previous green readiness").unwrap();

        retire_live_runtime(&paths, world, MigrationPhase::Checkpointing, Some("hosting consent withdrawn")).unwrap();
        let retired = load_migration_status(&paths, world).unwrap();
        assert_eq!(retired.world_id, world.to_string());
        assert_eq!(retired.phase, MigrationPhase::Checkpointing);
        assert!(!retired.runtime_ready);
        assert_eq!(retired.game_endpoint, None);
        assert_eq!(retired.failure_reason.as_deref(), Some("hosting consent withdrawn"));
        assert_eq!(retired.epoch, Some(4));
        assert_eq!(retired.fencing_token, Some(9));
        assert!(!proof.exists());
        assert!(!readiness.exists());

        retire_live_runtime(&paths, world, MigrationPhase::Blocked, Some("checkpoint failed")).unwrap();
        let blocked = load_migration_status(&paths, world).unwrap();
        assert_eq!(blocked.phase, MigrationPhase::Blocked);
        assert_eq!(blocked.failure_reason.as_deref(), Some("checkpoint failed"));
        assert!(!blocked.runtime_ready);
    }

    #[test]
    fn mismatched_world_status_cannot_retain_a_runtime_verification() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(dir.path());
        let world = WorldId([24; 32]);
        atomic_json(&migration_status_path(&paths, world), &ready_status(WorldId([25; 32]))).unwrap();
        let proof = control_dir(&paths, world).join("runtime-verified.json");
        fs::write(&proof, b"previous runtime proof").unwrap();
        assert!(retire_live_runtime(&paths, world, MigrationPhase::Superseded, None).is_err());
        assert!(!proof.exists());
    }

    #[test]
    fn safe_stop_intent_must_match_exact_world_epoch_fence_and_authority() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(dir.path());
        let current = epoch(WorldId([30; 32]));
        let intent_path = stop_intent_path(&paths, current.world_id);
        assert!(!load_stop_intent(&paths, &current).unwrap());

        atomic_json(&intent_path, &StopIntentV1::from_epoch(&current)).unwrap();
        assert!(load_stop_intent(&paths, &current).unwrap());

        let mut advanced = current.clone();
        advanced.epoch_number += 1;
        assert!(load_stop_intent(&paths, &advanced).is_err());
        advanced = current.clone();
        advanced.fencing_token += 1;
        assert!(load_stop_intent(&paths, &advanced).is_err());
        advanced = current.clone();
        advanced.authority_peer_id = PeerId([31; 32]);
        assert!(load_stop_intent(&paths, &advanced).is_err());

        fs::write(&intent_path, b"stop\n").unwrap();
        assert!(
            load_stop_intent(&paths, &current).is_err(),
            "legacy plain stop cannot survive an authority transition"
        );
        fs::write(&intent_path, vec![b'X'; MAX_STOP_INTENT_BYTES as usize + 1]).unwrap();
        assert!(load_stop_intent(&paths, &current).is_err(), "oversized stop must fail closed");
    }

    #[test]
    fn transfer_intent_requires_exact_world_generation_fence_and_authority() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(dir.path());
        let current = epoch(WorldId([36; 32]));
        let target = PeerId([37; 32]);
        let intent_path = transfer_intent_path(&paths, current.world_id);

        assert_eq!(load_transfer_intent(&paths, &current).unwrap(), None);
        atomic_json(&intent_path, &TransferIntentV1::from_epoch(&current, target)).unwrap();
        assert_eq!(load_transfer_intent(&paths, &current).unwrap(), Some(target));

        let mut successor = current.clone();
        successor.epoch_number += 1;
        assert!(load_transfer_intent(&paths, &successor).is_err());
        successor = current.clone();
        successor.fencing_token += 1;
        assert!(load_transfer_intent(&paths, &successor).is_err());
        successor = current.clone();
        successor.authority_peer_id = PeerId([38; 32]);
        assert!(load_transfer_intent(&paths, &successor).is_err());

        let mut wrong_world = TransferIntentV1::from_epoch(&current, target);
        wrong_world.world_id = WorldId([39; 32]);
        atomic_json(&intent_path, &wrong_world).unwrap();
        assert!(load_transfer_intent(&paths, &current).is_err());
        let mut unknown_version = TransferIntentV1::from_epoch(&current, target);
        unknown_version.schema_version += 1;
        atomic_json(&intent_path, &unknown_version).unwrap();
        assert!(load_transfer_intent(&paths, &current).is_err());

        // Legacy unscoped target text must not cause a checkpoint after
        // migration to the generation-fenced format.
        fs::write(&intent_path, format!("{}\n", target)).unwrap();
        assert!(load_transfer_intent(&paths, &current).is_err());
        fs::write(&intent_path, vec![b'X'; MAX_TRANSFER_INTENT_BYTES as usize + 1]).unwrap();
        assert!(load_transfer_intent(&paths, &current).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_transfer_intent_cannot_trigger_checkpoint() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(dir.path());
        let current = epoch(WorldId([40; 32]));
        let intent_path = transfer_intent_path(&paths, current.world_id);
        fs::create_dir_all(intent_path.parent().unwrap()).unwrap();
        let outside = dir.path().join("unrelated-transfer");
        fs::write(&outside, serde_json::to_vec(&TransferIntentV1::from_epoch(&current, PeerId([41; 32]))).unwrap())
            .unwrap();
        symlink(&outside, &intent_path).unwrap();
        assert!(load_transfer_intent(&paths, &current).is_err());
        assert!(atomic_json(&intent_path, &TransferIntentV1::from_epoch(&current, PeerId([41; 32]))).is_err());
        assert!(fs::symlink_metadata(&intent_path).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_stop_intent_does_not_request_a_checkpoint() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(dir.path());
        let current = epoch(WorldId([31; 32]));
        let path = stop_intent_path(&paths, current.world_id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let unrelated = dir.path().join("unrelated-stop");
        fs::write(&unrelated, serde_json::to_vec(&StopIntentV1::from_epoch(&current)).unwrap()).unwrap();
        symlink(&unrelated, &path).unwrap();
        assert!(load_stop_intent(&paths, &current).is_err());
        assert!(atomic_json(&path, &StopIntentV1::from_epoch(&current)).is_err());
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }

    #[test]
    fn concurrent_status_writes_never_publish_truncated_json() {
        use std::sync::{atomic::AtomicBool, Arc, Barrier};
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(dir.path());
        let world = WorldId([32; 32]);
        let path = migration_status_path(&paths, world);
        atomic_json(&path, &ready_status(world)).unwrap();
        let active = Arc::new(AtomicBool::new(true));
        let barrier = Arc::new(Barrier::new(3));
        let reader_path = path.clone();
        let reader_active = Arc::clone(&active);
        let reader_barrier = Arc::clone(&barrier);
        let reader = std::thread::spawn(move || {
            reader_barrier.wait();
            while reader_active.load(Ordering::Relaxed) {
                let bytes = match fs::read(&reader_path) {
                    Ok(bytes) => bytes,
                    // A Windows replacement can briefly deny or interrupt
                    // a read. No such failure may be interpreted as a Ready
                    // status: ONLY a complete decoded status is admissible.
                    Err(error)
                        if cfg!(windows)
                            && (matches!(
                                error.kind(),
                                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                            ) || matches!(error.raw_os_error(), Some(32 | 33))) =>
                    {
                        continue;
                    }
                    Err(error) => panic!("unexpected migration status read error: {error}"),
                };
                let status: MigrationStatus = serde_json::from_slice(&bytes).expect("torn migration status read");
                assert_eq!(status.world_id, world.to_string());
            }
        });
        let writer_barrier = Arc::clone(&barrier);
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            writer_barrier.wait();
            for _ in 0..20 {
                atomic_json(&writer_path, &ready_status(world)).unwrap();
            }
        });
        barrier.wait();
        for _ in 0..20 {
            let mut status = ready_status(world);
            status.phase = MigrationPhase::Checkpointing;
            status.runtime_ready = false;
            atomic_json(&path, &status).unwrap();
        }
        writer.join().unwrap();
        active.store(false, Ordering::Relaxed);
        reader.join().unwrap();
        let final_bytes = fs::read(&path).unwrap();
        let final_status: MigrationStatus =
            serde_json::from_slice(&final_bytes).expect("last Windows or Unix publication must be complete");
        assert_eq!(final_status.world_id, world.to_string());
    }
}
