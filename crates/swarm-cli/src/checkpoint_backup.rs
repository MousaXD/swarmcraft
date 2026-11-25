//! Encrypted data-only backup transport. No imported authority or original
//! world identity can be published into the recipient's canonical storage.
use anyhow::{bail, Context, Result};
use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, ErrorKind, Read, Write},
    path::{Component, Path, PathBuf},
};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_protocol::{
    BlobDescriptor, BlobEncoding, Hash32, HistoricalReplayProofV1, RuntimeCompatibilityManifestV1, SnapshotManifestV1,
    WorldId, MAX_REPLAY_PROOF_BYTES, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{Storage, WorldMetadataV1};
use tempfile::{Builder, NamedTempFile};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"SCBKUP01";
const HEADER_SIZE: usize = 24;
const FRAME_BYTES: usize = 256 * 1024;
const TAG_BYTES: usize = 16;
const FRAME_OVERHEAD: u64 = 4 + TAG_BYTES as u64;
const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: u64 = 1 << 40;
const INCOMPLETE: &str = ".swarmcraft-backup-import-incomplete";
type BackupKey = Zeroizing<[u8; 32]>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct Selection {
    world: WorldId,
    checkpoint: Hash32,
    acknowledged_conflicts: Option<Hash32>,
}

pub struct BackupExportRequest<'a> {
    pub world: WorldId,
    pub checkpoint: Hash32,
    pub acknowledged_conflicts: Option<Hash32>,
    pub key_file: &'a Path,
    pub destination: &'a Path,
}

pub struct BackupImportRequest<'a> {
    pub expected_world: WorldId,
    pub expected_checkpoint: Hash32,
    pub key_file: &'a Path,
    pub bundle: &'a Path,
    pub destination: &'a Path,
    pub confirm_data_only: bool,
}

#[derive(Debug, Serialize)]
pub struct BackupReceipt {
    pub format_version: u16,
    pub operation: &'static str,
    pub source_world_id: String,
    pub source_snapshot_number: u64,
    pub source_checkpoint_hash: String,
    pub source_conflict_digest: Option<String>,
    pub state_root: String,
    pub compatibility_fingerprint: String,
    pub compatibility: RuntimeCompatibilityManifestV1,
    pub files: usize,
    pub encoded_payload_bytes: u64,
    pub restored_payload_bytes: u64,
    pub historical_manifest_count: usize,
    pub destination: PathBuf,
    pub registered_world: bool,
    pub authority_granted: bool,
}

/// Generated random 256-bit binary key, not a password. Never print the key
/// or accept it as a CLI argument.
pub fn generate_key(paths: &DataPaths, destination: &Path) -> Result<PathBuf> {
    let destination = external_destination(&paths.root, destination)?;
    let mut key = Zeroizing::new([0u8; 32]);
    OsRng.try_fill_bytes(&mut *key).map_err(|_| anyhow::anyhow!("OS random key generation failed"))?;
    let mut file = private_tempfile(parent(&destination)?, ".swarmcraft-key-")?;
    file.write_all(&*key)?;
    publish_file(file, &destination)?;
    Ok(destination)
}

fn load_key(path: &Path) -> Result<BackupKey> {
    let mut file = open_regular_input(path)?;
    let metadata = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("backup key file must be owner-only (chmod 600)");
        }
    }
    if metadata.len() != 32 {
        bail!("backup key must be a generated 32-byte binary key file");
    }
    let mut key = Zeroizing::new([0u8; 32]);
    file.read_exact(&mut *key)?;
    if file.read(&mut [0u8; 1])? != 0 {
        bail!("backup key file changed while reading");
    }
    Ok(key)
}

fn open_regular_input(path: &Path) -> Result<File> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        bail!("backup input must be a regular non-symlink file");
    }
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        bail!("backup input is not a regular file");
    }
    Ok(file)
}

