//! Sabathon #34, experiment B: independent erasure coding of actual V1
//! encoded blobs. Nothing here is a production wire/storage format.
//! Four data shards and two GF(256) parity shards, one per simulated
//! failure domain. Every shard and the original V1 blob are verified.
#[path = "support/research_v1_fixture.rs"]
mod baseline;

use baseline::{capture, copy, random_bytes, require, tree, LabResult, CHUNK, WORLD};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use swarm_protocol::{BlobDescriptor, Hash32, SnapshotManifestV1};
use swarm_storage::{streaming::validate_manifest_shape, Storage};
use tempfile::{NamedTempFile, TempDir};

const DATA: usize = 4;
const PARITY: usize = 2;
const WIDTH: usize = DATA + PARITY;
const MAX_ENCODED: usize = 16 * 1024 * 1024;
const MAX_ARCHIVE: usize = 1024 * 1024;
const SHARD_DOMAIN: &[u8] = b"swarmcraft/research/erasure-shard/v1\0";
const ARCHIVE_DOMAIN: &[u8] = b"swarmcraft/research/erasure-archive/v1\0";
const COEFFICIENTS: [u8; DATA] = [1, 2, 4, 8];

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0;
    // GF(2^8) with primitive polynomial x^8+x^4+x^3+x^2+1 (0x11d).
    while b != 0 {
        if b & 1 != 0 {
            product ^= a;
        }
        let carry = a & 0x80 != 0;
        a <<= 1;
        if carry {
            a ^= 0x1d;
        }
        b >>= 1;
    }
    product
}

fn gf_inverse(value: u8) -> LabResult<u8> {
    require(value != 0, "singular erasure equation")?;
    // a^-1 = a^254 for nonzero field elements.
    let mut exponent = 254u8;
    let mut result = 1;
    let mut base = value;
    while exponent != 0 {
        if exponent & 1 != 0 {
            result = gf_mul(result, base);
        }
        base = gf_mul(base, base);
        exponent >>= 1;
    }
    Ok(result)
}

fn shard_hash(bytes: &[u8]) -> Hash32 {
    Hash32::from_domain_bytes(SHARD_DOMAIN, bytes)
}

