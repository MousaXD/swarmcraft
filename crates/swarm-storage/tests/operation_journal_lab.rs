//! Sabathon #34, experiment A: explicit byte-operation journal.
//! This does NOT capture Minecraft semantic operations or execute live writes.
//! The test replays only into bounded, newly owned scratch directories.
#[path = "support/research_v1_fixture.rs"]
mod baseline;

use baseline::{capture, copy, random_bytes, require, tree, LabResult, WORLD};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use swarm_protocol::{
    snapshot_state_root, BlobDescriptor, BlobEncoding, Hash32, PeerId, SnapshotEntry, SnapshotManifestV1,
    PROTOCOL_VERSION,
};
use swarm_storage::{streaming::validate_manifest_shape, Storage};
use tempfile::{NamedTempFile, TempDir};

const DOMAIN: &[u8] = b"swarmcraft/research/byte-operation-batch/v1\0";
const MAX_BATCH: usize = 512 * 1024;
const MAX_OPERATIONS: usize = 128;
const MAX_BATCHES: usize = 64;
const MAX_TREE: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 1024;
type Tree = BTreeMap<String, Vec<u8>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Operation {
    ReplaceRange { path: String, offset: u64, before: Hash32, bytes: Vec<u8> },
    ReplaceFile { path: String, before: Hash32, bytes: Vec<u8> },
    Create { path: String, bytes: Vec<u8> },
    Remove { path: String, before: Hash32 },
    Rename { from: String, to: String, before: Hash32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Batch {
    version: u16,
    number: u64,
    base_manifest: Hash32,
    previous: Option<Hash32>,
    before_root: Hash32,
    after_root: Hash32,
    operations: Vec<Operation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Pin {
    number: u64,
    hash: Hash32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    pin: Pin,
    encoded: Vec<u8>,
}

fn digest(bytes: &[u8]) -> Hash32 {
    BlobDescriptor::hash_uncompressed(bytes)
}

fn shape(entries: Vec<SnapshotEntry>) -> LabResult<Hash32> {
    let root = snapshot_state_root(&entries)?;
    let manifest = SnapshotManifestV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: WORLD,
        snapshot_number: 1,
        epoch: 1,
        sequence: 1,
        previous_snapshot_hash: None,
        entries,
        state_root: root,
        authority_peer_id: PeerId([1; 32]),
        authority_public_key: [2; 32],
        signature: vec![],
    };
    validate_manifest_shape(&manifest)?;
    let names = manifest.entries.iter().map(|entry| entry.path.to_ascii_lowercase()).collect::<BTreeSet<_>>();
    for name in &names {
        let mut prefix = name.as_str();
        while let Some((parent, _)) = prefix.rsplit_once('/') {
            require(!names.contains(parent), "file/directory collision")?;
            prefix = parent;
        }
    }
    Ok(root)
}

fn entry(path: &str, bytes: &[u8]) -> SnapshotEntry {
    SnapshotEntry {
        path: path.into(),
        blob: BlobDescriptor {
            hash: digest(bytes),
            uncompressed_size: bytes.len() as u64,
            encoded_size: bytes.len() as u64,
            encoding: BlobEncoding::Raw,
        },
    }
}

fn tree_root(files: &Tree) -> LabResult<Hash32> {
    require(files.len() <= MAX_FILES, "too many files")?;
    let total = files
        .values()
        .try_fold(0usize, |sum, bytes| sum.checked_add(bytes.len()))
        .ok_or_else(|| io::Error::other("tree length overflow"))?;
    require(total <= MAX_TREE, "journal replay exceeds tree size")?;
    shape(files.iter().map(|(path, bytes)| entry(path, bytes)).collect())
}

fn path_allowed(path: &str) -> LabResult<()> {
    shape(vec![entry(path, &[])])?;
    Ok(())
}

fn preimage<'a>(files: &'a Tree, path: &str, before: Hash32) -> LabResult<&'a [u8]> {
    path_allowed(path)?;
    let bytes = files.get(path).ok_or_else(|| io::Error::other("missing operation preimage"))?;
    require(digest(bytes) == before, "wrong operation preimage")?;
    Ok(bytes)
}