fn parent(path: &Path) -> Result<&Path> {
    path.parent().filter(|p| !p.as_os_str().is_empty()).context("backup destination has no parent")
}

/// Resolve the original root even if it has never been initialized. Import
/// and keygen must not open or create canonical storage.
fn resolved_root(root: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(root)?;
    if absolute.components().any(|c| c == Component::ParentDir) {
        bail!("backup data root must not contain parent-directory traversal");
    }
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing.push(existing.file_name().context("invalid data root")?);
                existing = parent(existing)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut resolved = existing.canonicalize()?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn external_destination(root: &Path, destination: &Path) -> Result<PathBuf> {
    if destination.components().any(|c| c == Component::ParentDir) {
        bail!("backup destination must not contain parent-directory traversal");
    }
    let leaf = destination.file_name().context("backup destination needs a new name")?;
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().context("backup destination parent must already exist")?;
    if !parent.is_dir() {
        bail!("backup destination parent is not a directory");
    }
    let destination = parent.join(leaf);
    let root = resolved_root(root)?;
    if destination.starts_with(&root) || root.starts_with(&destination) {
        bail!("backup destination must be outside SwarmCraft's data root");
    }
    match fs::symlink_metadata(&destination) {
        Ok(_) => bail!("backup destination already exists; nothing is overwritten"),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(destination),
        Err(error) => Err(error.into()),
    }
}

fn private_tempfile(parent: &Path, prefix: &str) -> Result<NamedTempFile> {
    let mut builder = Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    Ok(builder.tempfile_in(parent)?)
}

fn private_tempdir(parent: &Path) -> Result<tempfile::TempDir> {
    let mut builder = Builder::new();
    builder.prefix(".swarmcraft-backup-import-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir_in(parent)?)
}

fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn publish_file(file: NamedTempFile, destination: &Path) -> Result<()> {
    file.as_file().sync_all()?;
    file.persist_noclobber(destination).context("cannot publish completed backup without overwriting")?;
    sync_dir(parent(destination)?)
}

fn require_space(path: &Path, bytes: u64) -> Result<()> {
    let available = fs2::available_space(path)?;
    if bytes > available.saturating_sub(swarm_storage::space::DEFAULT_FREE_RESERVE_BYTES) {
        bail!("insufficient local space for isolated backup staging and restore; free disk space before retrying");
    }
    Ok(())
}

fn nonce_and_aad(header: &[u8; HEADER_SIZE], counter: u64, len: u32) -> ([u8; 24], Vec<u8>) {
    let mut nonce = [0u8; 24];
    nonce[..16].copy_from_slice(&header[8..]);
    nonce[16..].copy_from_slice(&counter.to_be_bytes());
    let mut aad = b"swarmcraft/checkpoint-backup/frame/v1\0".to_vec();
    aad.extend_from_slice(header);
    aad.extend_from_slice(&counter.to_be_bytes());
    aad.extend_from_slice(&len.to_be_bytes());
    (nonce, aad)
}

struct FrameWriter<W> {
    output: W,
    cipher: XChaCha20Poly1305,
    header: [u8; HEADER_SIZE],
    counter: u64,
}

impl<W: Write> FrameWriter<W> {
    fn new(mut output: W, key: &BackupKey) -> Result<Self> {
        let mut header = [0u8; HEADER_SIZE];
        header[..8].copy_from_slice(MAGIC);
        OsRng.try_fill_bytes(&mut header[8..]).map_err(|_| anyhow::anyhow!("OS random nonce generation failed"))?;
        output.write_all(&header)?;
        Ok(Self { output, cipher: XChaCha20Poly1305::new((&**key).into()), header, counter: 0 })
    }

    fn frame(&mut self, plaintext: &[u8]) -> Result<()> {
        if plaintext.len() > FRAME_BYTES {
            bail!("backup frame exceeds its fixed limit");
        }
        let len = plaintext.len() as u32;
        let (nonce, aad) = nonce_and_aad(&self.header, self.counter, len);
        let encrypted = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad })
            .map_err(|_| anyhow::anyhow!("backup frame encryption failed"))?;
        self.output.write_all(&len.to_be_bytes())?;
        self.output.write_all(&encrypted)?;
        self.counter = self.counter.checked_add(1).context("backup frame counter exhausted")?;
        Ok(())
    }

    fn metadata<T: Serialize>(&mut self, record: &T, limit: usize, remaining: &mut u64) -> Result<()> {
        let data = Zeroizing::new(postcard::to_allocvec(record)?);
        charge_metadata(data.len(), limit, remaining)?;
        self.frame(&(data.len() as u32).to_be_bytes())?;
        for chunk in data.chunks(FRAME_BYTES) {
            self.frame(chunk)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.frame(&[])?;
        self.output.flush()?;
        Ok(())
    }
}

struct FrameReader<R> {
    input: R,
    cipher: XChaCha20Poly1305,
    header: [u8; HEADER_SIZE],
    counter: u64,
}

impl<R: Read> FrameReader<R> {
    fn new(mut input: R, key: &BackupKey) -> Result<Self> {
        let mut header = [0u8; HEADER_SIZE];
        input.read_exact(&mut header).context("incomplete backup header")?;
        if &header[..8] != MAGIC {
            bail!("unsupported checkpoint backup format");
        }
        Ok(Self { input, cipher: XChaCha20Poly1305::new((&**key).into()), header, counter: 0 })
    }

    fn frame(&mut self, expected: usize) -> Result<Zeroizing<Vec<u8>>> {
        let mut length = [0u8; 4];
        self.input.read_exact(&mut length).context("incomplete backup frame")?;
        let len = u32::from_be_bytes(length);
        if len as usize != expected || expected > FRAME_BYTES {
            bail!("unexpected backup frame length");
        }
        let mut encrypted = vec![0u8; expected + TAG_BYTES];
        self.input.read_exact(&mut encrypted).context("incomplete authenticated backup frame")?;
        let (nonce, aad) = nonce_and_aad(&self.header, self.counter, len);
        let decrypted = self
            .cipher
            .decrypt(XNonce::from_slice(&nonce), Payload { msg: &encrypted, aad: &aad })
            .map_err(|_| anyhow::anyhow!("backup authentication failed (wrong key, damaged or modified bundle)"))?;
        self.counter = self.counter.checked_add(1).context("backup frame counter exhausted")?;
        Ok(Zeroizing::new(decrypted))
    }

    fn metadata<T: DeserializeOwned + Serialize>(&mut self, limit: usize, remaining: &mut u64) -> Result<T> {
        let size = self.frame(4)?;
        let length = u32::from_be_bytes(size.as_slice().try_into()?) as usize;
        charge_metadata(length, limit, remaining)?;
        let mut data = Zeroizing::new(Vec::with_capacity(length));
        while data.len() < length {
            let chunk = self.frame((length - data.len()).min(FRAME_BYTES))?;
            data.extend_from_slice(&chunk);
        }
        let (value, tail) = postcard::take_from_bytes::<T>(&data)?;
        if !tail.is_empty() || postcard::to_allocvec(&value)? != *data {
            bail!("backup record has a noncanonical encoding");
        }
        Ok(value)
    }

    fn finish(mut self) -> Result<()> {
        self.frame(0)?;
        if self.input.read(&mut [0u8; 1])? != 0 {
            bail!("backup contains trailing data after its authenticated end");
        }
        Ok(())
    }
}

fn charge_metadata(size: usize, limit: usize, remaining: &mut u64) -> Result<()> {
    if size == 0 || size > limit {
        bail!("backup metadata record exceeds supported bounds");
    }
    *remaining = remaining.checked_sub(size as u64).context("backup metadata exceeds its aggregate limit")?;
    Ok(())
}

struct PayloadLayout {
    blobs: Vec<BlobDescriptor>,
    encoded: u64,
    restored: u64,
}

fn payload_layout(manifest: &SnapshotManifestV1) -> Result<PayloadLayout> {
    swarm_storage::streaming::validate_manifest_shape(manifest)?;
    let mut by_hash = BTreeMap::<Hash32, BlobDescriptor>::new();
    let mut restored = 0u64;
    for entry in &manifest.entries {
        let blob = &entry.blob;
        if (blob.encoding == BlobEncoding::Raw && blob.encoded_size != blob.uncompressed_size)
            || (blob.encoding == BlobEncoding::Zstd && blob.encoded_size == 0)
        {
            bail!("invalid backup blob descriptor");
        }
        restored = restored.checked_add(blob.uncompressed_size).context("restore byte count exhausted")?;
        if let Some(prior) = by_hash.insert(blob.hash, blob.clone()) {
            if prior != *blob {
                bail!("inconsistent descriptors for the same backup blob");
            }
        }
    }
    let blobs: Vec<_> = by_hash.into_values().collect();
    let encoded = blobs
        .iter()
        .try_fold(0u64, |sum, blob| sum.checked_add(blob.encoded_size))
        .context("backup byte count exhausted")?;
    if encoded > MAX_PAYLOAD_BYTES || restored > MAX_PAYLOAD_BYTES {
        bail!("backup payload exceeds the V1 size limit");
    }
    Ok(PayloadLayout { blobs, encoded, restored })
}

fn receipt(
    operation: &'static str,
    selection: Selection,
    proof: &HistoricalReplayProofV1,
    manifest: &SnapshotManifestV1,
    layout: &PayloadLayout,
    destination: PathBuf,
) -> Result<BackupReceipt> {
    let compatibility = proof.configs.last().context("missing signed backup config")?.compatibility.clone();
    Ok(BackupReceipt {
        format_version: 1,
        operation,
        source_world_id: selection.world.to_string(),
        source_snapshot_number: manifest.snapshot_number,
        source_checkpoint_hash: selection.checkpoint.to_string(),
        source_conflict_digest: selection.acknowledged_conflicts.map(|hash| hash.to_string()),
        state_root: manifest.state_root.to_string(),
        compatibility_fingerprint: compatibility.fingerprint()?.to_string(),
        compatibility,
        files: manifest.entries.len(),
        encoded_payload_bytes: layout.encoded,
        restored_payload_bytes: layout.restored,
        historical_manifest_count: proof.checkpoints.len(),
        destination,
        registered_world: false,
        authority_granted: false,
    })
}

fn metadata_disk_estimate(
    selection: &Selection,
    proof: &HistoricalReplayProofV1,
    manifests: &[SnapshotManifestV1],
) -> Result<u64> {
    let mut remaining = MAX_METADATA_BYTES;
    let mut estimate = HEADER_SIZE as u64 + FRAME_OVERHEAD;
    let mut count = |size: usize, limit: usize| -> Result<()> {
        charge_metadata(size, limit, &mut remaining)?;
        estimate += 4 + FRAME_OVERHEAD + size as u64 + (size.div_ceil(FRAME_BYTES) as u64) * FRAME_OVERHEAD;
        Ok(())
    };
    count(postcard::to_allocvec(selection)?.len(), 128)?;
    count(postcard::to_allocvec(proof)?.len(), MAX_REPLAY_PROOF_BYTES)?;
    for manifest in manifests {
        count(postcard::to_allocvec(manifest)?.len(), MAX_MANIFEST_BYTES)?;
    }
    Ok(estimate)
}

pub fn export(paths: &DataPaths, request: &BackupExportRequest<'_>) -> Result<BackupReceipt> {
    let destination = external_destination(&paths.root, request.destination)?;
    let key = load_key(request.key_file)?;
    let storage = Storage::open(paths.root.clone())?;
    let identity =
        PeerIdentity::load_existing(paths).context("backup export requires the existing admitted peer identity")?;
    let source = crate::historical_authority::verify_selected_recovery_source(
        &storage,
        &identity,
        request.world,
        request.checkpoint,
        request.acknowledged_conflicts,
    )?;
    let (proof, manifests) =
        crate::historical_authority::verified_local_checkpoint_history(&storage, &identity, request.world)?;
    let selection = Selection {
        world: request.world,
        checkpoint: request.checkpoint,
        acknowledged_conflicts: request.acknowledged_conflicts,
    };
    let layout = payload_layout(&source.manifest)?;
    let estimate = metadata_disk_estimate(&selection, &proof, &manifests)?
        + layout.encoded
        + FRAME_OVERHEAD * layout.blobs.iter().map(|blob| blob.encoded_size.div_ceil(FRAME_BYTES as u64)).sum::<u64>();
    storage.require_restore_space(request.world, parent(&destination)?, estimate)?;
    let mut file = private_tempfile(parent(&destination)?, ".swarmcraft-backup-export-")?;
    {
        let mut writer = FrameWriter::new(BufWriter::new(file.as_file_mut()), &key)?;
        let mut remaining = MAX_METADATA_BYTES;
        writer.metadata(&selection, 128, &mut remaining)?;
        writer.metadata(&proof, MAX_REPLAY_PROOF_BYTES, &mut remaining)?;
        for manifest in &manifests {
            writer.metadata(manifest, MAX_MANIFEST_BYTES, &mut remaining)?;
        }
        for blob in &layout.blobs {
            let mut offset = 0;
            while offset < blob.encoded_size {
                let expected = (blob.encoded_size - offset).min(FRAME_BYTES as u64) as usize;
                let (bytes, finished) = storage.read_encoded_blob_chunk(request.world, blob, offset, expected)?;
                let bytes = Zeroizing::new(bytes);
                if bytes.len() != expected || finished != (offset + expected as u64 == blob.encoded_size) {
                    bail!("backup source blob changed during export");
                }
                writer.frame(&bytes)?;
                offset += expected as u64;
            }
            storage.verify_blob_streaming(request.world, blob)?;
        }
        writer.finish()?;
    }
    source.ensure_unchanged(&storage)?;
    let result = receipt("encrypted_export", selection, &proof, &source.manifest, &layout, destination.clone())?;
    publish_file(file, &destination)?;
    Ok(result)
}

fn read_history<R: Read>(
    reader: &mut FrameReader<R>,
    expected_world: WorldId,
    expected_checkpoint: Hash32,
) -> Result<(Selection, HistoricalReplayProofV1, SnapshotManifestV1)> {
    let mut remaining = MAX_METADATA_BYTES;
    let selection: Selection = reader.metadata(128, &mut remaining)?;
    if selection.world != expected_world || selection.checkpoint != expected_checkpoint {
        bail!("backup does not match the independently selected world and checkpoint");
    }
    let proof: HistoricalReplayProofV1 = reader.metadata(MAX_REPLAY_PROOF_BYTES, &mut remaining)?;
    crate::historical_authority::verify_offline_backup_history(&proof, expected_world)?;
    let mut selected = None;
    for (index, checkpoint) in proof.checkpoints.iter().enumerate() {
        let manifest: SnapshotManifestV1 = reader.metadata(MAX_MANIFEST_BYTES, &mut remaining)?;
        if manifest.snapshot_number != index as u64 + 1 || manifest.manifest_hash()? != checkpoint.manifest_hash {
            bail!("backup manifests are not in their exact signed history order");
        }
        crate::historical_authority::verify_manifest_against_history(&proof, &manifest)?;
        if checkpoint.manifest_hash == expected_checkpoint && selected.replace(manifest).is_some() {
            bail!("backup contains duplicate selected checkpoint");
        }
    }
    Ok((selection, proof, selected.context("selected checkpoint is absent from the verified signed history")?))
}

pub fn import(paths: &DataPaths, request: &BackupImportRequest<'_>) -> Result<BackupReceipt> {
    if !request.confirm_data_only {
        bail!("confirm a data-only restore: this does not register a world or restore original authority");
    }
    let destination = external_destination(&paths.root, request.destination)?;
    let key = load_key(request.key_file)?;
    let mut reader = FrameReader::new(BufReader::new(open_regular_input(request.bundle)?), &key)?;
    let (selection, proof, manifest) = read_history(&mut reader, request.expected_world, request.expected_checkpoint)?;
    let layout = payload_layout(&manifest)?;
    require_space(
        parent(&destination)?,
        layout.encoded.checked_add(layout.restored).context("backup staging size overflow")?,
    )?;

    // Original metadata is used solely in an isolated scratch store outside
    // the recipient's data root. Never commit a manifest or install any
    // epoch, membership, lease, invite, identity or runtime there.
    let scratch = private_tempdir(parent(&destination)?)?;
    let isolated = Storage::open(scratch.path().join("verify-store"))?;
    isolated.create_world(&WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: "Isolated backup verification".into(),
        world_id: selection.world,
        genesis: proof.genesis.clone(),
    })?;
    for blob in &layout.blobs {
        if blob.encoded_size == 0 {
            isolated.receive_blob_chunk(selection.world, blob, 0, &[], true)?;
            continue;
        }
        let mut offset = 0;
        while offset < blob.encoded_size {
            let len = (blob.encoded_size - offset).min(FRAME_BYTES as u64) as usize;
            let bytes = reader.frame(len)?;
            let expected = offset + len as u64;
            let next =
                isolated.receive_blob_chunk(selection.world, blob, offset, &bytes, expected == blob.encoded_size)?;
            if next != expected {
                bail!("backup blob verification made unexpected progress");
            }
            offset = next;
        }
    }
    reader.finish()?;
    // The normal restore path independently verifies every blob, portable
    // path and state root after the full ciphertext is known complete.
    let restored = scratch.path().join("restored");
    isolated.restore_snapshot(&manifest, &restored)?;
    let result = receipt("data_only_import", selection, &proof, &manifest, &layout, destination.clone())?;
    publish_import(&destination, &restored, &manifest, &proof, &result)?;
    Ok(result)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn publish_import(
    destination: &Path,
    restored: &Path,
    manifest: &SnapshotManifestV1,
    proof: &HistoricalReplayProofV1,
    result: &BackupReceipt,
) -> Result<()> {
    // Reserve a new directory exclusively. Only a fully verified tree is
    // moved beneath it; no existing save can be replaced.
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(destination).context("backup import destination already exists or cannot be created")?;
    write_new(&destination.join(INCOMPLETE), b"incomplete data-only backup import\n")?;
    sync_dir(destination)?;
    sync_dir(parent(destination)?)?;
    write_new(&destination.join("signed-manifest.postcard"), &postcard::to_allocvec(manifest)?)?;
    write_new(
        &destination.join("signed-config.postcard"),
        &postcard::to_allocvec(proof.configs.last().context("missing backup config")?)?,
    )?;
    sync_dir(restored)?;
    fs::rename(restored, destination.join("world"))?;
    sync_dir(parent(restored)?)?;
    sync_dir(destination)?;
    write_new(&destination.join("backup-receipt.json"), &serde_json::to_vec_pretty(result)?)?;
    sync_dir(destination)?;
    fs::remove_file(destination.join(INCOMPLETE))?;
    sync_dir(destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_import::{import_world, ImportWorldRequest};
    use swarm_protocol::{HistoricalCheckpointV1, WorldVisibilityV1};

    #[test]
    fn frame_authentication_rejects_reordering_splicing_length_changes_and_missing_end() {
        let key = Zeroizing::new([0x4a; 32]);
        let mut first = Vec::new();
        let mut writer = FrameWriter::new(&mut first, &key).unwrap();
        writer.frame(b"one").unwrap();
        writer.frame(b"two").unwrap();
        writer.finish().unwrap();
        let mut reader = FrameReader::new(first.as_slice(), &key).unwrap();
        assert_eq!(reader.frame(3).unwrap().as_slice(), b"one");
        assert_eq!(reader.frame(3).unwrap().as_slice(), b"two");
        reader.finish().unwrap();

        let start = HEADER_SIZE;
        let record = 3 + FRAME_OVERHEAD as usize;
        let mut second = Vec::new();
        let mut writer = FrameWriter::new(&mut second, &key).unwrap();
        writer.frame(b"one").unwrap();
        writer.frame(b"two").unwrap();
        writer.finish().unwrap();
        assert_ne!(&first[8..24], &second[8..24]);

        let mut reordered = first.clone();
        reordered[start..start + record].copy_from_slice(&first[start + record..start + record * 2]);
        reordered[start + record..start + record * 2].copy_from_slice(&first[start..start + record]);
        let mut spliced = first.clone();
        spliced[start..start + record].copy_from_slice(&second[start..start + record]);
        let mut bad_length = first.clone();
        bad_length[start..start + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        for ciphertext in [reordered, spliced, bad_length] {
            let mut reader = FrameReader::new(ciphertext.as_slice(), &key).unwrap();
            assert!(reader.frame(3).is_err());
        }
        let mut reader = FrameReader::new(&first[..first.len() - FRAME_OVERHEAD as usize], &key).unwrap();
        reader.frame(3).unwrap();
        reader.frame(3).unwrap();
        assert!(reader.finish().is_err());
        let mut forged_end = first;
        let last = forged_end.len() - 1;
        forged_end[last] ^= 0x1;
        let mut reader = FrameReader::new(forged_end.as_slice(), &key).unwrap();
        reader.frame(3).unwrap();
        reader.frame(3).unwrap();
        assert!(reader.finish().is_err());
    }

    #[test]
    fn authenticated_metadata_length_is_bounded_before_record_allocation() {
        let key = Zeroizing::new([0x4b; 32]);
        let mut ciphertext = Vec::new();
        let mut writer = FrameWriter::new(&mut ciphertext, &key).unwrap();
        writer.frame(&u32::MAX.to_be_bytes()).unwrap();
        writer.finish().unwrap();
        let mut reader = FrameReader::new(ciphertext.as_slice(), &key).unwrap();
        let mut remaining = MAX_METADATA_BYTES;
        assert!(reader.metadata::<Selection>(128, &mut remaining).is_err());
        assert_eq!(reader.counter, 1, "must reject oversized metadata before reading its body");
    }

    #[test]
    fn key_holder_cannot_substitute_unsigned_history_or_corrupted_signed_payload() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("origin"));
        let recipient = DataPaths::from_root(temp.path().join("untouched"));
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("level.dat"), b"original signed payload").unwrap();
        fs::write(source.join("session.lock"), b"backup test lock").unwrap();
        let imported = import_world(
            &paths,
            &ImportWorldRequest {
                source,
                name: "Backup fixture".into(),
                minecraft_version: "26.1.2".into(),
                fabric_loader_version: "0.19.3".into(),
                visibility: WorldVisibilityV1::Private,
                server_mod_jars: vec![],
                confirm_no_server_mods: true,
            },
        )
        .unwrap();
        let world: WorldId = imported.world_id.parse().unwrap();
        let storage = Storage::open(&paths.root).unwrap();
        let identity = PeerIdentity::load_or_create(&paths).unwrap();
        let (proof, manifests) =
            crate::historical_authority::verified_local_checkpoint_history(&storage, &identity, world).unwrap();
        let manifest = &manifests[0];
        let layout = payload_layout(manifest).unwrap();
        let selection =
            Selection { world, checkpoint: manifest.manifest_hash().unwrap(), acknowledged_conflicts: None };
        let key_path = temp.path().join("key");
        generate_key(&recipient, &key_path).unwrap();
        let key = load_key(&key_path).unwrap();

        // Construct valid AEAD frames with malicious plaintext; this is
        // stronger than corrupting ciphertext, because the key is known.
        let mut invalid_manifest = manifest.clone();
        invalid_manifest.signature[0] ^= 1;
        let mut forged_proof = proof.clone();
        forged_proof.checkpoints[0] = HistoricalCheckpointV1::from_manifest(&invalid_manifest).unwrap();
        let forged_selection = Selection { checkpoint: invalid_manifest.manifest_hash().unwrap(), ..selection };
        let mut forged = Vec::new();
        let mut writer = FrameWriter::new(&mut forged, &key).unwrap();
        let mut budget = MAX_METADATA_BYTES;
        writer.metadata(&forged_selection, 128, &mut budget).unwrap();
        writer.metadata(&forged_proof, MAX_REPLAY_PROOF_BYTES, &mut budget).unwrap();
        writer.metadata(&invalid_manifest, MAX_MANIFEST_BYTES, &mut budget).unwrap();
        writer.finish().unwrap();
        let mut reader = FrameReader::new(forged.as_slice(), &key).unwrap();
        assert!(read_history(&mut reader, world, forged_selection.checkpoint).is_err());

        let mut bad_membership = proof.clone();
        bad_membership.membership[0].signature[0] ^= 1;
        let mut forged = Vec::new();
        let mut writer = FrameWriter::new(&mut forged, &key).unwrap();
        let mut budget = MAX_METADATA_BYTES;
        writer.metadata(&selection, 128, &mut budget).unwrap();
        writer.metadata(&bad_membership, MAX_REPLAY_PROOF_BYTES, &mut budget).unwrap();
        writer.metadata(manifest, MAX_MANIFEST_BYTES, &mut budget).unwrap();
        writer.finish().unwrap();
        let mut reader = FrameReader::new(forged.as_slice(), &key).unwrap();
        assert!(read_history(&mut reader, world, selection.checkpoint).is_err());

        let bundle = temp.path().join("authenticated-corrupt.scbackup");
        let mut writer = FrameWriter::new(File::create(&bundle).unwrap(), &key).unwrap();
        let mut budget = MAX_METADATA_BYTES;
        writer.metadata(&selection, 128, &mut budget).unwrap();
        writer.metadata(&proof, MAX_REPLAY_PROOF_BYTES, &mut budget).unwrap();
        writer.metadata(manifest, MAX_MANIFEST_BYTES, &mut budget).unwrap();
        let mut changed = false;
        for blob in &layout.blobs {
            let mut offset = 0;
            while offset < blob.encoded_size {
                let (mut bytes, _) = storage.read_encoded_blob_chunk(world, blob, offset, FRAME_BYTES).unwrap();
                if !changed && !bytes.is_empty() {
                    bytes[0] ^= 0x20;
                    changed = true;
                }
                writer.frame(&bytes).unwrap();
                offset += bytes.len() as u64;
            }
        }
        assert!(changed);
        writer.finish().unwrap();
        let destination = temp.path().join("must-not-be-restored");
        let request = BackupImportRequest {
            expected_world: world,
            expected_checkpoint: selection.checkpoint,
            key_file: &key_path,
            bundle: &bundle,
            destination: &destination,
            confirm_data_only: true,
        };
        assert!(import(&recipient, &request).is_err());
        assert!(!destination.exists());
        assert!(!recipient.root.exists());
        assert_eq!(storage.latest_snapshot(world).unwrap().as_ref(), Some(manifest));
    }

    #[test]
    fn file_publication_never_replaces_a_racing_destination() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("occupied");
        let mut staged = private_tempfile(temp.path(), ".backup-test-").unwrap();
        staged.write_all(b"completed but competing bundle").unwrap();
        fs::write(&destination, b"previous backup").unwrap();
        assert!(publish_file(staged, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous backup");
    }
}
