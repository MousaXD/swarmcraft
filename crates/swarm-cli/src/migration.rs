use crate::authority_permit::PermitWatch;
use anyhow::{anyhow, bail, Context, Result};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    str::FromStr,
    time::{Duration, Instant},
};
use swarm_consensus::AuthorityGeneration;
use swarm_core::{
    lifecycle::verify_sleep_record_signature, verify_signature, verify_snapshot_signature, verify_transfer_signature,
    DataPaths, PeerIdentity,
};
use swarm_ipc::FabricBridgeListener;
use swarm_protocol::{
    AuthorityTransferV1, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, PeerId, SleepRecordV1,
    SnapshotManifestV1, TransferPhase, WorldId, PROTOCOL_VERSION,
};
use swarm_storage::{
    serde::{Deserialize, Serialize},
    serde_json, SnapshotContext, Storage,
};
use tokio::{
    task::JoinHandle,
    time::{sleep, timeout},
};
use tracing::{info, warn};

const FABRIC_START_TIMEOUT: Duration = Duration::from_secs(60);
const FABRIC_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const SUPERVISOR_POLL: Duration = Duration::from_millis(250);

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

pub fn save_runtime_config(paths: &DataPaths, world: WorldId, config: &RuntimeLaunchConfig) -> Result<()> {
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

pub fn request_world_wake(paths: &DataPaths, storage: &Storage, world: WorldId) -> Result<()> {
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
    atomic_bytes(&wake_intent_path(paths, world), b"wake\n")?;
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

pub async fn supervise(paths: DataPaths) -> Result<()> {
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
            let task = tokio::spawn(async move {
                if let Err(error) = supervise_world(task_paths.clone(), world).await {
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

async fn supervise_world(paths: DataPaths, world: WorldId) -> Result<()> {
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

        if let Ok(transfer) = storage.load_transfer_record(world) {
            if transfer.from_peer_id == identity.peer_id() {
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
        }

        if storage.load_sleep_record(world).is_ok() {
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
            let result = run_authority_runtime_inner(
                &paths,
                &storage,
                config.host_options(world),
                MigrationTrigger::WorldWake,
                config.game_endpoint.clone(),
                false,
            )
            .await;
            if let Err(error) = result {
                publish_failure(&paths, &storage, world, MigrationTrigger::WorldWake, &config, &error)?;
                sleep(Duration::from_secs(1)).await;
            }
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
        match run_authority_runtime_inner(
            &paths,
            &storage,
            config.host_options(world),
            trigger,
            config.game_endpoint.clone(),
            false,
        )
        .await
        {
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
        let current = storage.load_epoch_record(world)?;
        if current.epoch_number != epoch.epoch_number
            || current.fencing_token != epoch.fencing_token
            || current.authority_peer_id != identity.peer_id()
            || current.authority_public_key != identity.public_key()
        {
            return Ok(false);
        }
        if let Ok(transfer) = storage.load_transfer_record(world) {
            if transfer.from_peer_id == identity.peer_id() {
                return Ok(false);
            }
        }
        if watch.observe(paths, world, Instant::now()).unwrap_or(false) {
            return Ok(true);
        }
        sleep(SUPERVISOR_POLL).await;
    }
}

fn infer_trigger(storage: &Storage, epoch: &EpochRecordV1, local_peer: PeerId) -> MigrationTrigger {
    if epoch.mode == EpochMode::Recovery {
        return MigrationTrigger::AutomaticRecovery;
    }
    if storage.load_transfer_record(epoch.world_id).is_ok_and(|transfer| {
        transfer.phase == TransferPhase::Committed
            && transfer.to_peer_id == local_peer
            && transfer.next_epoch == epoch.epoch_number
            && transfer.next_fencing_token == epoch.fencing_token
    }) {
        return MigrationTrigger::ManualTransfer;
    }
    MigrationTrigger::DirectHost
}

pub async fn run_authority_runtime(paths: &DataPaths, storage: &Storage, options: HostOptions) -> Result<()> {
    let identity = PeerIdentity::load_or_create(paths)?;
    let descriptor = storage.load_world_descriptor(options.world)?;
    let member_count = descriptor.members.iter().filter(|member| !member.banned).count();
    if member_count > 1 {
        if storage.load_sleep_record(options.world).is_ok() {
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
    run_authority_runtime_inner(paths, storage, options, MigrationTrigger::DirectHost, None, true).await
}

async fn run_authority_runtime_inner(
    paths: &DataPaths,
    storage: &Storage,
    options: HostOptions,
    trigger: MigrationTrigger,
    game_endpoint: Option<String>,
    handle_shutdown_signal: bool,
) -> Result<()> {
    if !options.accept_eula {
        bail!("Minecraft server EULA acceptance is required; configure the runtime only after reviewing Mojang's EULA");
    }
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
    let epoch = prepare_authority_epoch(storage, &identity, options.world, &initial)?;
    let latest = storage.latest_snapshot(options.world)?.context("authority transition lost its canonical snapshot")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;
    ensure_authority_generation(storage, &identity, &epoch)?;

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
    reset_runtime_directory(&runtime)?;
    fs::create_dir_all(runtime.join("mods"))?;
    fs::copy(&options.mod_jar, runtime.join("mods/swarmcraft-fabric.jar"))
        .with_context(|| format!("cannot install Fabric bridge from {}", options.mod_jar.display()))?;
    fs::write(runtime.join("eula.txt"), "eula=true\n")?;

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
    let mut child = launch_server(
        &options.java,
        &options.server_jar,
        &runtime,
        &world_dir,
        metadata.genesis.compatibility_fingerprint,
        &launch.environment(),
    )?;

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
    let mut session = match listener.accept(FABRIC_START_TIMEOUT).await {
        Ok(session) => session,
        Err(error) => {
            terminate_child(&mut child);
            return Err(error.into());
        }
    };
    if let Err(error) = validate_world_info(
        session.world_info(),
        &metadata.genesis.minecraft_version,
        &metadata.genesis.fabric_loader_version,
        &world_dir,
        metadata.genesis.compatibility_fingerprint,
    ) {
        terminate_child(&mut child);
        return Err(error);
    }
    if let Err(error) = ensure_authority_generation(storage, &identity, &epoch) {
        terminate_child(&mut child);
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
    info!(world = %options.world, pid = child.id(), ?trigger, "Minecraft authority runtime is ready");

    let disposition = match wait_for_runtime_exit(
        paths,
        storage,
        &identity,
        &epoch,
        options.world,
        &mut child,
        &mut session,
        handle_shutdown_signal,
    )
    .await
    {
        Ok(disposition) => disposition,
        Err(error) => {
            terminate_child(&mut child);
            return Err(error);
        }
    };

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
    let number = storage.next_snapshot_number(options.world)?;
    let previous_hash = Some(latest.manifest_hash()?);
    let mut final_manifest = storage.snapshot_directory(
        &world_dir,
        SnapshotContext {
            world: options.world,
            snapshot_number: number,
            epoch: epoch.epoch_number,
            sequence: latest.sequence.saturating_add(1),
            previous_snapshot_hash: previous_hash,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
        },
    )?;
    ensure_authority_generation(storage, &identity, &epoch)?;
    identity.sign_snapshot(&mut final_manifest)?;
    storage.commit_snapshot(&final_manifest)?;

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

async fn wait_for_runtime_exit(
    paths: &DataPaths,
    storage: &Storage,
    identity: &PeerIdentity,
    epoch: &EpochRecordV1,
    world: WorldId,
    child: &mut Child,
    session: &mut swarm_ipc::FabricSession,
    handle_shutdown_signal: bool,
) -> Result<RuntimeDisposition> {
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("Minecraft server exited unsuccessfully: {status}");
            }
            return Ok(RuntimeDisposition::Sleep);
        }
        ensure_authority_generation(storage, identity, epoch)?;
        match load_transfer_intent(paths, world) {
            Ok(Some(target)) => {
                validate_transfer_target(storage, identity, world, target)?;
                session.prepare_shutdown(2, FABRIC_SHUTDOWN_TIMEOUT).await?;
                timeout(FABRIC_SHUTDOWN_TIMEOUT, wait_for_child(child))
                    .await
                    .map_err(|_| anyhow!("Minecraft did not stop after the transfer save barrier"))??;
                return Ok(RuntimeDisposition::Transfer(target));
            }
            Ok(None) => {}
            Err(error) => warn!(%world, %error, "ignoring unreadable transfer intent"),
        }
        if handle_shutdown_signal {
            tokio::select! {
                _ = sleep(SUPERVISOR_POLL) => {}
                result = tokio::signal::ctrl_c() => {
                    result.context("failed to listen for Ctrl+C")?;
                    session.prepare_shutdown(1, FABRIC_SHUTDOWN_TIMEOUT).await?;
                    timeout(FABRIC_SHUTDOWN_TIMEOUT, wait_for_child(child))
                        .await
                        .map_err(|_| anyhow!("Minecraft did not stop after the shutdown barrier"))??;
                    return Ok(RuntimeDisposition::Sleep);
                }
            }
        } else {
            sleep(SUPERVISOR_POLL).await;
        }
    }
}

pub fn prepare_manual_transfer(
    paths: &DataPaths,
    storage: &Storage,
    world: WorldId,
    target: PeerId,
) -> Result<TransferPrepareResult> {
    let identity = PeerIdentity::load_or_create(paths)?;
    let epoch = storage.load_epoch_record(world)?;
    ensure_authority_generation(storage, &identity, &epoch)?;
    validate_transfer_target(storage, &identity, world, target)?;
    let latest = storage.latest_snapshot(world)?.context("cannot transfer authority without a canonical snapshot")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;

    if let Ok(existing) = storage.load_transfer_record(world) {
        if existing.from_peer_id == identity.peer_id() && existing.to_peer_id == target {
            return Ok(TransferPrepareResult::Prepared(encode_transfer(&existing)?));
        }
        bail!("another authority transfer is already durable for this world");
    }

    if let Ok(sleep_record) = storage.load_sleep_record(world) {
        verify_sleep_record_signature(&sleep_record)?;
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

    atomic_bytes(&transfer_intent_path(paths, world), format!("{}\n", target).as_bytes())?;
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
    let identity = PeerIdentity::load_or_create(paths)?;
    let prepared = decode_transfer(token)?;
    if prepared.world_id != world || prepared.phase != TransferPhase::Prepared {
        bail!("transfer token is not a prepared transfer for this world");
    }
    validate_transfer_record(storage, &prepared)?;
    if prepared.to_peer_id != identity.peer_id() {
        bail!("only the prepared target may accept this transfer");
    }
    storage.save_transfer_record(&prepared)?;
    let mut accepted = prepared.clone();
    accepted.phase = TransferPhase::Accepted;
    identity.sign_transfer(&mut accepted)?;
    storage.save_transfer_record(&accepted)?;
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
    if let Ok(previous) = storage.load_transfer_record(world) {
        ensure_same_transfer(&previous, &accepted)?;
        if previous.phase != TransferPhase::Prepared && previous.phase != TransferPhase::Accepted {
            bail!("local transfer state cannot advance to committed");
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
    let identity = PeerIdentity::load_or_create(paths)?;
    let committed = decode_transfer(token)?;
    if committed.world_id != world || committed.phase != TransferPhase::Committed {
        bail!("transfer token is not a committed transfer for this world");
    }
    validate_transfer_record(storage, &committed)?;
    if committed.to_peer_id != identity.peer_id() {
        bail!("only the committed target may activate this transfer");
    }
    storage.save_transfer_record(&committed)?;
    let current = storage.load_epoch_record(world)?;
    if current.authority_peer_id != committed.from_peer_id
        || current.epoch_number.saturating_add(1) != committed.next_epoch
        || current.fencing_token.saturating_add(1) != committed.next_fencing_token
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
    storage.save_epoch_record(&next)?;
    storage.clear_sleep_record(world)?;
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
    if transfer.phase != TransferPhase::Committed
        || transfer.from_peer_id != current.authority_peer_id
        || transfer.to_peer_id != next.authority_peer_id
        || transfer.next_epoch != next.epoch_number
        || transfer.next_fencing_token != next.fencing_token
        || next.epoch_number != current.epoch_number.saturating_add(1)
        || next.fencing_token != current.fencing_token.saturating_add(1)
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

fn create_prepared_transfer(
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
    target: PeerId,
    epoch: &EpochRecordV1,
    latest: &SnapshotManifestV1,
) -> Result<AuthorityTransferV1> {
    validate_transfer_target(storage, identity, world, target)?;
    let mut transfer = AuthorityTransferV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        from_peer_id: identity.peer_id(),
        to_peer_id: target,
        base_snapshot_hash: latest.manifest_hash()?,
        next_epoch: epoch.epoch_number.saturating_add(1),
        next_fencing_token: epoch.fencing_token.saturating_add(1),
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
    verify_transfer_signature(transfer)?;
    let descriptor = storage.load_world_descriptor(transfer.world_id)?;
    let from = descriptor.member(transfer.from_peer_id).context("transfer source is not a world member")?;
    let to = descriptor.member(transfer.to_peer_id).context("transfer target is not a world member")?;
    if from.banned || to.banned || !to.authority_eligible {
        bail!("transfer participants are banned or target is not authority eligible");
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
    let current = storage.load_epoch_record(transfer.world_id)?;
    if transfer.next_epoch != current.epoch_number.saturating_add(1)
        || transfer.next_fencing_token != current.fencing_token.saturating_add(1)
        || transfer.from_peer_id != current.authority_peer_id
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
    Ok(postcard::from_bytes(&bytes).context("transfer token is malformed")?)
}

fn encode_epoch(epoch: &EpochRecordV1) -> Result<String> {
    Ok(hex::encode(postcard::to_allocvec(epoch)?))
}

fn decode_epoch(token: &str) -> Result<EpochRecordV1> {
    let bytes = hex::decode(token.trim()).context("epoch token is not valid hex")?;
    Ok(postcard::from_bytes(&bytes).context("epoch token is malformed")?)
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

    if let Ok(sleep_record) = storage.load_sleep_record(world) {
        verify_sleep_record_signature(&sleep_record)?;
        if sleep_record.latest_snapshot_hash != latest.manifest_hash()? {
            bail!("local replica is stale and cannot wake the sleeping world");
        }
        let previous = storage.load_epoch_record(world)?;
        let mut next = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: previous.epoch_number.saturating_add(1),
            previous_epoch_hash: Some(epoch_record_hash(&previous)?),
            base_state_hash: latest.state_root,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            mode: EpochMode::Solo,
            fencing_token: previous.fencing_token.saturating_add(1),
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
        if latest.epoch.saturating_add(1) != epoch.epoch_number || latest.state_root != epoch.base_state_hash {
            bail!("accepted authority epoch does not directly promote the latest canonical snapshot");
        }
        let mut promoted = SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: epoch.world_id,
            snapshot_number: storage.next_snapshot_number(epoch.world_id)?,
            epoch: epoch.epoch_number,
            sequence: latest.sequence.saturating_add(1),
            previous_snapshot_hash: Some(latest.manifest_hash()?),
            entries: latest.entries.clone(),
            state_root: latest.state_root,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity.sign_snapshot(&mut promoted)?;
        storage.commit_snapshot(&promoted)?;
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
                sequence: membership.sequence.saturating_add(1),
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
    runtime: &Path,
    world_dir: &Path,
    compatibility: Hash32,
    ipc_environment: &[(String, String); 3],
) -> Result<Child> {
    let mut command = Command::new(java);
    command.arg("-jar").arg(server_jar).arg("nogui").current_dir(runtime);
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

fn runtime_config_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("runtime.json")
}

fn migration_status_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    control_dir(paths, world).join("migration-status.json")
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

fn load_transfer_intent(paths: &DataPaths, world: WorldId) -> Result<Option<PeerId>> {
    let path = transfer_intent_path(paths, world);
    if !path.exists() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(Some(PeerId::from_str(value.trim()).context("transfer intent contains an invalid peer ID")?))
}

fn clear_transfer_intent(paths: &DataPaths, world: WorldId) -> Result<()> {
    remove_if_present(&transfer_intent_path(paths, world))
}

fn clear_wake_intent(paths: &DataPaths, world: WorldId) -> Result<()> {
    remove_if_present(&wake_intent_path(paths, world))
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
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn epoch_record_hash(record: &EpochRecordV1) -> Result<Hash32> {
    let encoded = postcard::to_allocvec(record)?;
    Ok(Hash32::from_domain_bytes(b"swarmcraft/epoch-record/v1\0", &encoded))
}
