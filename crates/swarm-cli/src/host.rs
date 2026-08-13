use anyhow::{anyhow, bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    time::Duration,
};
use swarm_core::{
    lifecycle::verify_sleep_record_signature, verify_snapshot_signature, DataPaths, PeerIdentity,
};
use swarm_ipc::FabricBridgeListener;
use swarm_protocol::{EpochMode, EpochRecordV1, Hash32, SleepRecordV1, WorldId, PROTOCOL_VERSION};
use swarm_storage::{SnapshotContext, Storage};
use tokio::time::{sleep, timeout};
use tracing::{info, warn};

pub struct HostOptions {
    pub world: WorldId,
    pub java: PathBuf,
    pub server_jar: PathBuf,
    pub mod_jar: PathBuf,
    pub accept_eula: bool,
}

pub async fn run(paths: &DataPaths, storage: &Storage, options: HostOptions) -> Result<()> {
    if !options.accept_eula {
        bail!("Minecraft server EULA acceptance is required; rerun with --accept-eula after reviewing Mojang's EULA");
    }
    let identity = PeerIdentity::load_or_create(paths)?;
    let metadata = storage.load_world(options.world)?;
    let latest = storage.latest_snapshot(options.world)?.context("world has no verified snapshot to host")?;
    storage.verify_snapshot(&latest)?;
    verify_snapshot_signature(&latest)?;

    let epoch = prepare_authority_epoch(storage, &identity, options.world, &latest)?;
    let runtime = paths.root.join("runtime").join(options.world.to_hex());
    let world_dir = runtime.join("world");
    reset_runtime_directory(&runtime)?;
    storage.restore_snapshot(&latest, &world_dir)?;
    fs::create_dir_all(runtime.join("mods"))?;
    fs::copy(&options.mod_jar, runtime.join("mods/swarmcraft-fabric.jar"))
        .with_context(|| format!("cannot install Fabric bridge from {}", options.mod_jar.display()))?;
    fs::write(runtime.join("eula.txt"), "eula=true\n")?;

    let listener = FabricBridgeListener::bind().await?;
    let launch = listener.launch_config()?;
    let mut child = launch_server(
        &options.java,
        &options.server_jar,
        &runtime,
        &world_dir,
        metadata.genesis.compatibility_fingerprint,
        &launch.environment(),
    )?;

    let mut session = match listener.accept(Duration::from_secs(60)).await {
        Ok(session) => session,
        Err(error) => {
            terminate_child(&mut child);
            return Err(error.into());
        }
    };
    validate_world_info(
        session.world_info(),
        &metadata.genesis.minecraft_version,
        &metadata.genesis.fabric_loader_version,
        &world_dir,
        metadata.genesis.compatibility_fingerprint,
    )?;
    info!(world = %options.world, pid = child.id(), "Minecraft authority runtime is ready");

    let status = tokio::select! {
        result = wait_for_child(&mut child) => result?,
        result = tokio::signal::ctrl_c() => {
            result.context("failed to listen for Ctrl+C")?;
            info!(world = %options.world, "graceful shutdown requested");
            if let Err(error) = session.prepare_shutdown(1, Duration::from_secs(30)).await {
                warn!(%error, "Fabric shutdown barrier failed; terminating server without committing a new snapshot");
                terminate_child(&mut child);
                return Err(error.into());
            }
            timeout(Duration::from_secs(30), wait_for_child(&mut child))
                .await
                .map_err(|_| anyhow!("Minecraft did not stop after the shutdown barrier"))??
        }
    };
    if !status.success() {
        bail!("Minecraft server exited unsuccessfully: {status}");
    }

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
    identity.sign_snapshot(&mut final_manifest)?;
    storage.commit_snapshot(&final_manifest)?;

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
    info!(world = %options.world, snapshot = final_manifest.snapshot_number, "world committed and sleeping");
    Ok(())
}

fn prepare_authority_epoch(
    storage: &Storage,
    identity: &PeerIdentity,
    world: WorldId,
    latest: &swarm_protocol::SnapshotManifestV1,
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
        return Ok(next);
    }

    match storage.load_epoch_record(world) {
        Ok(epoch) => {
            if epoch.authority_peer_id != identity.peer_id()
                || epoch.authority_public_key != identity.public_key()
                || epoch.epoch_number != latest.epoch
                || epoch.base_state_hash != latest.state_root
            {
                bail!("local peer does not hold the accepted authority epoch for this snapshot");
            }
            Ok(epoch)
        }
        Err(_) if latest.authority_peer_id == identity.peer_id() && latest.authority_public_key == identity.public_key() => {
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

fn epoch_record_hash(record: &EpochRecordV1) -> Result<Hash32> {
    let encoded = postcard::to_allocvec(record)?;
    Ok(Hash32::from_domain_bytes(b"swarmcraft/epoch-record/v1\0", &encoded))
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
        sleep(Duration::from_millis(250)).await;
    }
}

fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