fn apply(files: &mut Tree, operation: &Operation) -> LabResult<()> {
    match operation {
        Operation::ReplaceRange { path, offset, before, bytes } => {
            path_allowed(path)?;
            require(!bytes.is_empty() && bytes.len() <= MAX_BATCH, "invalid range length")?;
            let target = files.get_mut(path).ok_or_else(|| io::Error::other("missing range target"))?;
            let start = usize::try_from(*offset)?;
            let end = start.checked_add(bytes.len()).ok_or_else(|| io::Error::other("range overflow"))?;
            let range = target.get_mut(start..end).ok_or_else(|| io::Error::other("range outside file"))?;
            require(digest(range) == *before, "wrong range preimage")?;
            range.copy_from_slice(bytes);
        }
        Operation::ReplaceFile { path, before, bytes } => {
            preimage(files, path, *before)?;
            require(bytes.len() <= MAX_BATCH, "replacement exceeds operation bound")?;
            files.insert(path.clone(), bytes.clone());
        }
        Operation::Create { path, bytes } => {
            path_allowed(path)?;
            require(
                !files.contains_key(path) && bytes.len() <= MAX_BATCH && files.len() < MAX_FILES,
                "invalid create",
            )?;
            files.insert(path.clone(), bytes.clone());
        }
        Operation::Remove { path, before } => {
            preimage(files, path, *before)?;
            files.remove(path);
        }
        Operation::Rename { from, to, before } => {
            preimage(files, from, *before)?;
            path_allowed(to)?;
            require(!files.contains_key(to), "rename would overwrite")?;
            let bytes = files.remove(from).expect("checked preimage");
            files.insert(to.clone(), bytes);
        }
    }
    Ok(())
}

fn batch_bytes(batch: &Batch) -> LabResult<Vec<u8>> {
    require(
        batch.version == 1
            && batch.number > 0
            && !batch.operations.is_empty()
            && batch.operations.len() <= MAX_OPERATIONS,
        "invalid journal batch shape",
    )?;
    let bytes = postcard::to_allocvec(batch)?;
    require(bytes.len() <= MAX_BATCH, "batch too large")?;
    Ok(bytes)
}

fn encode(batch: &Batch) -> LabResult<(Pin, Vec<u8>)> {
    let raw = batch_bytes(batch)?;
    let pin = Pin { number: batch.number, hash: Hash32::from_domain_bytes(DOMAIN, &raw) };
    let encoded = zstd::stream::encode_all(raw.as_slice(), 3)?;
    let record = postcard::to_allocvec(&Record { pin, encoded })?;
    require(record.len() <= MAX_BATCH * 2, "encoded journal record too large")?;
    Ok((pin, record))
}

fn decode(record: &[u8], expected: Pin) -> LabResult<Batch> {
    require(record.len() <= MAX_BATCH * 2, "encoded journal record too large")?;
    let decoded: Record = postcard::from_bytes(record)?;
    require(
        decoded.pin == expected && postcard::to_allocvec(&decoded)? == record,
        "record pin/canonical encoding mismatch",
    )?;
    let mut raw = Vec::new();
    zstd::stream::read::Decoder::new(decoded.encoded.as_slice())?.take(MAX_BATCH as u64 + 1).read_to_end(&mut raw)?;
    require(
        raw.len() <= MAX_BATCH && Hash32::from_domain_bytes(DOMAIN, &raw) == expected.hash,
        "batch hash/size mismatch",
    )?;
    let batch: Batch = postcard::from_bytes(&raw)?;
    require(
        batch.number == expected.number && batch_bytes(&batch)? == raw,
        "batch number/canonical encoding mismatch",
    )?;
    Ok(batch)
}

struct Journal {
    root: PathBuf,
}

impl Journal {
    fn open(root: &Path) -> LabResult<Self> {
        fs::create_dir_all(root)?;
        Ok(Self { root: root.to_path_buf() })
    }

    fn path(&self, number: u64) -> PathBuf {
        self.root.join(format!("{number:020}.journal-lab"))
    }

