use crate::{server_mods, world_genesis};
use anyhow::{bail, Context, Result};
#[cfg(windows)]
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};
use swarm_core::{create_world_genesis_with_fingerprint, sign_world_config, DataPaths, PeerIdentity};
use swarm_protocol::{
    validate_runtime_selection, AuthorityPolicyV1, Hash32, MembershipPolicyV1, MembershipRecordV1,
    RuntimeCompatibilityManifestV1, SnapshotManifestV1, WorldConfigV1, WorldDescriptorV1, WorldId, WorldMemberV1,
    WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{SnapshotContext, Storage, WorldMetadataV1};

#[derive(Debug, Clone)]
pub struct ImportWorldRequest {
    pub source: PathBuf,
    pub name: String,
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub visibility: WorldVisibilityV1,
    pub server_mod_jars: Vec<PathBuf>,
    /// Importing a save cannot reveal which third-party server mods were used.
    /// Callers must either provide every canonical server-mod JAR or explicitly
    /// confirm that there are no third-party server-mod requirements.
    pub confirm_no_server_mods: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportWorldResult {
    pub world_id: String,
    pub display_name: String,
    pub snapshot_number: u64,
    pub snapshot_hash: String,
    pub state_root: String,
    pub files: usize,
    pub compatibility_fingerprint: String,
}

/// An explicitly selected local checkpoint becomes a separate private world.
/// This action never attempts to acquire or repair the original authority.
#[derive(Debug, Clone)]
pub struct RecoverCopyRequest {
    pub source_world: WorldId,
    pub expected_checkpoint: Hash32,
    pub name: Option<String>,
    pub confirm_new_identity: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoverCopyResult {
    pub source_world_id: String,
    pub source_snapshot_number: u64,
    pub source_snapshot_hash: String,
    pub world_id: String,
    pub display_name: String,
    pub snapshot_hash: String,
    pub state_root: String,
    pub compatibility_fingerprint: String,
    pub files: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct ImportFaults {
    after_snapshot_commit: bool,
    before_publication: bool,
}

pub(crate) struct MinecraftSourceQuiescence {
    file: fs::File,
}

#[cfg(unix)]
impl MinecraftSourceQuiescence {
    pub(crate) fn sync_lock_file(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}

impl Drop for MinecraftSourceQuiescence {
    fn drop(&mut self) {
        release_minecraft_session_lock(&self.file);
    }
}

/// Import Minecraft world DATA into SwarmCraft without importing machine-local
/// runtime binaries or EULA state.
///
/// The complete SwarmCraft world is assembled beneath an invisible staging
/// root on the same filesystem and becomes visible only through one directory
/// rename after every signed metadata object and canonical snapshot is durable.
pub fn import_world(paths: &DataPaths, request: &ImportWorldRequest) -> Result<ImportWorldResult> {
    import_world_inner(paths, request, ImportFaults::default())
}

pub fn recover_copy(paths: &DataPaths, request: &RecoverCopyRequest) -> Result<RecoverCopyResult> {
    recover_copy_inner(paths, request, ImportFaults::default())
}

fn recover_copy_inner(
    paths: &DataPaths,
    request: &RecoverCopyRequest,
    faults: ImportFaults,
) -> Result<RecoverCopyResult> {
    if !request.confirm_new_identity {
        bail!("recovery copy requires explicit confirmation that a NEW world identity will be created");
    }
    let storage = Storage::open(paths.root.clone())?;
    let identity = PeerIdentity::load_or_create(paths)?;
    let source = crate::historical_authority::verify_local_recovery_source(
        &storage,
        &identity,
        request.source_world,
        request.expected_checkpoint,
    )?;
    let name = request.name.as_deref().unwrap_or(&source.config.presentation.name).trim();
    if name.is_empty() {
        bail!("recovered world name is required");
    }
    let display_name = format!("Recovered: {name}");
    // Preserve the exact signed compatibility contract. Do not infer mods
    // from world bytes or silently copy EULA, Java paths or hosting consent.
    let compatibility = source.config.compatibility.clone();
    let fingerprint = compatibility.fingerprint()?;
    let (world, genesis) = create_world_genesis_with_fingerprint(
        &identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )?;
    if world == request.source_world {
        bail!("recovered world identity must differ from the original");
    }
    let staging_root = paths.root.join(".recovery-staging").join(world.to_hex());
    let final_world_dir = paths.worlds_dir().join(world.to_hex());
    if staging_root.exists() || final_world_dir.exists() {
        bail!("generated recovery destination already exists; refusing to overwrite");
    }
    swarm_core::create_private_dir_all(&staging_root)?;
    let outcome = (|| -> Result<RecoverCopyResult> {
        let staged = Storage::open(staging_root.clone())?;
        staged.create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: display_name.clone(),
            world_id: world,
            genesis: genesis.clone(),
        })?;
        let descriptor = WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: fingerprint,
            members: vec![local_member(&identity)],
            preferred_replication_factor: 2,
        };
        staged.save_world_descriptor(&descriptor)?;

        // Copy immutable encoded blobs into the invisible new world. The
        // bounded receiver independently verifies every descriptor, so a
        // concurrent source GC/corruption is a failure, never a partial copy.
        const COPY_CHUNK: usize = 256 * 1024;
        for entry in &source.manifest.entries {
            let blob = &entry.blob;
            if staged.has_complete_blob(world, blob) {
                continue;
            }
            let mut offset = 0;
            loop {
                let (bytes, finished) =
                    storage.read_encoded_blob_chunk(request.source_world, blob, offset, COPY_CHUNK)?;
                if bytes.is_empty() && !finished {
                    bail!("recovery blob copy made no progress");
                }
                offset = staged.receive_blob_chunk(world, blob, offset, &bytes, finished)?;
                if finished {
                    break;
                }
            }
        }
        let mut snapshot = SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            snapshot_number: 1,
            epoch: world_genesis::GENESIS_EPOCH,
            sequence: 1,
            previous_snapshot_hash: None,
            entries: source.manifest.entries.clone(),
            state_root: source.manifest.state_root,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity.sign_snapshot(&mut snapshot)?;
        staged.commit_snapshot(&snapshot)?;
        staged.verify_snapshot(&snapshot)?;
        swarm_core::verify_snapshot_signature(&snapshot)?;
        world_genesis::establish_initial_epoch(&staged, &identity, &snapshot)?;
        let mut membership = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: world_genesis::GENESIS_EPOCH,
            sequence: 0,
            previous_membership_hash: None,
            members: descriptor.members.clone(),
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        identity.sign_membership(&mut membership)?;
        staged.save_membership_record(&membership)?;
        let mut config = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            sequence: 1,
            previous_config_hash: None,
            compatibility,
            visibility: WorldVisibilityV1::Private,
            authority_policy: AuthorityPolicyV1 {
                allow_solo_advancement: false,
                preferred_replication_factor: 2,
            },
            membership_policy: MembershipPolicyV1::InviteOnly,
            presentation: WorldPresentationV1 {
                name: display_name.clone(),
                description: format!(
                    "Independent recovered copy of {} at checkpoint #{} ({}). Not a continuation of the original authority history.",
                    request.source_world, source.manifest.snapshot_number, request.expected_checkpoint
                ),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        sign_world_config(&identity, &mut config)?;
        staged.save_world_config(&config)?;
        if faults.after_snapshot_commit {
            bail!("injected recovered-copy interruption after snapshot commit");
        }
        // Recheck the user's exact selection before publishing. No lease,
        // membership, runtime, invite or recovery state was changed in source.
        source.ensure_unchanged(&storage)?;
        if faults.before_publication {
            bail!("injected recovered-copy interruption before publication");
        }
        let result = RecoverCopyResult {
            source_world_id: request.source_world.to_string(),
            source_snapshot_number: source.manifest.snapshot_number,
            source_snapshot_hash: request.expected_checkpoint.to_string(),
            world_id: world.to_string(),
            display_name,
            snapshot_hash: snapshot.manifest_hash()?.to_string(),
            state_root: snapshot.state_root.to_string(),
            compatibility_fingerprint: fingerprint.to_string(),
            files: snapshot.entries.len(),
        };
        publish_staged_world(paths, &staged.world_dir(world), &final_world_dir)?;
        Ok(result)
    })();
    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    outcome
}

fn import_world_inner(
    paths: &DataPaths,
    request: &ImportWorldRequest,
    faults: ImportFaults,
) -> Result<ImportWorldResult> {
    validate_request(request)?;
    validate_runtime_selection(request.minecraft_version.trim(), request.fabric_loader_version.trim(), None)
        .context("import compatibility is not supported by the shipped SwarmCraft Fabric adapter")?;

    // Minecraft's session.lock is an exclusive process-held record lock. Hold a
    // shared lock while reading: it conflicts with a running game's exclusive
    // lock while still allowing this process to snapshot the lock file on
    // Windows. The proof spans the complete hashed and committed snapshot.
    let _source_quiescence = acquire_source_quiescence(&request.source)?;

    let identity = PeerIdentity::load_or_create(paths)?;
    let required_server_mods = server_mods::requirements_from_jars(&request.server_mod_jars)?;
    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: request.minecraft_version.trim().to_owned(),
        loader_id: "fabric".into(),
        loader_version: request.fabric_loader_version.trim().to_owned(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
        required_server_mods,
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let compatibility_fingerprint = compatibility.fingerprint()?;
    let (world, genesis) = create_world_genesis_with_fingerprint(
        &identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        compatibility_fingerprint,
    )?;

    let staging_root = paths.root.join(".import-staging").join(world.to_hex());
    let final_world_dir = paths.worlds_dir().join(world.to_hex());
    if final_world_dir.exists() {
        bail!("generated import world already exists: {}", final_world_dir.display());
    }
    if staging_root.exists() {
        fs::remove_dir_all(&staging_root)
            .with_context(|| format!("cannot clear stale import staging directory {}", staging_root.display()))?;
    }

    let outcome = stage_and_publish(
        paths,
        &staging_root,
        &final_world_dir,
        &identity,
        world,
        genesis,
        compatibility,
        request,
        faults,
    );
    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
fn stage_and_publish(
    paths: &DataPaths,
    staging_root: &Path,
    final_world_dir: &Path,
    identity: &PeerIdentity,
    world: WorldId,
    genesis: swarm_protocol::WorldGenesisV1,
    compatibility: RuntimeCompatibilityManifestV1,
    request: &ImportWorldRequest,
    faults: ImportFaults,
) -> Result<ImportWorldResult> {
    let staged_paths = DataPaths::from_root(staging_root.to_path_buf());
    let staged = Storage::open(staged_paths.root.clone())?;
    let display_name = request.name.trim().to_owned();

    staged.create_world(&WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: display_name.clone(),
        world_id: world,
        genesis: genesis.clone(),
    })?;

    let descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: genesis.compatibility_fingerprint,
        members: vec![local_member(identity)],
        preferred_replication_factor: 2,
    };
    staged.save_world_descriptor(&descriptor)?;

    // `_source_quiescence` in import_world_inner remains alive throughout this
    // call, so snapshot_directory cannot race a live Minecraft writer.
    let mut snapshot = staged.snapshot_directory(
        &request.source,
        SnapshotContext {
            world,
            snapshot_number: 1,
            epoch: world_genesis::GENESIS_EPOCH,
            sequence: 1,
            previous_snapshot_hash: None,
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
        },
    )?;
    identity.sign_snapshot(&mut snapshot)?;
    staged.commit_snapshot(&snapshot)?;
    staged.verify_snapshot(&snapshot)?;
    swarm_core::verify_snapshot_signature(&snapshot)?;

    world_genesis::establish_initial_epoch(&staged, identity, &snapshot)?;

    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: world_genesis::GENESIS_EPOCH,
        sequence: 0,
        previous_membership_hash: None,
        members: descriptor.members.clone(),
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    identity.sign_membership(&mut membership)?;
    staged.save_membership_record(&membership)?;

    let mut world_config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: request.visibility,
        authority_policy: AuthorityPolicyV1 {
            allow_solo_advancement: true,
            preferred_replication_factor: descriptor.preferred_replication_factor,
        },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: display_name.clone(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: identity.peer_id(),
        authority_public_key: identity.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(identity, &mut world_config)?;
    staged.save_world_config(&world_config)?;

    if faults.after_snapshot_commit {
        bail!("injected existing-world import interruption after snapshot commit");
    }

    // Server mod JARs are machine-local runtime profile material. They are
    // verified against the canonical requirements but are deliberately not
    // copied into the canonical Minecraft save snapshot.
    for source in &request.server_mod_jars {
        server_mods::add_local_mod(&staged_paths, world, &world_config.compatibility, source)?;
    }

    if faults.before_publication {
        bail!("injected existing-world import interruption before publication");
    }

    let result = ImportWorldResult {
        world_id: world.to_string(),
        display_name,
        snapshot_number: snapshot.snapshot_number,
        snapshot_hash: snapshot.manifest_hash()?.to_string(),
        state_root: snapshot.state_root.to_string(),
        files: snapshot.entries.len(),
        compatibility_fingerprint: genesis.compatibility_fingerprint.to_string(),
    };
    let staged_world_dir = staged.world_dir(world);

    // SnapshotPublication owns a kernel-held publication lock until it is
    // dropped. The manifest is durable and its pins were released by
    // commit_snapshot above, so release that transaction lease before moving
    // the complete staged directory. Unix permits renaming beneath an open
    // handle, while Windows correctly rejects it with AccessDenied.
    drop(snapshot);

    publish_staged_world(paths, &staged_world_dir, final_world_dir)?;
    Ok(result)
}

fn publish_staged_world(paths: &DataPaths, staged_world_dir: &Path, final_world_dir: &Path) -> Result<()> {
    fs::create_dir_all(paths.worlds_dir())
        .with_context(|| format!("cannot create SwarmCraft worlds directory {}", paths.worlds_dir().display()))?;
    if final_world_dir.exists() {
        bail!("import destination unexpectedly appeared: {}", final_world_dir.display());
    }
    fs::rename(staged_world_dir, final_world_dir).with_context(|| {
        format!(
            "cannot atomically publish imported world {} -> {}",
            staged_world_dir.display(),
            final_world_dir.display()
        )
    })?;
    if let Err(error) = sync_directory(&paths.worlds_dir()) {
        // Publication durability was not proven. Roll the directory back out of
        // the visible namespace when possible and fail closed.
        let _ = fs::rename(final_world_dir, staged_world_dir);
        return Err(error).context("import publication directory could not be durably synchronized");
    }

    Ok(())
}

fn validate_request(request: &ImportWorldRequest) -> Result<()> {
    if request.name.trim().is_empty() {
        bail!("imported world name is required");
    }
    if request.minecraft_version.trim().is_empty() || request.minecraft_version.trim().eq_ignore_ascii_case("unknown") {
        bail!("exact Minecraft version is required for safe import compatibility");
    }
    if request.fabric_loader_version.trim().is_empty()
        || request.fabric_loader_version.trim().eq_ignore_ascii_case("unknown")
    {
        bail!("exact Fabric loader version is required for safe import compatibility");
    }
    let metadata = fs::metadata(&request.source)
        .with_context(|| format!("cannot inspect Minecraft world directory {}", request.source.display()))?;
    if !metadata.is_dir() {
        bail!("Minecraft world import source is not a directory: {}", request.source.display());
    }
    let level = request.source.join("level.dat");
    let level_metadata = fs::symlink_metadata(&level)
        .with_context(|| format!("Minecraft world is missing readable level.dat at {}", level.display()))?;
    if !level_metadata.file_type().is_file() || level_metadata.file_type().is_symlink() || level_metadata.len() == 0 {
        bail!("Minecraft world level.dat must be a non-empty regular file");
    }
    if request.server_mod_jars.is_empty() && !request.confirm_no_server_mods {
        bail!(
            "server-mod compatibility is unknown; provide every required server mod JAR or explicitly confirm that this world requires no third-party server mods"
        );
    }
    if !request.server_mod_jars.is_empty() && request.confirm_no_server_mods {
        bail!("cannot both provide server mod JARs and declare that the imported world requires no third-party server mods");
    }
    Ok(())
}

fn acquire_source_quiescence(source: &Path) -> Result<MinecraftSourceQuiescence> {
    acquire_minecraft_source_lock(source, false)
}

#[cfg(unix)]
pub(crate) fn acquire_archive_quiescence(source: &Path) -> Result<MinecraftSourceQuiescence> {
    acquire_minecraft_source_lock(source, true)
}

fn acquire_minecraft_source_lock(source: &Path, exclusive: bool) -> Result<MinecraftSourceQuiescence> {
    let lock_path = source.join("session.lock");
    let metadata = fs::symlink_metadata(&lock_path).with_context(|| {
        format!(
            "Minecraft world import requires an existing session.lock so source quiescence can be proven: {}",
            lock_path.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("Minecraft session.lock must be a regular file: {}", lock_path.display());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("cannot open Minecraft session.lock at {}", lock_path.display()))?;
    try_minecraft_session_lock(&file, exclusive).with_context(|| {
        format!(
            "Minecraft world is currently open or its session lock cannot be acquired; stop Minecraft before importing {}",
            source.display()
        )
    })?;
    Ok(MinecraftSourceQuiescence { file })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn try_minecraft_session_lock(file: &fs::File, exclusive: bool) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    #[repr(C)]
    struct Flock {
        l_type: i16,
        l_whence: i16,
        l_start: i64,
        l_len: i64,
        l_pid: i32,
    }

    const F_SETLK: i32 = 6;
    const F_RDLCK: i16 = 0;
    const F_WRLCK: i16 = 1;
    const SEEK_SET: i16 = 0;
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }

    let mut lock =
        Flock { l_type: if exclusive { F_WRLCK } else { F_RDLCK }, l_whence: SEEK_SET, l_start: 0, l_len: 0, l_pid: 0 };
    let result = unsafe { fcntl(file.as_raw_fd(), F_SETLK, &mut lock) };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn try_minecraft_session_lock(file: &fs::File, exclusive: bool) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    #[repr(C)]
    struct Flock {
        l_start: i64,
        l_len: i64,
        l_pid: i32,
        l_type: i16,
        l_whence: i16,
    }

    const F_SETLK: i32 = 8;
    const F_RDLCK: i16 = 1;
    const F_WRLCK: i16 = 3;
    const SEEK_SET: i16 = 0;
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }

    let mut lock =
        Flock { l_start: 0, l_len: 0, l_pid: 0, l_type: if exclusive { F_WRLCK } else { F_RDLCK }, l_whence: SEEK_SET };
    let result = unsafe { fcntl(file.as_raw_fd(), F_SETLK, &mut lock) };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn try_minecraft_session_lock(file: &fs::File, exclusive: bool) -> std::io::Result<()> {
    if exclusive {
        FileExt::try_lock_exclusive(file)?;
    } else {
        FileExt::try_lock_shared(file)?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", windows)))]
fn try_minecraft_session_lock(_file: &fs::File, _exclusive: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Minecraft-compatible session locking is not implemented on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn release_minecraft_session_lock(file: &fs::File) {
    use std::os::fd::AsRawFd;

    #[repr(C)]
    struct Flock {
        l_type: i16,
        l_whence: i16,
        l_start: i64,
        l_len: i64,
        l_pid: i32,
    }

    const F_SETLK: i32 = 6;
    const F_UNLCK: i16 = 2;
    const SEEK_SET: i16 = 0;
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    let mut lock = Flock { l_type: F_UNLCK, l_whence: SEEK_SET, l_start: 0, l_len: 0, l_pid: 0 };
    let _ = unsafe { fcntl(file.as_raw_fd(), F_SETLK, &mut lock) };
}

#[cfg(target_os = "macos")]
fn release_minecraft_session_lock(file: &fs::File) {
    use std::os::fd::AsRawFd;

    #[repr(C)]
    struct Flock {
        l_start: i64,
        l_len: i64,
        l_pid: i32,
        l_type: i16,
        l_whence: i16,
    }

    const F_SETLK: i32 = 8;
    const F_UNLCK: i16 = 2;
    const SEEK_SET: i16 = 0;
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    let mut lock = Flock { l_start: 0, l_len: 0, l_pid: 0, l_type: F_UNLCK, l_whence: SEEK_SET };
    let _ = unsafe { fcntl(file.as_raw_fd(), F_SETLK, &mut lock) };
}

#[cfg(windows)]
fn release_minecraft_session_lock(file: &fs::File) {
    let _ = FileExt::unlock(file);
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", windows)))]
fn release_minecraft_session_lock(_file: &fs::File) {}

fn local_member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|file| file.sync_all())
            .with_context(|| format!("cannot sync directory {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migration, runtime_installer::RuntimeInstaller};
    use std::{process::Command, str::FromStr, thread, time::Duration};

    const LOCK_HELPER_PATH_ENV: &str = "SWARMCRAFT_TEST_IMPORT_LOCK_PATH";
    const LOCK_HELPER_READY_ENV: &str = "SWARMCRAFT_TEST_IMPORT_LOCK_READY";
    const LOCK_HELPER_RELEASE_ENV: &str = "SWARMCRAFT_TEST_IMPORT_LOCK_RELEASE";

    fn source_world(root: &Path) -> PathBuf {
        let source = root.join("source-world");
        fs::create_dir_all(source.join("region")).unwrap();
        fs::write(source.join("level.dat"), b"existing-level-data\n").unwrap();
        fs::write(source.join("region/r.0.0.mca"), b"existing-region-data\n").unwrap();
        fs::write(source.join("session.lock"), b"swarmcraft-import-test\n").unwrap();
        source
    }

    fn request(source: PathBuf) -> ImportWorldRequest {
        ImportWorldRequest {
            source,
            name: "Imported Survival".into(),
            minecraft_version: "26.1.2".into(),
            fabric_loader_version: "0.19.3".into(),
            visibility: WorldVisibilityV1::Private,
            server_mod_jars: Vec::new(),
            confirm_no_server_mods: true,
        }
    }

    #[test]
    fn valid_import_is_atomic_restart_safe_and_leaves_source_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let source = source_world(temp.path());
        let level_before = fs::read(source.join("level.dat")).unwrap();
        let region_before = fs::read(source.join("region/r.0.0.mca")).unwrap();

        let result = import_world(&paths, &request(source.clone())).unwrap();
        assert_eq!(fs::read(source.join("level.dat")).unwrap(), level_before);
        assert_eq!(fs::read(source.join("region/r.0.0.mca")).unwrap(), region_before);

        let world = WorldId::from_str(&result.world_id).unwrap();
        let reopened = Storage::open(paths.root.clone()).unwrap();
        let metadata = reopened.load_world(world).unwrap();
        assert_eq!(metadata.display_name, "Imported Survival");
        let latest = reopened.latest_snapshot(world).unwrap().unwrap();
        reopened.verify_snapshot(&latest).unwrap();
        swarm_core::verify_snapshot_signature(&latest).unwrap();
        assert_eq!(latest.snapshot_number, 1);
        assert_eq!(latest.entries.len(), 3);

        let restored = temp.path().join("restored");
        reopened.restore_snapshot(&latest, &restored).unwrap();
        assert_eq!(fs::read(restored.join("level.dat")).unwrap(), level_before);
        assert_eq!(fs::read(restored.join("region/r.0.0.mca")).unwrap(), region_before);

        // Import carries world data and exact compatibility only. Runtime paths
        // and EULA remain machine-local setup owned by Runtime Wizard.
        assert!(migration::load_runtime_config(&paths, world).is_err());
        let runtime = RuntimeInstaller::new(&paths, &reopened).inspect(world).unwrap();
        assert!(!runtime.eula_accepted);
        assert!(!runtime.launch_configured);
    }

    #[test]
    fn recovery_copy_preserves_exact_bytes_and_starts_a_separate_private_identity() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let original = import_world(&paths, &request(source_world(temp.path()))).unwrap();
        let world: WorldId = original.world_id.parse().unwrap();
        let storage = Storage::open(paths.root.clone()).unwrap();
        let original_snapshot = storage.latest_snapshot(world).unwrap().unwrap();
        let original_epoch = storage.load_epoch_record(world).unwrap();
        let original_membership = storage.load_membership_record(world).unwrap();
        let original_config = storage.load_world_config(world).unwrap();
        let mut recovery = RecoverCopyRequest {
            source_world: world,
            expected_checkpoint: original_snapshot.manifest_hash().unwrap(),
            name: Some("Survival backup".into()),
            confirm_new_identity: false,
        };
        assert!(recover_copy(&paths, &recovery).is_err());
        recovery.confirm_new_identity = true;
        let expected = recovery.expected_checkpoint;
        recovery.expected_checkpoint = Hash32([0x55; 32]);
        assert!(recover_copy(&paths, &recovery).is_err());
        recovery.expected_checkpoint = expected;
        for faults in [
            ImportFaults { after_snapshot_commit: true, before_publication: false },
            ImportFaults { after_snapshot_commit: false, before_publication: true },
        ] {
            assert!(recover_copy_inner(&paths, &recovery, faults).is_err());
            assert_eq!(storage.list_worlds().unwrap().len(), 1);
        }
        let result = recover_copy(&paths, &recovery).unwrap();
        let copy: WorldId = result.world_id.parse().unwrap();
        assert_ne!(copy, world);
        assert_eq!(result.source_world_id, original.world_id);
        assert_eq!(result.source_snapshot_hash, original.snapshot_hash);
        assert_eq!(result.display_name, "Recovered: Survival backup");
        let new_snapshot = storage.latest_snapshot(copy).unwrap().unwrap();
        assert_eq!(new_snapshot.entries, original_snapshot.entries);
        assert_eq!(new_snapshot.state_root, original_snapshot.state_root);
        assert_ne!(new_snapshot.manifest_hash().unwrap(), original_snapshot.manifest_hash().unwrap());
        assert_eq!(new_snapshot.epoch, world_genesis::GENESIS_EPOCH);
        assert!(new_snapshot.previous_snapshot_hash.is_none());
        storage.verify_snapshot(&new_snapshot).unwrap();
        swarm_core::verify_snapshot_signature(&new_snapshot).unwrap();
        let config = storage.load_world_config(copy).unwrap();
        assert_eq!(config.visibility, WorldVisibilityV1::Private);
        assert_eq!(config.compatibility, original_config.compatibility);
        assert!(!config.authority_policy.allow_solo_advancement);
        assert!(config.presentation.description.contains(&original.world_id));
        assert!(config.presentation.description.contains(&original.snapshot_hash));
        assert!(crate::hosting_budget::load_consent(&paths, copy).unwrap().is_none());
        assert!(migration::load_runtime_config(&paths, copy).is_err());
        assert!(!paths.root.join("runtime").join(copy.to_hex()).exists());
        assert_eq!(storage.load_epoch_record(world).unwrap(), original_epoch);
        assert_eq!(storage.load_membership_record(world).unwrap(), original_membership);
        assert_eq!(storage.load_world_config(world).unwrap(), original_config);
        assert_eq!(storage.latest_snapshot(world).unwrap().unwrap(), original_snapshot);
        assert_eq!(storage.list_worlds().unwrap().len(), 2);
        let restored = temp.path().join("recovered-bytes");
        storage.restore_snapshot(&new_snapshot, &restored).unwrap();
        assert_eq!(fs::read(restored.join("level.dat")).unwrap(), b"existing-level-data\n");
        assert_eq!(fs::read(restored.join("region/r.0.0.mca")).unwrap(), b"existing-region-data\n");
    }

    #[test]
    fn recovery_copy_refuses_corrupt_source_without_publishing_a_world() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let original = import_world(&paths, &request(source_world(temp.path()))).unwrap();
        let world: WorldId = original.world_id.parse().unwrap();
        let storage = Storage::open(paths.root.clone()).unwrap();
        let snapshot = storage.latest_snapshot(world).unwrap().unwrap();
        let blob = &snapshot.entries[0].blob;
        let extension = match blob.encoding {
            swarm_protocol::BlobEncoding::Raw => "raw",
            swarm_protocol::BlobEncoding::Zstd => "zst",
        };
        let path = storage.world_dir(world).join("blobs").join(format!("{}.{extension}", blob.hash.to_hex()));
        fs::write(path, b"corrupt").unwrap();
        assert!(recover_copy(
            &paths,
            &RecoverCopyRequest {
                source_world: world,
                expected_checkpoint: snapshot.manifest_hash().unwrap(),
                name: None,
                confirm_new_identity: true,
            }
        )
        .is_err());
        assert_eq!(storage.list_worlds().unwrap().len(), 1);
        assert_eq!(storage.latest_snapshot(world).unwrap().unwrap(), snapshot);
    }

    #[test]
    fn invalid_directory_and_unknown_compatibility_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let invalid = temp.path().join("not-a-world");
        fs::create_dir_all(&invalid).unwrap();
        assert!(import_world(&paths, &request(invalid)).is_err());

        let source = source_world(temp.path());
        let mut unknown = request(source);
        unknown.minecraft_version = "unknown".into();
        assert!(import_world(&paths, &unknown).is_err());
        assert!(Storage::open(paths.root.clone()).unwrap().list_worlds().unwrap().is_empty());
    }

    #[test]
    fn unsupported_bridge_tuple_is_rejected_before_publication() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let source = source_world(temp.path());
        let mut unsupported = request(source);
        unsupported.minecraft_version = "26.2".into();
        let error = import_world(&paths, &unsupported).unwrap_err();
        assert!(error.to_string().contains("shipped SwarmCraft Fabric adapter"));
        assert!(Storage::open(paths.root.clone()).unwrap().list_worlds().unwrap().is_empty());
    }

    #[test]
    fn import_requires_explicit_server_mod_compatibility_statement() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let source = source_world(temp.path());
        let mut ambiguous = request(source);
        ambiguous.confirm_no_server_mods = false;
        let error = import_world(&paths, &ambiguous).unwrap_err();
        assert!(error.to_string().contains("server-mod compatibility is unknown"));
    }

    #[test]
    fn minecraft_session_lock_helper_process() {
        let Ok(lock_path) = std::env::var(LOCK_HELPER_PATH_ENV) else {
            return;
        };
        let ready = PathBuf::from(std::env::var(LOCK_HELPER_READY_ENV).unwrap());
        let release = PathBuf::from(std::env::var(LOCK_HELPER_RELEASE_ENV).unwrap());
        let file = OpenOptions::new().read(true).write(true).open(lock_path).unwrap();
        try_minecraft_session_lock(&file, true).unwrap();
        fs::write(&ready, b"locked\n").unwrap();
        while !release.exists() {
            thread::sleep(Duration::from_millis(10));
        }
        release_minecraft_session_lock(&file);
    }

    #[test]
    fn import_refuses_source_while_minecraft_session_lock_is_held() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let source = source_world(temp.path());
        let ready = temp.path().join("lock-ready");
        let release = temp.path().join("lock-release");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("minecraft_session_lock_helper_process")
            .arg("--nocapture")
            .env(LOCK_HELPER_PATH_ENV, source.join("session.lock"))
            .env(LOCK_HELPER_READY_ENV, &ready)
            .env(LOCK_HELPER_RELEASE_ENV, &release)
            .spawn()
            .unwrap();

        for _ in 0..500 {
            if ready.exists() {
                break;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("session-lock helper exited before acquiring the lock: {status}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "session-lock helper never acquired the lock");

        let error = import_world(&paths, &request(source.clone())).unwrap_err();
        assert!(error.to_string().contains("currently open") || error.to_string().contains("session lock"));

        fs::write(&release, b"release\n").unwrap();
        assert!(child.wait().unwrap().success());
        assert!(import_world(&paths, &request(source)).is_ok());
    }

    #[test]
    fn interrupted_import_never_exposes_half_world_and_retry_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let source = source_world(temp.path());
        let req = request(source.clone());
        let faults = ImportFaults { after_snapshot_commit: true, ..ImportFaults::default() };

        assert!(import_world_inner(&paths, &req, faults).is_err());
        assert!(Storage::open(paths.root.clone()).unwrap().list_worlds().unwrap().is_empty());
        assert_eq!(fs::read(source.join("level.dat")).unwrap(), b"existing-level-data\n");

        let result = import_world(&paths, &req).unwrap();
        let world = WorldId::from_str(&result.world_id).unwrap();
        assert!(Storage::open(paths.root.clone()).unwrap().load_world(world).is_ok());
    }

    #[test]
    fn publication_failure_injection_leaves_no_visible_world() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let source = source_world(temp.path());
        let req = request(source);
        let faults = ImportFaults { before_publication: true, ..ImportFaults::default() };
        assert!(import_world_inner(&paths, &req, faults).is_err());
        assert!(Storage::open(paths.root.clone()).unwrap().list_worlds().unwrap().is_empty());
    }

    #[test]
    fn importing_same_source_twice_never_overwrites_first_world() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let source = source_world(temp.path());
        let req = request(source);
        let first = import_world(&paths, &req).unwrap();
        let second = import_world(&paths, &req).unwrap();
        assert_ne!(first.world_id, second.world_id);
        let reopened = Storage::open(paths.root.clone()).unwrap();
        assert_eq!(reopened.list_worlds().unwrap().len(), 2);
    }
}
