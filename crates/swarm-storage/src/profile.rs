//! Local reader compatibility floor. It does not change wire or world schema,
//! authorize hosting, or attempt to rewrite historical signed records.
use crate::{transaction::durable_atomic_write, StorageError};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::Path,
};
use swarm_protocol::{WorldId, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION};

const FORMAT_FILE: &str = "profile-compatibility.json";
const LOCK_FILE: &str = ".profile-compatibility.lock";
const FORMAT_SCHEMA: u16 = 1;
// Independent of the application version: published 0.5.1 predates this
// guard and must not be treated as equivalent to a current candidate.
pub const PROFILE_READER_GENERATION: u16 = 1;
const MAX_FORMAT_BYTES: u64 = 16 * 1024;
const MAX_WORLD_METADATA_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileCompatibilityV1 {
    pub format_schema: u16,
    pub minimum_reader_generation: u16,
    pub storage_schema_version: u16,
    pub protocol_version: u16,
}

impl ProfileCompatibilityV1 {
    fn current() -> Self {
        Self {
            format_schema: FORMAT_SCHEMA,
            minimum_reader_generation: PROFILE_READER_GENERATION,
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            protocol_version: PROTOCOL_VERSION,
        }
    }

    fn validate(&self) -> Result<(), StorageError> {
        if self != &Self::current() {
            return Err(StorageError::IncompatibleProfile {
                detail: format!(
                    "profile format {}, minimum reader generation {}, storage schema {}, protocol {}; this build requires {}/{}/{}/{}",
                    self.format_schema, self.minimum_reader_generation, self.storage_schema_version,
                    self.protocol_version, FORMAT_SCHEMA, PROFILE_READER_GENERATION,
                    STORAGE_SCHEMA_VERSION, PROTOCOL_VERSION
                ),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileInspection {
    pub initialized: bool,
    pub world_metadata_count: usize,
    pub compatibility: Option<ProfileCompatibilityV1>,
    pub supported_reader_generation: u16,
    pub storage_schema_version: u16,
    pub protocol_version: u16,
}

fn io_error(path: &Path, source: io::Error) -> StorageError {
    StorageError::Io { path: path.to_path_buf(), source }
}

fn incompatible(detail: impl Into<String>) -> StorageError {
    StorageError::IncompatibleProfile { detail: detail.into() }
}

fn regular_metadata(path: &Path, directory: bool) -> Result<Option<fs::Metadata>, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || (directory && !metadata.is_dir())
                || (!directory && !metadata.is_file())
            {
                return Err(incompatible(format!("unsafe profile entry {}", path.display())));
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path, error)),
    }
}

fn read_bounded_json<T: DeserializeOwned>(path: &Path, max_bytes: u64) -> Result<T, StorageError> {
    let metadata = regular_metadata(path, false)?.ok_or_else(|| io_error(path, io::ErrorKind::NotFound.into()))?;
    if metadata.len() > max_bytes {
        return Err(incompatible(format!("oversized profile metadata {}", path.display())));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| io_error(path, error))?
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(path, error))?;
    if bytes.len() as u64 > max_bytes {
        return Err(incompatible(format!("oversized profile metadata {}", path.display())));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| incompatible(format!("invalid profile metadata {}: {error}", path.display())))
}

#[derive(Deserialize)]
struct WorldSchema {
    storage_schema_version: u16,
    genesis: GenesisSchema,
}

#[derive(Deserialize)]
struct GenesisSchema {
    protocol_version: u16,
}

pub(crate) fn validate_world_schema(storage: u16, protocol: u16) -> Result<(), StorageError> {
    if storage != STORAGE_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedStorageSchema { found: storage, supported: STORAGE_SCHEMA_VERSION });
    }
    if protocol != PROTOCOL_VERSION {
        return Err(StorageError::UnsupportedProtocol(protocol));
    }
    Ok(())
}