fn encode(encoded: &[u8]) -> LabResult<[Vec<u8>; WIDTH]> {
    require(!encoded.is_empty() && encoded.len() <= MAX_ENCODED, "encoded blob outside lab bound")?;
    let size = encoded.len().div_ceil(DATA);
    let mut shards: [Vec<u8>; WIDTH] = std::array::from_fn(|_| vec![0; size]);
    for (index, chunk) in encoded.chunks(size).enumerate() {
        shards[index][..chunk.len()].copy_from_slice(chunk);
    }
    let (data, parity) = shards.split_at_mut(DATA);
    let (p, q) = parity.split_at_mut(1);
    for (offset, (p_byte, q_byte)) in p[0].iter_mut().zip(q[0].iter_mut()).enumerate() {
        for (shard, coefficient) in data.iter().zip(COEFFICIENTS) {
            let byte = shard[offset];
            *p_byte ^= byte;
            *q_byte ^= gf_mul(coefficient, byte);
        }
    }
    Ok(shards)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stripe {
    blob: BlobDescriptor,
    shard_size: u64,
    hashes: [Hash32; WIDTH],
}

impl Stripe {
    fn new(blob: BlobDescriptor, encoded: &[u8]) -> LabResult<(Self, [Vec<u8>; WIDTH])> {
        require(blob.encoded_size == encoded.len() as u64, "encoded blob size mismatch")?;
        let shards = encode(encoded)?;
        let stripe = Self {
            blob,
            shard_size: shards[0].len() as u64,
            hashes: std::array::from_fn(|index| shard_hash(&shards[index])),
        };
        stripe.validate()?;
        Ok((stripe, shards))
    }

    fn validate(&self) -> LabResult<()> {
        require(
            self.blob.encoded_size > 0
                && self.blob.encoded_size <= MAX_ENCODED as u64
                && self.shard_size == self.blob.encoded_size.div_ceil(DATA as u64)
                && self.hashes.iter().all(|hash| *hash != Hash32::default()),
            "invalid stripe bounds",
        )
    }

    fn accepts(&self, index: usize, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.shard_size && shard_hash(bytes) == self.hashes[index]
    }
}

struct Recovery {
    encoded: Vec<u8>,
    repaired: Vec<(usize, Vec<u8>)>,
}

fn recover(stripe: &Stripe, mut shards: [Option<Vec<u8>>; WIDTH]) -> LabResult<Recovery> {
    stripe.validate()?;
    // Corruption is an erasure, not an input to a parity equation.
    for (index, shard) in shards.iter_mut().enumerate() {
        if shard.as_ref().is_some_and(|bytes| !stripe.accepts(index, bytes)) {
            *shard = None;
        }
    }
    let unavailable = (0..WIDTH).filter(|&index| shards[index].is_none()).collect::<Vec<_>>();
    require(unavailable.len() <= PARITY, "too many unavailable shards")?;
    let missing_data = unavailable.iter().copied().filter(|&index| index < DATA).collect::<Vec<_>>();
    let size = usize::try_from(stripe.shard_size)?;

    match missing_data.as_slice() {
        [] => {}
        &[index] => {
            let mut output = vec![0; size];
            if let Some(p) = shards[DATA].as_ref() {
                for offset in 0..size {
                    let mut residual = p[offset];
                    for shard in shards[..DATA].iter().flatten() {
                        residual ^= shard[offset];
                    }
                    output[offset] = residual;
                }
            } else {
                let q = shards[DATA + 1].as_ref().ok_or_else(|| io::Error::other("missing both parity equations"))?;
                let inverse = gf_inverse(COEFFICIENTS[index])?;
                for offset in 0..size {
                    let mut residual = q[offset];
                    for (i, shard) in shards[..DATA].iter().enumerate() {
                        if let Some(bytes) = shard {
                            residual ^= gf_mul(COEFFICIENTS[i], bytes[offset]);
                        }
                    }
                    output[offset] = gf_mul(inverse, residual);
                }
            }
            shards[index] = Some(output);
        }
        &[left, right] => {
            let p = shards[DATA].as_ref().ok_or_else(|| io::Error::other("missing P equation"))?;
            let q = shards[DATA + 1].as_ref().ok_or_else(|| io::Error::other("missing Q equation"))?;
            let a = COEFFICIENTS[left];
            let b = COEFFICIENTS[right];
            let inverse = gf_inverse(a ^ b)?;
            let mut x = vec![0; size];
            let mut y = vec![0; size];
            for offset in 0..size {
                let mut p_residual = p[offset];
                let mut q_residual = q[offset];
                for (i, shard) in shards[..DATA].iter().enumerate() {
                    if let Some(bytes) = shard {
                        p_residual ^= bytes[offset];
                        q_residual ^= gf_mul(COEFFICIENTS[i], bytes[offset]);
                    }
                }
                x[offset] = gf_mul(inverse, q_residual ^ gf_mul(b, p_residual));
                y[offset] = p_residual ^ x[offset];
            }
            shards[left] = Some(x);
            shards[right] = Some(y);
        }
        _ => return Err(io::Error::other("insufficient parity").into()),
    }

    let mut encoded = Vec::with_capacity(size * DATA);
    for shard in &shards[..DATA] {
        encoded.extend_from_slice(shard.as_ref().ok_or_else(|| io::Error::other("unrecovered data shard"))?);
    }
    encoded.truncate(usize::try_from(stripe.blob.encoded_size)?);
    // Recompute ALL parity and canonical zero padding, then verify against
    // the externally pinned stripe hashes, including reconstructed shards.
    let verified = encode(&encoded)?;
    for (index, bytes) in verified.iter().enumerate() {
        require(stripe.accepts(index, bytes), "reconstructed shard failed verification")?;
    }
    let repaired = unavailable.into_iter().map(|index| (index, verified[index].clone())).collect();
    Ok(Recovery { encoded, repaired })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Archive {
    version: u16,
    data: u8,
    parity: u8,
    source: SnapshotManifestV1,
    stripes: Vec<Stripe>,
}

fn unique_descriptors(manifest: &SnapshotManifestV1) -> LabResult<BTreeMap<Hash32, BlobDescriptor>> {
    let mut descriptors = BTreeMap::new();
    for entry in &manifest.entries {
        if let Some(previous) = descriptors.insert(entry.blob.hash, entry.blob.clone()) {
            require(previous == entry.blob, "conflicting blob descriptors")?;
        }
    }
    Ok(descriptors)
}

impl Archive {
    fn validate(&self) -> LabResult<()> {
        require(
            self.version == 1
                && self.data == DATA as u8
                && self.parity == PARITY as u8
                && self.source.world_id == WORLD
                && self.source.snapshot_number == 1,
            "invalid archive scheme or source",
        )?;
        validate_manifest_shape(&self.source)?;
        let descriptors = unique_descriptors(&self.source)?;
        require(self.stripes.len() == descriptors.len(), "archive does not cover exact V1 blob set")?;
        for (stripe, descriptor) in self.stripes.iter().zip(descriptors.values()) {
            require(stripe.blob == *descriptor, "stripe descriptor differs from original manifest")?;
            stripe.validate()?;
        }
        Ok(())
    }

    fn encoded(&self) -> LabResult<Vec<u8>> {
        self.validate()?;
        let bytes = postcard::to_allocvec(self)?;
        require(bytes.len() <= MAX_ARCHIVE, "archive metadata exceeds lab bound")?;
        Ok(bytes)
    }

    fn pin(&self) -> LabResult<Hash32> {
        Ok(Hash32::from_domain_bytes(ARCHIVE_DOMAIN, &self.encoded()?))
    }
}

fn durable_new(path: &Path, bytes: &[u8]) -> LabResult<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("invalid path"))?;
    fs::create_dir_all(parent)?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn node(root: &Path, index: usize) -> PathBuf {
    root.join(format!("failure-domain-{index}"))
}

fn shard_path(root: &Path, index: usize, blob: &BlobDescriptor) -> PathBuf {
    node(root, index).join(format!("{}.shard", blob.hash.to_hex()))
}

fn metadata_path(root: &Path, index: usize) -> PathBuf {
    node(root, index).join("archive.lab")
}

fn read_bounded(path: &Path, maximum: usize) -> LabResult<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => require(metadata.is_file(), "not a regular shard/metadata file")?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn read_original(storage: &Storage, blob: &BlobDescriptor) -> LabResult<Vec<u8>> {
    require(blob.encoded_size <= MAX_ENCODED as u64 && blob.encoded_size > 0, "source blob too large")?;
    storage.verify_blob_streaming(WORLD, blob)?;
    let mut result = Vec::with_capacity(blob.encoded_size as usize);
    loop {
        let (bytes, finished) = storage.read_encoded_blob_chunk(WORLD, blob, result.len() as u64, CHUNK)?;
        result.extend_from_slice(&bytes);
        require(result.len() <= blob.encoded_size as usize, "source encoded length overrun")?;
        if finished {
            break;
        }
        require(!bytes.is_empty(), "source read made no progress")?;
    }
    require(result.len() == blob.encoded_size as usize, "source encoded length differs")?;
    Ok(result)
}

fn archive_world(source: &Storage, manifest: &SnapshotManifestV1, root: &Path) -> LabResult<(Hash32, u64, Archive)> {
    source.verify_snapshot(manifest)?;
    let mut stripes = Vec::new();
    let mut stored = 0;
    for descriptor in unique_descriptors(manifest)?.into_values() {
        let original = read_original(source, &descriptor)?;
        let (stripe, shards) = Stripe::new(descriptor, &original)?;
        for (index, shard) in shards.iter().enumerate() {
            durable_new(&shard_path(root, index, &stripe.blob), shard)?;
            stored += shard.len() as u64;
        }
        stripes.push(stripe);
    }
    let archive = Archive { version: 1, data: DATA as u8, parity: PARITY as u8, source: manifest.clone(), stripes };
    let bytes = archive.encoded()?;
    for index in 0..WIDTH {
        durable_new(&metadata_path(root, index), &bytes)?;
        stored += bytes.len() as u64;
    }
    Ok((archive.pin()?, stored, archive))
}

fn load_archive(root: &Path, expected: Hash32) -> LabResult<(Archive, u64)> {
    let mut read = 0;
    for index in 0..WIDTH {
        let Some(bytes) = read_bounded(&metadata_path(root, index), MAX_ARCHIVE)? else { continue };
        read += bytes.len() as u64;
        if bytes.len() > MAX_ARCHIVE || Hash32::from_domain_bytes(ARCHIVE_DOMAIN, &bytes) != expected {
            continue;
        }
        let archive: Archive = postcard::from_bytes(&bytes)?;
        require(archive.encoded()? == bytes, "noncanonical archive metadata")?;
        return Ok((archive, read));
    }
    Err(io::Error::other("no metadata copy matches exact archive pin").into())
}

#[derive(Debug, Default)]
struct Metrics {
    stripes: usize,
    shard_reads: usize,
    unavailable: usize,
    read_bytes: u64,
    reconstructed_encoded_bytes: u64,
    repair_shard_bytes: u64,
    repair_metadata_bytes: u64,
}

fn rebuild(root: &Path, pin: Hash32, target: &Storage, repair_root: &Path) -> LabResult<(SnapshotManifestV1, Metrics)> {
    let (archive, metadata_read) = load_archive(root, pin)?;
    let mut metrics = Metrics { read_bytes: metadata_read, ..Metrics::default() };
    let mut repaired_domains = BTreeSet::new();
    for stripe in &archive.stripes {
        let mut available: [Option<Vec<u8>>; WIDTH] = std::array::from_fn(|_| None);
        for (index, slot) in available.iter_mut().enumerate() {
            *slot = read_bounded(&shard_path(root, index, &stripe.blob), stripe.shard_size as usize)?;
            if let Some(bytes) = slot.as_ref() {
                metrics.shard_reads += 1;
                metrics.read_bytes += bytes.len() as u64;
            }
        }
        let recovery = recover(stripe, available)?;
        metrics.stripes += 1;
        metrics.unavailable += recovery.repaired.len();
        metrics.reconstructed_encoded_bytes += recovery.encoded.len() as u64;
        // The erasure equations and shard hashes alone do not establish the
        // V1 payload. Use the actual receiver and its decoded-size/hash check.
        let mut offset = 0u64;
        for chunk in recovery.encoded.chunks(CHUNK) {
            let finished = offset + chunk.len() as u64 == stripe.blob.encoded_size;
            offset = target.receive_blob_chunk(WORLD, &stripe.blob, offset, chunk, finished)?;
        }
        require(
            offset == stripe.blob.encoded_size && target.has_complete_blob(WORLD, &stripe.blob),
            "original V1 blob verification failed",
        )?;
        for (index, bytes) in recovery.repaired {
            let path = shard_path(repair_root, index, &stripe.blob);
            durable_new(&path, &bytes)?;
            require(stripe.accepts(index, &fs::read(path)?), "repair readback failed")?;
            repaired_domains.insert(index);
            metrics.repair_shard_bytes += bytes.len() as u64;
        }
    }
    target.finalize_replica(&archive.source)?;
    // Publish exact pinned metadata to repaired domains only after the
    // entire original checkpoint has been verified. A partial repair
    // scratch directory is not a complete archive.
    let metadata = archive.encoded()?;
    for index in repaired_domains {
        let path = metadata_path(repair_root, index);
        durable_new(&path, &metadata)?;
        require(Hash32::from_domain_bytes(ARCHIVE_DOMAIN, &fs::read(path)?) == pin, "repair metadata mismatch")?;
        metrics.repair_metadata_bytes += metadata.len() as u64;
    }
    Ok((archive.source, metrics))
}

#[test]
fn every_pair_of_missing_shards_recovers_exact_bytes_and_triple_loss_fails() -> LabResult<()> {
    for value in 1..=255 {
        assert_eq!(gf_mul(value, gf_inverse(value)?), 1);
    }
    // Test all 1 + 6 + 15 = 22 recoverable erasure patterns at each
    // length, including padding and short final shards.
    for length in [1, 2, 3, 4, 5, 257, 1024] {
        let bytes = random_bytes(5 + length as u64, length);
        let descriptor = BlobDescriptor {
            hash: BlobDescriptor::hash_uncompressed(&bytes),
            uncompressed_size: bytes.len() as u64,
            encoded_size: bytes.len() as u64,
            encoding: swarm_protocol::BlobEncoding::Raw,
        };
        let (stripe, original) = Stripe::new(descriptor, &bytes)?;
        let run = |erased: &[usize]| -> LabResult<()> {
            let available = std::array::from_fn(|index| (!erased.contains(&index)).then(|| original[index].clone()));
            let recovered = recover(&stripe, available)?;
            assert_eq!(recovered.encoded, bytes);
            assert_eq!(recovered.repaired.len(), erased.len());
            for (index, shard) in recovered.repaired {
                assert_eq!(shard, original[index]);
            }
            Ok(())
        };
        run(&[])?;
        for first in 0..WIDTH {
            run(&[first])?;
            for second in first + 1..WIDTH {
                run(&[first, second])?;
                for third in second + 1..WIDTH {
                    let available = std::array::from_fn(|index| {
                        (![first, second, third].contains(&index)).then(|| original[index].clone())
                    });
                    assert!(recover(&stripe, available).is_err());
                }
            }
        }
        let mut corrupt = original.clone().map(Some);
        corrupt[1].as_mut().unwrap()[0] ^= 0x33;
        corrupt[5] = None;
        let repaired = recover(&stripe, corrupt)?;
        assert_eq!(repaired.encoded, bytes);
        assert_eq!(repaired.repaired.len(), 2);
    }
    Ok(())
}

#[test]
fn actual_v1_encoded_archive_measures_storage_reconstruction_repair_and_failure_domains() -> LabResult<()> {
    let temp = tempfile::tempdir()?;
    let world = temp.path().join("world");
    fs::create_dir_all(world.join("region"))?;
    for i in 0..4 {
        fs::write(world.join(format!("region/r.{i}.0.mca")), random_bytes(800 + i * 17, 2 * 1024 * 1024))?;
    }
    fs::write(world.join("level.dat"), b"synthetic metadata")?;
    fs::write(world.join("duplicate.dat"), b"synthetic metadata")?;
    fs::write(world.join("empty.dat"), [])?;
    let source = Storage::open(temp.path().join("v1-source"))?;
    let full_replica = Storage::open(temp.path().join("full-replica"))?;
    let manifest = capture(&source, &world, None)?;
    let start = Instant::now();
    let one_copy = copy(&source, &full_replica, &manifest)?;
    let full_copy_time = start.elapsed();
    let full_restore = TempDir::new_in(temp.path())?;
    let start = Instant::now();
    full_replica.restore_snapshot(&manifest, full_restore.path())?;
    let full_restore_time = start.elapsed();
    assert_eq!(tree(full_restore.path())?, tree(&world)?);

    let archive_root = temp.path().join("six-domains");
    let start = Instant::now();
    let (pin, archive_bytes, archive) = archive_world(&source, &manifest, &archive_root)?;
    let encode_time = start.elapsed();
    let mut disk_bytes = 0;
    for index in 0..WIDTH {
        for file in fs::read_dir(node(&archive_root, index))? {
            disk_bytes += file?.metadata()?.len();
        }
    }
    assert_eq!(archive_bytes, disk_bytes);
    assert!(archive_bytes > one_copy && archive_bytes < one_copy * 2);
    assert!(load_archive(&archive_root, Hash32([7; 32])).is_err());

    // Remove a whole data failure domain; corrupt another whole data
    // domain's same-length shards. Exactly four valid shards remain per
    // stripe. Surviving storage must never be overwritten during recovery.
    fs::remove_dir_all(node(&archive_root, 1))?;
    // Metadata corruption is also checked against the separately pinned
    // archive; it must not prevent using another exact metadata copy.
    let metadata_zero = metadata_path(&archive_root, 0);
    let original_metadata = fs::read(&metadata_zero)?;
    let mut damaged_metadata = original_metadata.clone();
    damaged_metadata[0] ^= 0x33;
    fs::write(&metadata_zero, damaged_metadata)?;
    assert_eq!(load_archive(&archive_root, pin)?.0, archive);
    for stripe in &archive.stripes {
        let path = shard_path(&archive_root, 3, &stripe.blob);
        let mut bytes = fs::read(&path)?;
        bytes[0] ^= 0x5a;
        fs::write(path, bytes)?;
    }
    let reconstructed = Storage::open(temp.path().join("reconstructed"))?;
    let repair_root = temp.path().join("repaired-shards");
    let start = Instant::now();
    let (recovered_manifest, metrics) = rebuild(&archive_root, pin, &reconstructed, &repair_root)?;
    let rebuild_time = start.elapsed();
    assert_eq!(recovered_manifest, manifest);
    assert_eq!(metrics.unavailable, metrics.stripes * 2);
    assert_eq!(metrics.shard_reads, metrics.stripes * 5);
    assert_eq!(reconstructed.latest_snapshot(WORLD)?.unwrap(), manifest);
    let restored = TempDir::new_in(temp.path())?;
    let start = Instant::now();
    reconstructed.restore_snapshot(&recovered_manifest, restored.path())?;
    let restore_time = start.elapsed();
    assert_eq!(tree(restored.path())?, tree(full_restore.path())?);
    assert_eq!(tree(restored.path())?, tree(&world)?);
    for stripe in &archive.stripes {
        for index in [1, 3] {
            assert!(stripe.accepts(index, &fs::read(shard_path(&repair_root, index, &stripe.blob))?));
        }
    }
    assert_eq!(load_archive(&repair_root, pin)?.0, archive);
    assert_eq!(metrics.repair_metadata_bytes, 2 * original_metadata.len() as u64);
    for index in [1, 3] {
        assert_eq!(fs::read(metadata_path(&repair_root, index))?, original_metadata);
    }
    // Recovery must not have changed any surviving source metadata.
    assert_ne!(fs::read(&metadata_zero)?, original_metadata);
    eprintln!("erasure_lab one_full_copy_bytes={one_copy} two_full_copies_bytes={} three_full_copies_bytes={} coded_six_domains_bytes={archive_bytes} stripes={} full_copy_ms={:.3} full_restore_ms={:.3} coded_encode_ms={:.3} coded_rebuild_and_repair_ms={:.3} coded_restore_ms={:.3} shard_reads={} actual_read_bytes={} reconstructed_encoded_bytes={} repaired_shards={} repair_shard_bytes={} repair_metadata_bytes={}",
        one_copy*2, one_copy*3, metrics.stripes,
        full_copy_time.as_secs_f64()*1000., full_restore_time.as_secs_f64()*1000.,
        encode_time.as_secs_f64()*1000., rebuild_time.as_secs_f64()*1000., restore_time.as_secs_f64()*1000.,
        metrics.shard_reads, metrics.read_bytes, metrics.reconstructed_encoded_bytes, metrics.unavailable, metrics.repair_shard_bytes, metrics.repair_metadata_bytes);

    // A third unavailable domain is beyond this code's fault budget.
    fs::remove_dir_all(node(&archive_root, 4))?;
    let failed = Storage::open(temp.path().join("failed-reconstruction"))?;
    assert!(rebuild(&archive_root, pin, &failed, &temp.path().join("failed-repair")).is_err());
    assert!(failed.latest_snapshot(WORLD)?.is_none());
    assert!(failed.list_worlds()?.is_empty());

    // Independently ensure the archive pin cannot replace the V1 blob
    // integrity boundary: make a self-consistent coded stripe of corrupted
    // encoded bytes under the original descriptor. It decodes algebraically
    // but the original V1 content hash must reject it.
    let descriptor = archive.stripes.iter().max_by_key(|stripe| stripe.blob.encoded_size).unwrap().blob.clone();
    let mut bad_bytes = read_original(&source, &descriptor)?;
    bad_bytes[0] ^= 0x55;
    let (bad_stripe, bad_shards) = Stripe::new(descriptor.clone(), &bad_bytes)?;
    let algebraic = recover(&bad_stripe, bad_shards.map(Some))?;
    assert_eq!(algebraic.encoded, bad_bytes);
    let rejected = Storage::open(temp.path().join("bad-original"))?;
    let mut offset = 0u64;
    let mut denied = false;
    for chunk in algebraic.encoded.chunks(CHUNK) {
        let finished = offset + chunk.len() as u64 == descriptor.encoded_size;
        match rejected.receive_blob_chunk(WORLD, &descriptor, offset, chunk, finished) {
            Ok(next) => offset = next,
            Err(_) => {
                denied = true;
                break;
            }
        }
    }
    assert!(denied);
    assert!(!rejected.has_complete_blob(WORLD, &descriptor));
    assert!(rejected.latest_snapshot(WORLD)?.is_none());
    Ok(())
}
