//! Sabathon #33: controlled, data-only checkpoint experiment.
//!
//! This integration test deliberately has no production call sites. The lab
//! uses isolated temporary stores and a distinct manifest/hash namespace,
//! never a signed world, live save barrier, lease, or canonical checkpoint.
//! V1's actual streaming capture/commit/restore is the comparison baseline.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use swarm_protocol::{
    snapshot_state_root, BlobDescriptor, BlobEncoding, Hash32, PeerId, SnapshotEntry, SnapshotManifestV1, WorldId,
    BLOB_HASH_DOMAIN, PROTOCOL_VERSION,
};
use swarm_storage::{streaming::validate_manifest_shape, SnapshotContext, Storage};
use tempfile::{NamedTempFile, TempDir};
use walkdir::WalkDir;

#[path = "support/checkpoint_schedule_lab.rs"]
mod checkpoint_schedule_lab;

type LabResult<T> = Result<T, Box<dyn Error>>;
const WORLD: WorldId = WorldId([0x93; 32]);
const LAB_DOMAIN: &[u8] = b"swarmcraft/research/block-checkpoint/v1\0";
const BLOCK: usize = 256 * 1024;
const MAX_FILES: usize = 4096;
const MAX_BLOCKS: usize = 4096;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_TREE_BYTES: u64 = 512 * 1024 * 1024;