/// Read-only preflight. Never creates files, initializes an identity, or
/// invokes snapshot/retention recovery. A partially staged world without
/// published world.json is not a committed world.
pub fn inspect(root: &Path) -> Result<ProfileInspection, StorageError> {
    let mut report = ProfileInspection {
        initialized: false,
        world_metadata_count: 0,
        compatibility: None,
        supported_reader_generation: PROFILE_READER_GENERATION,
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        protocol_version: PROTOCOL_VERSION,
    };
    if regular_metadata(root, true)?.is_none() {
        return Ok(report);
    }
    report.initialized = fs::read_dir(root)
        .map_err(|error| io_error(root, error))?
        .next()
        .transpose()
        .map_err(|error| io_error(root, error))?
        .is_some();
    let marker = root.join(FORMAT_FILE);
    if regular_metadata(&marker, false)?.is_some() {
        let format: ProfileCompatibilityV1 = read_bounded_json(&marker, MAX_FORMAT_BYTES)?;
        format.validate()?;
        report.compatibility = Some(format);
    }
    let worlds = root.join("worlds");
    if regular_metadata(&worlds, true)?.is_none() {
        return Ok(report);
    }
    for entry in fs::read_dir(&worlds).map_err(|error| io_error(&worlds, error))? {
        let entry = entry.map_err(|error| io_error(&worlds, error))?;
        if entry.file_name().to_str().and_then(|name| name.parse::<WorldId>().ok()).is_none() {
            continue;
        }
        let path = entry.path();
        if regular_metadata(&path, true)?.is_none() {
            continue;
        }
        let metadata_dir = path.join("metadata");
        if regular_metadata(&metadata_dir, true)?.is_none() {
            continue;
        }
        let file = metadata_dir.join("world.json");
        if regular_metadata(&file, false)?.is_none() {
            continue;
        }
        let schema: WorldSchema = read_bounded_json(&file, MAX_WORLD_METADATA_BYTES)?;
        validate_world_schema(schema.storage_schema_version, schema.genesis.protocol_version)?;
        report.world_metadata_count += 1;
    }
    Ok(report)
}

/// Guard all supported opens before recovery. Activation is a single
/// atomic metadata publication, not a rewrite of a world, identity, or
/// consent. A crash before publication leaves the legacy profile readable
/// by this build; a crash after publication leaves a complete reader floor.
pub(crate) fn activate(root: &Path) -> Result<(), StorageError> {
    if inspect(root)?.compatibility.is_some() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(root).map_err(|error| io_error(root, error))?;
    let lock_path = root.join(LOCK_FILE);
    regular_metadata(&lock_path, false)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| io_error(&lock_path, error))?;
    FileExt::lock_exclusive(&lock).map_err(|error| io_error(&lock_path, error))?;
    // A concurrent supported opener may have already published the floor.
    // Do not overwrite either its marker or an incompatible marker.
    if inspect(root)?.compatibility.is_none() {
        durable_atomic_write(&root.join(FORMAT_FILE), &serde_json::to_vec_pretty(&ProfileCompatibilityV1::current())?)?;
    }
    // Keep stable inode; process exit releases the OS lock.
    Ok(())
}

