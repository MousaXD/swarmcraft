//! Shared storage-only V1 baseline for the independent #34 experiments.
use std::{collections::BTreeMap, error::Error, fs, io, path::Path};
use swarm_protocol::{PeerId, SnapshotManifestV1, WorldId};
use swarm_storage::{SnapshotContext, Storage};
use walkdir::WalkDir;

pub(super) type LabResult<T> = Result<T, Box<dyn Error>>;
pub(super) const WORLD: WorldId = WorldId([0x94; 32]);
pub(super) const CHUNK: usize = 256 * 1024;

pub(super) fn require(condition: bool, message: &str) -> LabResult<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message).into())
    }
}

pub(super) fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            if i % 4096 >= 3072 {
                0
            } else {
                (state >> 19) as u8
            }
        })
        .collect()
}

pub(super) fn capture(
    storage: &Storage,
    root: &Path,
    previous: Option<&SnapshotManifestV1>,
) -> LabResult<SnapshotManifestV1> {
    let number = previous.map_or(1, |manifest| manifest.snapshot_number + 1);
    let mut publication = storage.snapshot_directory(
        root,
        SnapshotContext {
            world: WORLD,
            snapshot_number: number,
            epoch: 1,
            sequence: number,
            previous_snapshot_hash: previous.map(SnapshotManifestV1::manifest_hash).transpose()?,
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [2; 32],
        },
    )?;
    // This is a storage-only fixture, NOT a valid authority signature.
    publication.signature = vec![0; 64];
    storage.commit_snapshot(&publication)?;
    Ok(publication.manifest().clone())
}

pub(super) fn copy(source: &Storage, target: &Storage, manifest: &SnapshotManifestV1) -> LabResult<u64> {
    let mut transferred = postcard::to_allocvec(manifest)?.len() as u64;
    for descriptor in target.missing_blobs(manifest) {
        let mut offset = target.partial_blob_offset(WORLD, &descriptor)?;
        loop {
            let (bytes, finished) = source.read_encoded_blob_chunk(WORLD, &descriptor, offset, CHUNK)?;
            transferred += bytes.len() as u64;
            offset = target.receive_blob_chunk(WORLD, &descriptor, offset, &bytes, finished)?;
            if finished {
                break;
            }
        }
    }
    target.finalize_replica(manifest)?;
    Ok(transferred)
}

pub(super) fn tree(root: &Path) -> LabResult<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        require(entry.file_type().is_file(), "non-regular fixture file")?;
        total += entry.metadata()?.len();
        require(total <= 64 * 1024 * 1024 && files.len() < 1024, "fixture tree exceeds lab bound")?;
        let relative = entry
            .path()
            .strip_prefix(root)?
            .components()
            .map(|part| part.as_os_str().to_str().ok_or_else(|| io::Error::other("non-UTF8 fixture file")))
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        files.insert(relative, fs::read(entry.path())?);
    }
    Ok(files)
}