fn require(condition: bool, message: &str) -> LabResult<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message).into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Pin {
    number: u64,
    hash: Hash32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileRecord {
    path: String,
    length: u64,
    whole_hash: Hash32,
    blocks: Vec<BlobDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Checkpoint {
    version: u16,
    block_bytes: u32,
    number: u64,
    previous: Option<Pin>,
    files: Vec<FileRecord>,
}

impl Checkpoint {
    fn encoded(&self) -> LabResult<Vec<u8>> {
        let bytes = postcard::to_allocvec(self)?;
        require(bytes.len() <= MAX_MANIFEST_BYTES, "lab manifest exceeds byte limit")?;
        Ok(bytes)
    }

    fn pin(&self) -> LabResult<Pin> {
        Ok(Pin { number: self.number, hash: Hash32::from_domain_bytes(LAB_DOMAIN, &self.encoded()?) })
    }

    fn descriptors(&self) -> LabResult<BTreeMap<Hash32, BlobDescriptor>> {
        let mut unique = BTreeMap::new();
        for descriptor in self.files.iter().flat_map(|file| &file.blocks) {
            if let Some(previous) = unique.insert(descriptor.hash, descriptor.clone()) {
                require(previous == *descriptor, "conflicting descriptors for block identity")?;
            }
        }
        Ok(unique)
    }

    fn validate(&self) -> LabResult<()> {
        require(self.version == 1 && self.block_bytes == BLOCK as u32, "unsupported lab format")?;
        require(self.number > 0, "invalid lab checkpoint number")?;
        match self.previous {
            Some(previous) => require(
                previous.number.checked_add(1) == Some(self.number) && previous.hash != Hash32::default(),
                "invalid lab predecessor",
            )?,
            None => require(self.number == 1, "missing lab predecessor")?,
        }
        require(self.files.len() <= MAX_FILES, "too many lab files")?;
        require(self.files.windows(2).all(|pair| pair[0].path < pair[1].path), "unordered or duplicate file paths")?;
        let mut total = 0u64;
        let mut blocks = 0usize;
        let mut entries = Vec::with_capacity(self.files.len());
        let mut names = BTreeSet::new();
        for file in &self.files {
            total = total.checked_add(file.length).ok_or_else(|| io::Error::other("tree length overflow"))?;
            require(total <= MAX_TREE_BYTES, "lab tree too large")?;
            blocks = blocks.checked_add(file.blocks.len()).ok_or_else(|| io::Error::other("block count overflow"))?;
            require(blocks <= MAX_BLOCKS, "too many blocks")?;
            let expected = file.length.div_ceil(BLOCK as u64);
            require(file.blocks.len() as u64 == expected, "incorrect number of blocks")?;
            let mut remaining = file.length;
            for block in &file.blocks {
                require(block.uncompressed_size == remaining.min(BLOCK as u64), "invalid block length")?;
                require(
                    block.encoding == BlobEncoding::Zstd
                        && block.encoded_size > 0
                        && block.encoded_size <= (BLOCK * 2 + 1024) as u64,
                    "invalid encoded block bounds",
                )?;
                remaining -= block.uncompressed_size;
            }
            require(remaining == 0, "incomplete file")?;
            names.insert(file.path.to_ascii_lowercase());
            // Reuse V1's portable-path validation on a synthetic full-file
            // descriptor. This is not a V1 canonical state root or signature.
            entries.push(SnapshotEntry {
                path: file.path.clone(),
                blob: BlobDescriptor {
                    hash: file.whole_hash,
                    uncompressed_size: file.length,
                    encoded_size: file.length,
                    encoding: BlobEncoding::Raw,
                },
            });
        }
        for name in &names {
            let mut prefix = name.as_str();
            while let Some((parent, _)) = prefix.rsplit_once('/') {
                require(!names.contains(parent), "file/directory namespace collision")?;
                prefix = parent;
            }
        }
        let shape = SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WORLD,
            snapshot_number: self.number,
            epoch: 1,
            sequence: self.number,
            previous_snapshot_hash: None,
            state_root: snapshot_state_root(&entries)?,
            entries,
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [2; 32],
            signature: vec![],
        };
        validate_manifest_shape(&shape)?;
        self.descriptors()?;
        Ok(())
    }
}

struct LabStore {
    storage: Storage,
    manifests: PathBuf,
}

impl LabStore {
    fn open(root: &Path) -> LabResult<Self> {
        let storage = Storage::open(root.join("isolated-blobs"))?;
        let manifests = root.join("research-manifests");
        fs::create_dir_all(&manifests)?;
        Ok(Self { storage, manifests })
    }

    fn path(&self, number: u64) -> PathBuf {
        self.manifests.join(format!("{number:020}.lab"))
    }

    fn load(&self, expected: Pin) -> LabResult<Checkpoint> {
        let path = self.path(expected.number);
        require(fs::symlink_metadata(&path)?.is_file(), "manifest is not a regular file")?;
        let mut bytes = Vec::new();
        File::open(path)?.take(MAX_MANIFEST_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        require(bytes.len() <= MAX_MANIFEST_BYTES, "oversized lab manifest")?;
        require(Hash32::from_domain_bytes(LAB_DOMAIN, &bytes) == expected.hash, "lab manifest pin mismatch")?;
        let manifest: Checkpoint = postcard::from_bytes(&bytes)?;
        require(manifest.encoded()? == bytes && manifest.number == expected.number, "noncanonical lab manifest")?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn capture(&self, source: &Path, previous: Option<Pin>) -> LabResult<Checkpoint> {
        // Fixture is quiescent; no attempt to trust mtimes, dirty hints, or
        // read a mutable Minecraft runtime. Every source byte is hashed.
        let mut inventory = match previous {
            Some(pin) => self.load(pin)?.descriptors()?,
            None => BTreeMap::new(),
        };
        let number = match previous {
            Some(pin) => pin.number.checked_add(1).ok_or_else(|| io::Error::other("checkpoint number exhausted"))?,
            None => 1,
        };
        let paths = file_paths(source)?;
        let mut files = Vec::new();
        let mut total = 0u64;
        let mut block_count = 0usize;
        let mut buffer = vec![0; BLOCK];
        for (path, absolute) in paths {
            let mut file = File::open(absolute)?;
            let mut hasher = blake3::Hasher::new();
            hasher.update(BLOB_HASH_DOMAIN);
            let mut length = 0u64;
            let mut blocks = Vec::new();
            loop {
                let mut count = 0;
                while count < buffer.len() {
                    let read = file.read(&mut buffer[count..])?;
                    if read == 0 {
                        break;
                    }
                    count += read;
                }
                if count == 0 {
                    break;
                }
                total += count as u64;
                length += count as u64;
                block_count += 1;
                require(total <= MAX_TREE_BYTES && block_count <= MAX_BLOCKS, "capture exceeds lab bounds")?;
                let bytes = &buffer[..count];
                hasher.update(bytes);
                let hash = BlobDescriptor::hash_uncompressed(bytes);
                let descriptor = if let Some(existing) = inventory.get(&hash) {
                    require(existing.uncompressed_size == count as u64, "block collision")?;
                    existing.clone()
                } else {
                    // Actual V1 bounded Zstd encoding and durable blob write;
                    // the lab is single-owner, without concurrent GC.
                    let descriptor = self.storage.put_blob(WORLD, bytes)?;
                    require(
                        descriptor.hash == hash && descriptor.uncompressed_size == count as u64,
                        "unexpected stored block",
                    )?;
                    inventory.insert(hash, descriptor.clone());
                    descriptor
                };
                blocks.push(descriptor);
            }
            files.push(FileRecord { path, length, whole_hash: Hash32(*hasher.finalize().as_bytes()), blocks });
        }
        let manifest = Checkpoint { version: 1, block_bytes: BLOCK as u32, number, previous, files };
        self.publish(&manifest)?;
        Ok(manifest)
    }

    fn verify_file(&self, file: &FileRecord, mut output: impl Write) -> LabResult<()> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(BLOB_HASH_DOMAIN);
        let mut length = 0u64;
        for descriptor in &file.blocks {
            // read_blob validates encoded size, decoded size and BLAKE3.
            let bytes = self.storage.read_blob(WORLD, descriptor)?;
            length += bytes.len() as u64;
            hasher.update(&bytes);
            output.write_all(&bytes)?;
        }
        require(
            length == file.length && Hash32(*hasher.finalize().as_bytes()) == file.whole_hash,
            "reconstructed file digest mismatch",
        )
    }

    fn publish(&self, manifest: &Checkpoint) -> LabResult<()> {
        manifest.validate()?;
        if let Some(previous) = manifest.previous {
            self.load(previous)?;
        }
        // A reference alone is insufficient: check all block bytes and
        // ordered whole-file digests before an immutable manifest exists.
        for file in &manifest.files {
            self.verify_file(file, io::sink())?;
        }
        let bytes = manifest.encoded()?;
        let mut temporary = NamedTempFile::new_in(&self.manifests)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist_noclobber(self.path(manifest.number))?;
        #[cfg(unix)]
        File::open(&self.manifests)?.sync_all()?;
        Ok(())
    }

    fn restore(&self, pin: Pin, parent: &Path) -> LabResult<TempDir> {
        let manifest = self.load(pin)?;
        // A failed restoration never exposes an existing playable directory.
        // The only result returned is a fresh complete scratch tree.
        let result = TempDir::new_in(parent)?;
        for file in &manifest.files {
            let path = result.path().join(&file.path);
            fs::create_dir_all(path.parent().ok_or_else(|| io::Error::other("missing parent"))?)?;
            let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
            self.verify_file(file, &mut output)?;
            output.sync_all()?;
        }
        Ok(result)
    }
}

fn file_paths(root: &Path) -> LabResult<BTreeMap<String, PathBuf>> {
    require(fs::symlink_metadata(root)?.is_dir(), "source is not a directory")?;
    let mut paths = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        require(entry.file_type().is_file(), "source contains non-regular file")?;
        let path = entry
            .path()
            .strip_prefix(root)?
            .components()
            .map(|component| component.as_os_str().to_str().ok_or_else(|| io::Error::other("non-UTF8 path")))
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        require(paths.insert(path, entry.path().to_path_buf()).is_none(), "duplicate source file")?;
        require(paths.len() <= MAX_FILES, "too many source files")?;
    }
    Ok(paths)
}

fn copy_blob(source: &Storage, receiver: &Storage, descriptor: &BlobDescriptor) -> LabResult<u64> {
    if receiver.has_complete_blob(WORLD, descriptor) {
        return Ok(0);
    }
    let mut offset = receiver.partial_blob_offset(WORLD, descriptor)?;
    let mut transferred = 0;
    loop {
        let (bytes, finished) = source.read_encoded_blob_chunk(WORLD, descriptor, offset, BLOCK)?;
        transferred += bytes.len() as u64;
        offset = receiver.receive_blob_chunk(WORLD, descriptor, offset, &bytes, finished)?;
        if finished {
            return Ok(transferred);
        }
    }
}

fn baseline_capture(
    storage: &Storage,
    source: &Path,
    previous: Option<&SnapshotManifestV1>,
) -> LabResult<SnapshotManifestV1> {
    let number = previous.map_or(1, |manifest| manifest.snapshot_number + 1);
    let mut publication = storage.snapshot_directory(
        source,
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
    // Storage-only fixture, not a network identity or valid authority proof.
    publication.signature = vec![0; 64];
    storage.commit_snapshot(&publication)?;
    Ok(publication.manifest().clone())
}

fn baseline_copy(source: &Storage, receiver: &Storage, manifest: &SnapshotManifestV1) -> LabResult<(u64, usize)> {
    let mut bytes = 0;
    for descriptor in receiver.missing_blobs(manifest) {
        bytes += copy_blob(source, receiver, &descriptor)?;
    }
    receiver.finalize_replica(manifest)?;
    Ok((bytes, postcard::to_allocvec(manifest)?.len()))
}

fn lab_copy(source: &LabStore, receiver: &LabStore, pin: Pin) -> LabResult<(u64, usize)> {
    let manifest = source.load(pin)?;
    let mut bytes = 0;
    for descriptor in manifest.descriptors()?.values() {
        bytes += copy_blob(&source.storage, &receiver.storage, descriptor)?;
    }
    receiver.publish(&manifest)?;
    require(receiver.load(pin)? == manifest, "replica manifest did not retain the exact pin")?;
    Ok((bytes, manifest.encoded()?.len()))
}

fn exact_tree(left: &Path, right: &Path) -> LabResult<()> {
    let left = file_paths(left)?;
    let right = file_paths(right)?;
    require(left.keys().eq(right.keys()), "restored file set differs")?;
    for (name, source) in left {
        assert_eq!(fs::read(source)?, fs::read(&right[&name])?, "file differs: {name}");
    }
    Ok(())
}

fn tree_hashes(root: &Path) -> LabResult<BTreeMap<String, Hash32>> {
    file_paths(root)?
        .into_iter()
        .map(|(path, source)| Ok((path, BlobDescriptor::hash_uncompressed(&fs::read(source)?))))
        .collect()
}

fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut value = seed | 1;
    (0..len)
        .map(|i| {
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            // A repeatable mixture of opaque payload and sector padding. The
            // region-shaped bytes are not a validated Minecraft Anvil save.
            if i % 4096 >= 3072 {
                0
            } else {
                (value >> 19) as u8
            }
        })
        .collect()
}

const REGION_COUNT: usize = 8;
const REGION_BYTES: usize = 4 * 1024 * 1024;

fn region(root: &Path, index: usize) -> PathBuf {
    root.join(format!("region/r.{index}.0.mca"))
}

fn create_fixture(root: &Path) -> LabResult<()> {
    fs::create_dir_all(root.join("region"))?;
    fs::create_dir_all(root.join("data"))?;
    for index in 0..REGION_COUNT {
        fs::write(region(root, index), random_bytes(200 + index as u64 * 13, REGION_BYTES))?;
    }
    fs::write(root.join("level.dat"), b"level metadata")?;
    fs::write(root.join("data/duplicate-a.dat"), b"same content")?;
    fs::write(root.join("data/duplicate-b.dat"), b"same content")?;
    fs::write(root.join("data/empty.dat"), [])?;
    fs::write(root.join("data/deleted.dat"), b"retained only in first checkpoint")?;
    Ok(())
}

fn mutate_fixture(root: &Path, round: u64, dense: bool) -> LabResult<()> {
    for index in 0..REGION_COUNT {
        let seed = 10000 + round * 193 + index as u64 * 7;
        if dense {
            fs::write(region(root, index), random_bytes(seed, REGION_BYTES))?;
        } else {
            let mut file = OpenOptions::new().write(true).open(region(root, index))?;
            let offset = (round as usize * BLOCK + 4096 * (index + 1)) % REGION_BYTES;
            file.seek(SeekFrom::Start(offset as u64))?;
            file.write_all(&random_bytes(seed, 4096))?;
        }
    }
    fs::write(root.join("level.dat"), format!("level metadata {round}"))?;
    if round == 1 {
        fs::remove_file(root.join("data/deleted.dat"))?;
        fs::rename(root.join("data/duplicate-b.dat"), root.join("data/renamed.dat"))?;
        fs::write(root.join("data/added.dat"), b"new file")?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct Measurement {
    save: Duration,
    transfer: Duration,
    restore: Duration,
    payload: u64,
    manifest: usize,
}

impl Measurement {
    fn total_bytes(self) -> u64 {
        self.payload + self.manifest as u64
    }
    fn report(self, scenario: &str, method: &str) {
        eprintln!("checkpoint_lab scenario={scenario} method={method} save_ms={:.3} transfer_ms={:.3} restore_ms={:.3} payload_bytes={} manifest_bytes={} total_bytes={}",
            self.save.as_secs_f64()*1000., self.transfer.as_secs_f64()*1000., self.restore.as_secs_f64()*1000.,
            self.payload, self.manifest, self.total_bytes());
    }
}

#[test]
fn compare_real_v1_full_file_checkpoint_with_verified_block_checkpoint() -> LabResult<()> {
    let temp = tempfile::tempdir()?;
    let world = temp.path().join("quiescent-world");
    create_fixture(&world)?;
    let original = tree_hashes(&world)?;
    let baseline = Storage::open(temp.path().join("v1-source"))?;
    let baseline_replica = Storage::open(temp.path().join("v1-replica"))?;
    let lab = LabStore::open(&temp.path().join("lab-source"))?;
    let lab_replica = LabStore::open(&temp.path().join("lab-replica"))?;

    let mut previous_v1 = None;
    let mut previous_lab = None;
    let mut original_v1 = None;
    let mut original_lab = None;
    let mut sparse_measurements = Vec::new();
    for (round, scenario) in ["initial", "sparse-1", "sparse-2", "sparse-3", "dense"].iter().enumerate() {
        let dense = *scenario == "dense";
        if round > 0 {
            mutate_fixture(&world, round as u64, dense)?;
        }

        let run_v1 = || -> LabResult<(SnapshotManifestV1, Measurement, TempDir)> {
            let start = Instant::now();
            let manifest = baseline_capture(&baseline, &world, previous_v1.as_ref())?;
            let save = start.elapsed();
            let start = Instant::now();
            let (payload, manifest_bytes) = baseline_copy(&baseline, &baseline_replica, &manifest)?;
            let transfer = start.elapsed();
            let restored = TempDir::new_in(temp.path())?;
            let start = Instant::now();
            baseline_replica.restore_snapshot(&manifest, restored.path())?;
            let restore = start.elapsed();
            Ok((manifest, Measurement { save, transfer, restore, payload, manifest: manifest_bytes }, restored))
        };
        let run_lab = || -> LabResult<(Pin, Measurement, TempDir)> {
            let start = Instant::now();
            let manifest = lab.capture(&world, previous_lab)?;
            let save = start.elapsed();
            let pin = manifest.pin()?;
            let start = Instant::now();
            let (payload, manifest_bytes) = lab_copy(&lab, &lab_replica, pin)?;
            let transfer = start.elapsed();
            let start = Instant::now();
            let restored = lab_replica.restore(pin, temp.path())?;
            let restore = start.elapsed();
            Ok((pin, Measurement { save, transfer, restore, payload, manifest: manifest_bytes }, restored))
        };
        // Alternate order to avoid systematically giving the second method
        // the warm source filesystem cache. Timings are observations, not
        // fragile pass/fail thresholds on a shared CI runner.
        let (v1, experiment) = if round % 2 == 0 {
            (run_v1()?, run_lab()?)
        } else {
            let experiment = run_lab()?;
            (run_v1()?, experiment)
        };
        exact_tree(&world, v1.2.path())?;
        exact_tree(&world, experiment.2.path())?;
        exact_tree(v1.2.path(), experiment.2.path())?;
        v1.1.report(scenario, "v1");
        experiment.1.report(scenario, "block");
        if round == 0 {
            original_v1 = Some(v1.0.clone());
            original_lab = Some(experiment.0);
        } else if !dense {
            assert!(
                experiment.1.total_bytes() * 4 < v1.1.total_bytes(),
                "sparse mutation should substantially reduce real transfer bytes"
            );
            sparse_measurements.push((v1.1, experiment.1));
        }
        previous_v1 = Some(v1.0);
        previous_lab = Some(experiment.0);
    }
    // Cross-check the original known full restore AFTER all newer captures
    // and reopening both stores. No in-memory inventory is recovery state.
    drop(baseline_replica);
    drop(lab_replica);
    let baseline_replica = Storage::open(temp.path().join("v1-replica"))?;
    let lab_replica = LabStore::open(&temp.path().join("lab-replica"))?;
    let restored = TempDir::new_in(temp.path())?;
    baseline_replica.restore_snapshot(&original_v1.unwrap(), restored.path())?;
    let lab_restored = lab_replica.restore(original_lab.unwrap(), temp.path())?;
    assert_eq!(tree_hashes(restored.path())?, original);
    exact_tree(restored.path(), lab_restored.path())?;
    let latest = TempDir::new_in(temp.path())?;
    let latest_manifest = baseline_replica.latest_snapshot(WORLD)?.unwrap();
    assert_eq!(Some(&latest_manifest), previous_v1.as_ref());
    baseline_replica.restore_snapshot(&latest_manifest, latest.path())?;
    let lab_latest = lab_replica.restore(previous_lab.unwrap(), temp.path())?;
    exact_tree(&world, latest.path())?;
    exact_tree(latest.path(), lab_latest.path())?;
    assert!(lab.storage.latest_snapshot(WORLD)?.is_none());
    assert!(lab_replica.storage.latest_snapshot(WORLD)?.is_none());
    assert!(lab.storage.list_worlds()?.is_empty());
    let mut baseline_saves = sparse_measurements.iter().map(|(v1, _)| v1.save).collect::<Vec<_>>();
    let mut lab_saves = sparse_measurements.iter().map(|(_, lab)| lab.save).collect::<Vec<_>>();
    baseline_saves.sort();
    lab_saves.sort();
    let mut v1_costs =
        sparse_measurements.iter().map(|(v1, _)| (v1.save + v1.transfer, v1.total_bytes())).collect::<Vec<_>>();
    let mut block_costs =
        sparse_measurements.iter().map(|(_, lab)| (lab.save + lab.transfer, lab.total_bytes())).collect::<Vec<_>>();
    v1_costs.sort_unstable();
    block_costs.sort_unstable();
    checkpoint_schedule_lab::report(v1_costs[1], block_costs[1]);
    eprintln!(
        "checkpoint_lab sparse_median_v1_save_ms={:.3} sparse_median_block_save_ms={:.3}",
        baseline_saves[1].as_secs_f64() * 1000.,
        lab_saves[1].as_secs_f64() * 1000.
    );
    Ok(())
}

fn blob_path(store: &LabStore, descriptor: &BlobDescriptor) -> PathBuf {
    store.storage.world_dir(WORLD).join("blobs").join(format!("{}.zst", descriptor.hash.to_hex()))
}

#[test]
fn lab_refuses_wrong_pins_malformed_paths_missing_corruption_and_reordered_blocks() -> LabResult<()> {
    let temp = tempfile::tempdir()?;
    let world = temp.path().join("world");
    fs::create_dir(&world)?;
    fs::write(world.join("region.dat"), random_bytes(31, BLOCK * 2 + 11))?;
    fs::write(world.join("empty.dat"), [])?;
    let lab = LabStore::open(&temp.path().join("lab"))?;
    let first = lab.capture(&world, None)?;
    let pin = first.pin()?;
    assert!(lab.restore(Pin { hash: Hash32([5; 32]), ..pin }, temp.path()).is_err());
    assert!(lab.capture(&world, Some(Pin { hash: Hash32([7; 32]), ..pin })).is_err());
    let path = lab.path(1);
    let original_manifest = fs::read(&path)?;
    let mut appended = original_manifest.clone();
    appended.push(0);
    fs::write(&path, &appended)?;
    assert!(lab.restore(pin, temp.path()).is_err());
    // Even supplying a pin for the tampered bytes must reject trailing
    // material because load requires canonical re-encoding.
    let appended_pin = Pin { hash: Hash32::from_domain_bytes(LAB_DOMAIN, &appended), ..pin };
    assert!(lab.restore(appended_pin, temp.path()).is_err());
    fs::write(&path, original_manifest)?;

    let mut changed = first.clone();
    changed.number += 1;
    changed.previous = Some(pin);
    changed.files[0].path = "../escape.dat".into();
    assert!(lab.publish(&changed).is_err());
    assert!(!temp.path().join("escape.dat").exists());
    assert!(!lab.path(2).exists());

    let mut changed = first.clone();
    changed.number += 1;
    changed.previous = Some(pin);
    let region = changed.files.iter_mut().find(|file| file.path == "region.dat").unwrap();
    region.blocks.swap(0, 1);
    assert!(lab.publish(&changed).is_err(), "per-block hashes alone must not authorize rearranged bytes");
    assert!(!lab.path(2).exists());
    let mut changed = first.clone();
    changed.number += 1;
    changed.previous = Some(pin);
    changed.files.iter_mut().find(|file| file.path == "region.dat").unwrap().blocks.pop();
    assert!(lab.publish(&changed).is_err());
    assert!(!lab.path(2).exists());

    let descriptor = first.files.iter().find(|file| file.path == "region.dat").unwrap().blocks[0].clone();
    let path = blob_path(&lab, &descriptor);
    let original = fs::read(&path)?;
    let mut damaged = original.clone();
    damaged[0] ^= 0x55;
    fs::write(&path, damaged)?;
    assert!(lab.restore(pin, temp.path()).is_err());
    assert!(lab.capture(&world, Some(pin)).is_err());
    assert!(!lab.path(2).exists());
    fs::remove_file(&path)?;
    assert!(lab.restore(pin, temp.path()).is_err());
    fs::write(&path, original)?;
    let restored = lab.restore(pin, temp.path())?;
    exact_tree(&world, restored.path())?;
    assert!(lab.publish(&first).is_err(), "immutable sequence must never be overwritten");
    assert_eq!(lab.load(pin)?, first);
    assert!(lab.storage.latest_snapshot(WORLD)?.is_none());
    Ok(())
}