/// Historical releases predate this reader floor and cannot be trusted
/// to enforce it, even if their displayed app version is 0.5.1.
/// This preflight is mandatory for the supported rollback workflow, not
/// a claim that an arbitrary unmodified historical executable obeys it.
pub fn check_historical_target(root: &Path) -> Result<ProfileInspection, StorageError> {
    let report = inspect(root)?;
    if report.initialized {
        return Err(incompatible(
            "unsupported downgrade into an initialized profile; stop all SwarmCraft processes, preserve the current profile and use verified checkpoint export/data-only recovery or an offline pre-upgrade backup; never open or reconnect the same live identity in the historical release",
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Storage, WorldMetadataV1};
    use swarm_protocol::WorldGenesisV1;

    fn legacy_metadata(root: &Path, schema: u16, protocol: u16) -> (WorldId, std::path::PathBuf) {
        let genesis = WorldGenesisV1 {
            protocol_version: protocol,
            minecraft_version: "26.1.2".into(),
            fabric_loader_version: "0.19.3".into(),
            compatibility_fingerprint: swarm_protocol::Hash32([1; 32]),
            creation_nonce: [2; 32],
            creator_public_key: [3; 32],
            initial_membership: vec![swarm_protocol::peer_id_from_public_key(&[3; 32])],
        };
        let world = genesis.world_id().unwrap();
        let path = root.join("worlds").join(world.to_hex()).join("metadata").join("world.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&WorldMetadataV1 {
                storage_schema_version: schema,
                display_name: "legacy".into(),
                world_id: world,
                genesis,
            })
            .unwrap(),
        )
        .unwrap();
        (world, path)
    }

    #[test]
    fn preflight_read_only_and_atomic_activation_preserves_legacy_world() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("legacy");
        let (world, path) = legacy_metadata(&root, STORAGE_SCHEMA_VERSION, PROTOCOL_VERSION);
        let before = fs::read(&path).unwrap();
        let initial = inspect(&root).unwrap();
        assert!(initial.initialized && initial.compatibility.is_none());
        assert_eq!(initial.world_metadata_count, 1);
        assert!(!root.join(FORMAT_FILE).exists());
        assert!(check_historical_target(&root).is_err());
        let store = Storage::open(&root).unwrap();
        assert_eq!(store.load_world(world).unwrap().display_name, "legacy");
        let marker = fs::read(root.join(FORMAT_FILE)).unwrap();
        assert!(inspect(&root).unwrap().compatibility.is_some());
        drop(store);
        Storage::open(&root).unwrap();
        assert_eq!(fs::read(root.join(FORMAT_FILE)).unwrap(), marker);
        assert_eq!(fs::read(path).unwrap(), before);
        assert!(check_historical_target(&root).is_err());
    }

    #[test]
    fn unknown_storage_or_protocol_denied_before_floor_or_recovery_mutation() {
        for (schema, protocol) in
            [(STORAGE_SCHEMA_VERSION + 1, PROTOCOL_VERSION), (STORAGE_SCHEMA_VERSION, PROTOCOL_VERSION + 1)]
        {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let (_, metadata) = legacy_metadata(root, schema, protocol);
            let before = fs::read(&metadata).unwrap();
            assert!(inspect(root).is_err());
            let error = Storage::open(root).unwrap_err();
            assert!(matches!(
                error,
                StorageError::UnsupportedStorageSchema { .. } | StorageError::UnsupportedProtocol(_)
            ));
            assert!(!root.join(FORMAT_FILE).exists());
            assert!(!root.join(LOCK_FILE).exists());
            assert_eq!(fs::read(metadata).unwrap(), before);
        }
    }

    #[test]
    fn unsupported_or_truncated_floor_cannot_be_overwritten_or_initialize_worlds() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let marker = root.join(FORMAT_FILE);
        let mut future = ProfileCompatibilityV1::current();
        future.minimum_reader_generation += 1;
        for bytes in [serde_json::to_vec(&future).unwrap(), b"{\"format_schema\":".to_vec(), b"{}".to_vec()] {
            fs::write(&marker, &bytes).unwrap();
            assert!(inspect(root).is_err());
            assert!(Storage::open(root).is_err());
            assert_eq!(fs::read(&marker).unwrap(), bytes);
            assert!(!root.join("worlds").exists());
            assert!(!root.join(LOCK_FILE).exists());
        }
    }

    #[test]
    fn interrupted_floor_staging_is_not_promoted_or_used_as_a_reader_floor() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let (_, metadata) = legacy_metadata(root, STORAGE_SCHEMA_VERSION, PROTOCOL_VERSION);
        let original = fs::read(&metadata).unwrap();
        // Exact same temporary-file allocator as durable_atomic_write. Simulate
        // a killed writer before the atomic rename, with incomplete bytes.
        let (staging, mut file) = crate::transaction::create_unique_temp(root, "atomic", "tmp").unwrap();
        use std::io::Write;
        file.write_all(b"{\"format_schema\":").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert!(inspect(root).unwrap().compatibility.is_none());
        let store = Storage::open(root).unwrap();
        assert_eq!(inspect(root).unwrap().compatibility, Some(ProfileCompatibilityV1::current()));
        assert_eq!(fs::read(staging).unwrap(), b"{\"format_schema\":");
        assert_eq!(fs::read(metadata).unwrap(), original);
        drop(store);
        Storage::open(root).unwrap();
    }

    #[test]
    fn historical_target_only_allows_new_empty_root_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fresh");
        assert!(!check_historical_target(&root).unwrap().initialized);
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        assert!(!check_historical_target(&root).unwrap().initialized);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::write(root.join("existing-data"), b"keep").unwrap();
        assert!(check_historical_target(&root).is_err());
        assert_eq!(fs::read(root.join("existing-data")).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_marker_and_world_metadata_are_denied_without_following_targets() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("profile");
        let (_, metadata) = legacy_metadata(&root, STORAGE_SCHEMA_VERSION, PROTOCOL_VERSION);
        let outside = temp.path().join("outside");
        fs::write(&outside, serde_json::to_vec(&ProfileCompatibilityV1::current()).unwrap()).unwrap();
        let before = fs::read(&outside).unwrap();
        symlink(&outside, root.join(FORMAT_FILE)).unwrap();
        assert!(Storage::open(&root).is_err());
        assert_eq!(fs::read(&outside).unwrap(), before);
        fs::remove_file(root.join(FORMAT_FILE)).unwrap();
        fs::remove_file(&metadata).unwrap();
        symlink(&outside, &metadata).unwrap();
        assert!(Storage::open(&root).is_err());
        assert!(!root.join(FORMAT_FILE).exists());
    }

    #[test]
    fn already_open_storage_cannot_load_or_silently_hide_future_schema_world() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let (world, path) = legacy_metadata(root, STORAGE_SCHEMA_VERSION, PROTOCOL_VERSION);
        let store = Storage::open(root).unwrap();
        let mut metadata: WorldMetadataV1 = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        metadata.storage_schema_version += 1;
        fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert!(matches!(store.load_world(world), Err(StorageError::UnsupportedStorageSchema { .. })));
        assert!(matches!(store.list_worlds(), Err(StorageError::UnsupportedStorageSchema { .. })));
        assert!(store.create_world(&metadata).is_err());
    }
}