    fn bytes(&self, pin: Pin) -> LabResult<Vec<u8>> {
        let path = self.path(pin.number);
        require(fs::symlink_metadata(&path)?.is_file(), "invalid record file")?;
        let mut bytes = Vec::new();
        File::open(path)?.take((MAX_BATCH * 2 + 1) as u64).read_to_end(&mut bytes)?;
        decode(&bytes, pin)?;
        Ok(bytes)
    }

    fn install(&self, pin: Pin, record: &[u8]) -> LabResult<()> {
        decode(record, pin)?;
        let mut temp = NamedTempFile::new_in(&self.root)?;
        temp.write_all(record)?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(self.path(pin.number))?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    fn append(&self, batch: &Batch) -> LabResult<(Pin, u64)> {
        let (pin, record) = encode(batch)?;
        self.install(pin, &record)?;
        Ok((pin, record.len() as u64))
    }

    fn restore(
        &self,
        base_store: &Storage,
        base_pin: Hash32,
        pins: &[Pin],
        final_pin: Option<Pin>,
        parent: &Path,
    ) -> LabResult<(TempDir, usize, u64)> {
        require(pins.len() <= MAX_BATCHES, "journal suffix exceeds lab replay bound")?;
        require(pins.last().copied() == final_pin, "requested final journal point missing")?;
        let base = base_store.load_snapshot(WORLD, 1)?;
        require(base.manifest_hash()? == base_pin, "wrong full checkpoint base")?;
        base_store.verify_snapshot(&base)?;
        let base_tree = TempDir::new_in(parent)?;
        base_store.restore_snapshot(&base, base_tree.path())?;
        let mut files = tree(base_tree.path())?;
        let mut root = tree_root(&files)?;
        let mut previous = None;
        let mut operations = 0;
        let mut record_bytes = 0;
        for (index, pin) in pins.iter().enumerate() {
            require(pin.number == index as u64 + 1, "non-contiguous journal sequence")?;
            let record = self.bytes(*pin)?;
            record_bytes += record.len() as u64;
            let batch = decode(&record, *pin)?;
            require(
                batch.base_manifest == base_pin && batch.previous == previous && batch.before_root == root,
                "journal predecessor/base/state mismatch",
            )?;
            for op in &batch.operations {
                apply(&mut files, op)?;
            }
            root = tree_root(&files)?;
            require(root == batch.after_root, "journal replay result mismatch")?;
            operations += batch.operations.len();
            previous = Some(pin.hash);
        }
        // No output files are touched until the entire requested chain and
        // every operation's result are verified. Never modify the base tree.
        let restored = TempDir::new_in(parent)?;
        for (path, bytes) in &files {
            let output = restored.path().join(path);
            fs::create_dir_all(output.parent().ok_or_else(|| io::Error::other("invalid parent"))?)?;
            let mut file = OpenOptions::new().create_new(true).write(true).open(output)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        require(tree(restored.path())? == files, "written scratch tree differs")?;
        Ok((restored, operations, record_bytes))
    }
}

fn create_world(root: &Path) -> LabResult<()> {
    fs::create_dir_all(root.join("region"))?;
    fs::create_dir_all(root.join("data"))?;
    for i in 0..4 {
        fs::write(root.join(format!("region/r.{i}.0.mca")), random_bytes(600 + i * 31, 2 * 1024 * 1024))?;
    }
    fs::write(root.join("level.dat"), b"metadata")?;
    fs::write(root.join("data/remove.dat"), b"remove only in journal")?;
    fs::write(root.join("data/rename.dat"), b"rename me")?;
    fs::write(root.join("data/empty.dat"), [])?;
    Ok(())
}

fn mutate_world(root: &Path, round: u64) -> LabResult<Vec<Operation>> {
    // The source is independently modified through actual file operations.
    // The journal is the description of those known changes, not the code
    // used to generate the full V1 comparison state.
    let mut operations = Vec::new();
    for i in 0..4 {
        let path = format!("region/r.{i}.0.mca");
        let mut file = OpenOptions::new().read(true).write(true).open(root.join(&path))?;
        let offset = round * 65536 + i * 4096;
        let mut old = vec![0; 4096];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut old)?;
        let bytes = random_bytes(13000 + round * 97 + i * 7, old.len());
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        operations.push(Operation::ReplaceRange { path, offset, before: digest(&old), bytes });
    }
    let path = "level.dat";
    let before = digest(&fs::read(root.join(path))?);
    let bytes = format!("metadata generation {round}").into_bytes();
    fs::write(root.join(path), &bytes)?;
    operations.push(Operation::ReplaceFile { path: path.into(), before, bytes });
    if round == 1 {
        let path = "data/remove.dat";
        let before = digest(&fs::read(root.join(path))?);
        fs::remove_file(root.join(path))?;
        operations.push(Operation::Remove { path: path.into(), before });
        let from = "data/rename.dat";
        let to = "data/renamed.dat";
        let before = digest(&fs::read(root.join(from))?);
        fs::rename(root.join(from), root.join(to))?;
        operations.push(Operation::Rename { from: from.into(), to: to.into(), before });
        let path = "data/new.dat";
        let bytes = b"new small file".to_vec();
        fs::write(root.join(path), &bytes)?;
        operations.push(Operation::Create { path: path.into(), bytes });
    }
    Ok(operations)
}

#[test]
fn byte_journal_measures_actual_encoded_cost_and_verifies_full_restore_and_failures() -> LabResult<()> {
    let temp = tempfile::tempdir()?;
    let world = temp.path().join("world");
    create_world(&world)?;
    let original = tree(&world)?;
    let source = Storage::open(temp.path().join("v1-source"))?;
    let receiver = Storage::open(temp.path().join("v1-receiver"))?;
    let base_store = Storage::open(temp.path().join("journal-base"))?;
    let journal = Journal::open(&temp.path().join("journal-source"))?;
    let replica = Journal::open(&temp.path().join("journal-receiver"))?;
    let base = capture(&source, &world, None)?;
    let base_pin = base.manifest_hash()?;
    let baseline_base = copy(&source, &receiver, &base)?;
    assert_eq!(baseline_base, copy(&source, &base_store, &base)?);
    let mut baseline_total = baseline_base;
    let mut journal_total = baseline_base;
    let mut last = base;
    let mut pins = Vec::new();
    let mut before_root = tree_root(&original)?;
    let mut previous = None;
    for number in 1..=3 {
        let operations = mutate_world(&world, number)?;
        let start = Instant::now();
        let after_root = tree_root(&tree(&world)?)?;
        let batch =
            Batch { version: 1, number, base_manifest: base_pin, previous, before_root, after_root, operations };
        let journal_prepare = start.elapsed();

        let start = Instant::now();
        let latest = capture(&source, &world, Some(&last))?;
        let v1_save = start.elapsed();
        let start = Instant::now();
        let v1_bytes = copy(&source, &receiver, &latest)?;
        let v1_copy = start.elapsed();

        let start = Instant::now();
        let (pin, journal_bytes) = journal.append(&batch)?;
        let journal_save = start.elapsed();
        let start = Instant::now();
        let record = journal.bytes(pin)?;
        assert_eq!(journal_bytes, record.len() as u64);
        replica.install(pin, &record)?;
        let journal_copy = start.elapsed();

        pins.push(pin);
        let baseline_restore = TempDir::new_in(temp.path())?;
        let start = Instant::now();
        receiver.restore_snapshot(&latest, baseline_restore.path())?;
        let v1_restore = start.elapsed();
        let start = Instant::now();
        let (restored, count, read) = replica.restore(&base_store, base_pin, &pins, Some(pin), temp.path())?;
        let journal_restore = start.elapsed();
        let actual = tree(&world)?;
        assert_eq!(tree(baseline_restore.path())?, actual);
        assert_eq!(tree(restored.path())?, actual);
        assert!(journal_bytes * 20 < v1_bytes, "sparse journal encoded size must be substantially lower");
        baseline_total += v1_bytes;
        journal_total += journal_bytes;
        eprintln!("journal_lab round={number} v1_delta_bytes={v1_bytes} journal_delta_bytes={journal_bytes} v1_capture_ms={:.3} journal_prepare_ms={:.3} journal_append_ms={:.3} v1_copy_ms={:.3} journal_copy_ms={:.3} v1_restore_ms={:.3} journal_restore_ms={:.3} replay_operations={count} journal_read_bytes={read}",
            v1_save.as_secs_f64()*1000., journal_prepare.as_secs_f64()*1000., journal_save.as_secs_f64()*1000., v1_copy.as_secs_f64()*1000.,
            journal_copy.as_secs_f64()*1000., v1_restore.as_secs_f64()*1000., journal_restore.as_secs_f64()*1000.);
        last = latest;
        previous = Some(pin.hash);
        before_root = after_root;
    }
    eprintln!("journal_lab base_bytes={baseline_base} retained_v1_payload_and_manifests={baseline_total} retained_journal_base_and_records={journal_total}");
    assert!(journal_total < baseline_total);
    let final_pin = pins.last().copied().unwrap();
    let original_restored = replica.restore(&base_store, base_pin, &[], None, temp.path())?;
    assert_eq!(tree(original_restored.0.path())?, original);
    assert!(replica.restore(&base_store, Hash32([9; 32]), &pins, Some(final_pin), temp.path()).is_err());
    assert!(replica.restore(&base_store, base_pin, &pins[..2], Some(final_pin), temp.path()).is_err());
    assert!(replica
        .restore(&base_store, base_pin, &[pins[1], pins[0], pins[2]], Some(final_pin), temp.path())
        .is_err());

    // Missing middle record blocks the suffix, not the verified base/prefix.
    let middle = replica.path(2);
    let record = fs::read(&middle)?;
    fs::remove_file(&middle)?;
    assert!(replica.restore(&base_store, base_pin, &pins, Some(final_pin), temp.path()).is_err());
    assert!(replica.restore(&base_store, base_pin, &pins[..1], Some(pins[0]), temp.path()).is_ok());
    let mut truncated = record.clone();
    truncated.pop();
    assert!(replica.install(pins[1], &truncated).is_err());
    assert!(!middle.exists());
    let mut corrupted = record.clone();
    let end = corrupted.len() - 1;
    corrupted[end] ^= 0x5a;
    assert!(replica.install(pins[1], &corrupted).is_err());
    replica.install(pins[1], &record)?;
    assert!(replica.install(pins[1], &record).is_err());

    // A freshly hashed record is not sufficient: wrong operation preimage,
    // mismatched predecessor and an unsafe path must fail replay.
    let invalid = Batch {
        version: 1,
        number: 4,
        base_manifest: base_pin,
        previous: Some(final_pin.hash),
        before_root,
        after_root: before_root,
        operations: vec![Operation::ReplaceRange {
            path: "region/r.0.0.mca".into(),
            offset: 0,
            before: Hash32([7; 32]),
            bytes: vec![0; 4096],
        }],
    };
    let (bad_pin, bad_record) = encode(&invalid)?;
    replica.install(bad_pin, &bad_record)?;
    let mut bad_chain = pins.clone();
    bad_chain.push(bad_pin);
    assert!(replica.restore(&base_store, base_pin, &bad_chain, Some(bad_pin), temp.path()).is_err());
    fs::remove_file(replica.path(4))?;
    let invalid =
        Batch { operations: vec![Operation::Create { path: "../escape.dat".into(), bytes: vec![1] }], ..invalid };
    let (bad_pin, bad_record) = encode(&invalid)?;
    replica.install(bad_pin, &bad_record)?;
    *bad_chain.last_mut().unwrap() = bad_pin;
    assert!(replica.restore(&base_store, base_pin, &bad_chain, Some(bad_pin), temp.path()).is_err());
    assert!(!temp.path().join("escape.dat").exists());
    fs::remove_file(replica.path(4))?;

    drop(replica);
    let reopened = Journal::open(&temp.path().join("journal-receiver"))?;
    let restored = reopened.restore(&base_store, base_pin, &pins, Some(final_pin), temp.path())?;
    assert_eq!(tree(restored.0.path())?, tree(&world)?);
    assert_eq!(base_store.latest_snapshot(WORLD)?.unwrap().manifest_hash()?, base_pin);
    assert_eq!(receiver.latest_snapshot(WORLD)?.unwrap(), last);
    Ok(())
}
