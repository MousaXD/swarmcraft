//! Offline, separately verified save bundles. No canonical storage, JVM,
//! hosting grant or runtime permit is created. Only cooperative local
//! devices with a stopped Minecraft save are in the trust model.
use super::Reviewed;
use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use swarm_core::DataPaths;
use swarm_protocol::Hash32;

#[derive(Debug, Args)]
pub struct Digests {
    #[arg(long)]
    expected_manifest: Hash32,
    #[arg(long)]
    expected_state_root: Hash32,
    #[arg(long)]
    expected_compatibility: Hash32,
}

#[derive(Debug, Subcommand)]
pub enum SaveCommand {
    /// Capture a stopped Minecraft-shaped save into a new immutable bundle.
    Capture {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        compatibility: Hash32,
        #[arg(long)]
        confirm: bool,
    },
    /// Verify the exact independently pinned manifest and every blob.
    Verify {
        #[arg(long)]
        bundle: PathBuf,
        #[command(flatten)]
        digests: Digests,
    },
    /// Restore verified data into a new private directory, never an existing save.
    Restore {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[command(flatten)]
        digests: Digests,
        #[arg(long)]
        confirm: bool,
    },
    /// Serve exact save bytes for a locally prepared and independently reviewed offer.
    Serve {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_offer: Hash32,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        bind: std::net::SocketAddr,
    },
    /// Fetch and durably verify a reviewed offer's save data; no turn is granted.
    Fetch {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_offer: Hash32,
        #[arg(long)]
        offer: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        peer: String,
    },
}

#[cfg(not(target_os = "linux"))]
pub(super) fn run(_paths: &DataPaths, _command: SaveCommand) -> Result<()> {
    anyhow::bail!("offline exclusive-turn save verification is qualified on Linux only")
}

#[cfg(target_os = "linux")]
pub(super) fn run(paths: &DataPaths, command: SaveCommand) -> Result<()> {
    linux::run(paths, command)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::super::{
        check_dir, create_dir, open_regular, opened, print_document, private_file, read_document, sync_dir, Phase,
    };
    use super::*;
    use crate::reference_channel::{authenticate, private_ip, remaining, Channel};
    use anyhow::{bail, ensure, Context};
    use serde::{Deserialize, Serialize};
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::CString,
        fs::{self, File, Metadata, OpenOptions},
        io::{Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        os::unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawFd,
        },
        path::{Component, Path},
        thread,
        time::{Duration, Instant},
    };
    use swarm_consensus::exclusive_turn::verify_exclusive_turn_offer;
    use swarm_protocol::{
        exclusive_turn_reference_commitment, ExclusiveTurnCheckpointV1, ExclusiveTurnSignedOfferV1,
        ExclusiveTurnWorldIdV1, PeerId, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES,
    };

    const MAX_ENTRIES: usize = 4096;
    const MAX_FILE: u64 = 512 * 1024 * 1024;
    const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
    const DISK_RESERVE: u64 = 1024 * 1024 * 1024;
    const CHUNK: usize = 32 * 1024;
    const SESSION: Duration = Duration::from_secs(120);
    const MANIFEST_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/save-manifest/v1\0";
    const STATE_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/save-state/v1\0";
    const BLOB_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/save-blob/v1\0";
    const NETWORK_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/save-transfer/v1\0";
    const AUTH_DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/save-transfer-auth/v1\0";

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum Kind {
        Directory,
        File { size: u64, hash: Hash32 },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Entry {
        path: String,
        kind: Kind,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Manifest {
        format: u16,
        compatibility: Hash32,
        entries: Vec<Entry>,
    }

    // Strict ASCII portable namespace; validate directories as well as
    // files so case aliases and file/directory collisions cannot hide
    // in an otherwise unique list of file paths.
    fn portable(path: &str) -> Result<()> {
        ensure!(path.len() <= 512 && !path.is_empty() && path.is_ascii(), "unsafe save path");
        for part in path.split('/') {
            ensure!(
                !part.is_empty()
                    && part.len() <= 128
                    && part != "."
                    && part != ".."
                    && !part.ends_with(['.', ' '])
                    && !part.bytes().any(|b| b < 32
                        || b == 127
                        || matches!(b, b'\\' | b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*')),
                "unsafe save path {path}"
            );
            let folded = part.to_ascii_lowercase();
            let stem = folded.split('.').next().expect("nonempty component");
            ensure!(
                !matches!(stem, "con" | "prn" | "aux" | "nul")
                    && !matches!(
                        stem.strip_prefix("com").or_else(|| stem.strip_prefix("lpt")),
                        Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                    ),
                "reserved portable save path {path}"
            );
        }
        ensure!(path.split('/').count() <= 32 && !path.eq_ignore_ascii_case("session.lock"), "unsafe save path {path}");
        Ok(())
    }

    impl Manifest {
        fn validate(&self) -> Result<u64> {
            ensure!(
                self.format == 1 && self.compatibility != Hash32::default(),
                "invalid save manifest format or compatibility"
            );
            ensure!((1..=MAX_ENTRIES).contains(&self.entries.len()), "save manifest entry count out of bounds");
            let mut previous = "";
            let mut dirs = BTreeSet::new();
            let mut aliases = BTreeSet::new();
            let mut total = 0u64;
            let mut level = false;
            let mut blobs = BTreeMap::new();
            for entry in &self.entries {
                portable(&entry.path)?;
                ensure!(
                    entry.path.as_str() > previous && aliases.insert(entry.path.to_ascii_lowercase()),
                    "duplicate or noncanonical save path"
                );
                previous = &entry.path;
                if let Some((parent, _)) = entry.path.rsplit_once('/') {
                    ensure!(dirs.contains(parent), "missing or non-directory save parent");
                }
                match entry.kind {
                    Kind::Directory => {
                        dirs.insert(entry.path.as_str());
                    }
                    Kind::File { size, hash } => {
                        ensure!(size <= MAX_FILE && hash != Hash32::default(), "save blob size or digest invalid");
                        total = total.checked_add(size).context("save size overflow")?;
                        ensure!(total <= MAX_TOTAL, "save total size exceeds bound");
                        if let Some(old) = blobs.insert(hash, size) {
                            ensure!(old == size, "conflicting save blob lengths");
                        }
                        if entry.path == "level.dat" && size > 0 {
                            level = true;
                        }
                    }
                }
            }
            ensure!(level, "save requires nonempty level.dat");
            Ok(total)
        }

        fn encode(&self) -> Result<Vec<u8>> {
            self.validate()?;
            let bytes = postcard::to_allocvec(self)?;
            ensure!(bytes.len() <= MAX_EXCLUSIVE_TURN_REFERENCE_BYTES, "save manifest exceeds byte bound");
            Ok(bytes)
        }

        fn decode(bytes: &[u8]) -> Result<Self> {
            ensure!(
                !bytes.is_empty() && bytes.len() <= MAX_EXCLUSIVE_TURN_REFERENCE_BYTES,
                "save manifest length out of bounds"
            );
            let value: Self = postcard::from_bytes(bytes)?;
            ensure!(value.encode()? == bytes, "noncanonical save manifest");
            Ok(value)
        }

        fn checkpoint(&self) -> Result<ExclusiveTurnCheckpointV1> {
            let bytes = self.encode()?;
            Ok(ExclusiveTurnCheckpointV1 {
                manifest_hash: Hash32::from_domain_bytes(MANIFEST_DOMAIN, &bytes),
                state_root: Hash32::from_domain_bytes(STATE_DOMAIN, &postcard::to_allocvec(&self.entries)?),
                reference_commitment: exclusive_turn_reference_commitment(&bytes)?,
            })
        }

        fn blobs(&self) -> BTreeMap<Hash32, u64> {
            self.entries
                .iter()
                .filter_map(|entry| match entry.kind {
                    Kind::Directory => None,
                    Kind::File { hash, size } => Some((hash, size)),
                })
                .collect()
        }

        fn check_pins(&self, pins: &Digests) -> Result<()> {
            let checkpoint = self.checkpoint()?;
            ensure!(
                checkpoint.manifest_hash == pins.expected_manifest
                    && checkpoint.state_root == pins.expected_state_root
                    && self.compatibility == pins.expected_compatibility,
                "save manifest, state root or compatibility differs from independent pin"
            );
            Ok(())
        }

        fn check_offer(&self, offer: &ExclusiveTurnSignedOfferV1) -> Result<()> {
            ensure!(
                self.checkpoint()? == offer.offer.checkpoint && self.compatibility == offer.offer.compatibility,
                "save bundle does not match exact signed offer checkpoint and compatibility"
            );
            Ok(())
        }
    }

    fn hash_start() -> blake3::Hasher {
        let mut hasher = blake3::Hasher::new();
        hasher.update(BLOB_DOMAIN);
        hasher
    }

    fn hash_finish(hasher: blake3::Hasher) -> Hash32 {
        Hash32(*hasher.finalize().as_bytes())
    }

    fn uid() -> u32 {
        // SAFETY: no pointer or lifetime precondition.
        unsafe { libc::geteuid() }
    }

    fn safe_directory(path: &Path) -> Result<PathBuf> {
        let path = std::path::absolute(path)?;
        ensure!(!path.components().any(|c| c == Component::ParentDir), "unsafe save directory traversal");
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component.as_os_str());
            let meta = fs::symlink_metadata(&current)?;
            ensure!(
                meta.is_dir()
                    && (meta.uid() == uid() || meta.uid() == 0)
                    && (meta.mode() & 0o022 == 0 || (meta.uid() == 0 && meta.mode() & 0o1000 != 0)),
                "unsafe save directory ancestor {}",
                current.display()
            );
        }
        Ok(path)
    }

    fn source_file(path: &Path) -> Result<File> {
        let file = open_regular(path)?;
        let meta = file.metadata()?;
        ensure!(
            meta.is_file() && meta.nlink() == 1 && meta.uid() == uid() && meta.mode() & 0o022 == 0,
            "unsafe save file ownership or links"
        );
        Ok(file)
    }

    // POSIX record lock, the same kernel lock family as Java FileChannel.
    // Do not open/close another fd to this inode while it is held.
    struct SessionLock {
        file: File,
        path: PathBuf,
    }

    impl SessionLock {
        fn acquire(source: &Path) -> Result<Self> {
            let path = source.join("session.lock");
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&path)
                .context("existing Minecraft session.lock is required")?;
            let meta = file.metadata()?;
            ensure!(
                meta.is_file() && meta.uid() == uid() && meta.nlink() == 1 && meta.mode() & 0o022 == 0,
                "unsafe Minecraft session.lock"
            );
            let mut lock: libc::flock = unsafe { std::mem::zeroed() };
            lock.l_type = libc::F_WRLCK as _;
            lock.l_whence = libc::SEEK_SET as _;
            // SAFETY: valid owned fd and initialized flock for this call.
            let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) };
            ensure!(
                result == 0,
                "Minecraft save is open or its session lock cannot be acquired: {}",
                std::io::Error::last_os_error()
            );
            Ok(Self { file, path })
        }

        fn check(&self) -> Result<()> {
            let named = fs::symlink_metadata(&self.path)?;
            let held = self.file.metadata()?;
            ensure!(
                named.is_file() && named.nlink() == 1 && named.dev() == held.dev() && named.ino() == held.ino(),
                "Minecraft session lock identity changed"
            );
            Ok(())
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Stamp {
        dev: u64,
        ino: u64,
        mode: u32,
        size: u64,
        mtime: i64,
        mtime_ns: i64,
        ctime: i64,
        ctime_ns: i64,
    }

    impl Stamp {
        fn from(meta: &Metadata) -> Self {
            Self {
                dev: meta.dev(),
                ino: meta.ino(),
                mode: meta.mode(),
                size: meta.len(),
                mtime: meta.mtime(),
                mtime_ns: meta.mtime_nsec(),
                ctime: meta.ctime(),
                ctime_ns: meta.ctime_nsec(),
            }
        }
    }

    fn scan(source: &Path) -> Result<BTreeMap<String, Stamp>> {
        let mut result = BTreeMap::new();
        let mut total = 0u64;
        let device = fs::metadata(source)?.dev();
        for item in walkdir::WalkDir::new(source).follow_links(false).max_depth(33) {
            let item = item?;
            if item.depth() == 0 {
                continue;
            }
            let path = item.path().strip_prefix(source)?.to_str().context("non-UTF8 save path")?;
            if path == "session.lock" {
                continue;
            }
            portable(path)?;
            ensure!(result.len() < MAX_ENTRIES, "save entry limit exceeded");
            let meta = fs::symlink_metadata(item.path())?;
            ensure!(
                meta.dev() == device
                    && meta.uid() == uid()
                    && meta.mode() & 0o022 == 0
                    && (meta.is_dir() || (meta.is_file() && meta.nlink() == 1)),
                "unsafe save entry {path}"
            );
            ensure!(item.depth() <= 32, "save path too deep");
            if meta.is_file() {
                ensure!(meta.len() <= MAX_FILE, "save file exceeds size bound");
                total = total.checked_add(meta.len()).context("save size overflow")?;
                ensure!(total <= MAX_TOTAL, "save total size exceeds bound");
            }
            result.insert(path.to_owned(), Stamp::from(&meta));
        }
        Ok(result)
    }

    fn absent(path: &Path) -> Result<()> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => bail!("save destination already exists; no replacement permitted"),
            Err(error) => Err(error.into()),
        }
    }

    struct Publication {
        temp: tempfile::TempDir,
        destination: PathBuf,
    }

    impl Publication {
        fn new(destination: &Path, bytes: u64) -> Result<Self> {
            let parent =
                safe_directory(destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?;
            check_dir(&parent)?;
            let destination = parent.join(destination.file_name().context("invalid save destination")?);
            absent(&destination)?;
            ensure!(
                fs2::available_space(&parent)?
                    >= bytes.checked_add(DISK_RESERVE).context("save reservation overflow")?,
                "insufficient disk space for verified save publication"
            );
            let temp = tempfile::Builder::new().prefix(".exclusive-save-").tempdir_in(&parent)?;
            check_dir(temp.path())?;
            Ok(Self { temp, destination })
        }

        fn path(&self) -> &Path {
            self.temp.path()
        }

        fn finish(self) -> Result<PathBuf> {
            sync_dir(self.path())?;
            let source = CString::new(self.path().as_os_str().as_bytes())?;
            let target = CString::new(self.destination.as_os_str().as_bytes())?;
            // SAFETY: both C strings are live. Never fall back to a
            // replacement-capable rename or cross-filesystem copy.
            let result = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    libc::AT_FDCWD,
                    target.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error()).context("atomic no-replace save publication failed");
            }
            sync_dir(self.destination.parent().context("missing save parent")?)?;
            Ok(self.destination)
        }
    }

    fn create_file(path: &Path) -> Result<File> {
        Ok(OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?)
    }

    fn copy_blob(mut input: &File, mut output: Option<&mut File>, size: u64) -> Result<Hash32> {
        let mut hash = hash_start();
        let mut left = size;
        let mut buffer = [0u8; CHUNK];
        while left > 0 {
            let len = left.min(CHUNK as u64) as usize;
            input.read_exact(&mut buffer[..len])?;
            hash.update(&buffer[..len]);
            if let Some(output) = output.as_deref_mut() {
                output.write_all(&buffer[..len])?;
            }
            left -= len as u64;
        }
        ensure!(input.read(&mut buffer[..1])? == 0, "save blob length differs");
        Ok(hash_finish(hash))
    }

    fn blob_path(bundle: &Path, hash: Hash32) -> PathBuf {
        bundle.join("blobs").join(hash.to_string())
    }

    fn store_manifest(bundle: &Path, manifest: &Manifest) -> Result<()> {
        let mut file = create_file(&bundle.join("manifest.postcard"))?;
        file.write_all(&manifest.encode()?)?;
        file.sync_all()?;
        sync_dir(&bundle.join("blobs"))?;
        sync_dir(bundle)
    }

    fn load_bundle(bundle: &Path) -> Result<Manifest> {
        safe_directory(bundle)?;
        check_dir(bundle)?;
        check_dir(&bundle.join("blobs"))?;
        let names =
            fs::read_dir(bundle)?.take(3).map(|e| Ok(e?.file_name())).collect::<std::io::Result<BTreeSet<_>>>()?;
        ensure!(
            names == ["manifest.postcard".into(), "blobs".into()].into_iter().collect(),
            "unexpected save bundle files"
        );
        let file = private_file(&bundle.join("manifest.postcard"))?;
        let bytes = super::super::read_bounded(file, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
        Manifest::decode(&bytes)
    }

    fn verify_bundle(bundle: &Path, manifest: &Manifest) -> Result<()> {
        ensure!(load_bundle(bundle)? == *manifest, "save manifest changed");
        let expected = manifest.blobs();
        let mut seen = BTreeSet::new();
        for entry in fs::read_dir(bundle.join("blobs"))? {
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|_| anyhow::anyhow!("invalid save blob name"))?;
            let hash: Hash32 = name.parse()?;
            ensure!(hash.to_string() == name && seen.insert(hash), "noncanonical save blob filename");
            let size = *expected.get(&hash).context("unexpected save blob")?;
            let file = private_file(&entry.path())?;
            ensure!(
                file.metadata()?.len() == size && copy_blob(&file, None, size)? == hash,
                "save blob digest or length differs"
            );
            file.sync_all()?;
        }
        ensure!(seen.len() == expected.len(), "save bundle missing blob");
        private_file(&bundle.join("manifest.postcard"))?.sync_all()?;
        sync_dir(&bundle.join("blobs"))?;
        sync_dir(bundle)?;
        Ok(())
    }

    fn capture(source: &Path, destination: &Path, compatibility: Hash32) -> Result<(Manifest, PathBuf)> {
        ensure!(compatibility != Hash32::default(), "save compatibility must be nonzero");
        let source = safe_directory(source)?;
        ensure!(fs::metadata(&source)?.uid() == uid(), "save source must belong to local user");
        let root_stamp = Stamp::from(&fs::metadata(&source)?);
        let lock = SessionLock::acquire(&source)?;
        let entries = scan(&source)?;
        let size = entries.values().filter(|s| s.mode & libc::S_IFMT == libc::S_IFREG).map(|s| s.size).sum();
        let parent =
            safe_directory(destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?;
        ensure!(!parent.starts_with(&source), "save destination cannot be inside source");
        let publication = Publication::new(destination, size)?;
        create_dir(&publication.path().join("blobs"))?;
        let mut manifest = Manifest { format: 1, compatibility, entries: Vec::new() };
        for (name, stamp) in &entries {
            let kind = if stamp.mode & libc::S_IFMT == libc::S_IFDIR {
                Kind::Directory
            } else {
                let file = source_file(&source.join(name))?;
                ensure!(Stamp::from(&file.metadata()?) == *stamp, "save file changed before capture");
                let mut temp = tempfile::NamedTempFile::new_in(publication.path().join("blobs"))?;
                let hash = copy_blob(&file, Some(temp.as_file_mut()), stamp.size)?;
                ensure!(Stamp::from(&file.metadata()?) == *stamp, "save file changed during capture");
                temp.as_file().sync_all()?;
                let target = blob_path(publication.path(), hash);
                if target.exists() {
                    let existing = private_file(&target)?;
                    ensure!(copy_blob(&existing, None, stamp.size)? == hash, "save blob collision");
                } else {
                    temp.persist_noclobber(&target).map_err(|e| e.error)?;
                }
                Kind::File { size: stamp.size, hash }
            };
            manifest.entries.push(Entry { path: name.clone(), kind });
        }
        lock.check()?;
        ensure!(
            Stamp::from(&fs::metadata(&source)?) == root_stamp && scan(&source)? == entries,
            "save tree changed during capture"
        );
        store_manifest(publication.path(), &manifest)?;
        verify_bundle(publication.path(), &manifest)?;
        lock.check()?;
        ensure!(
            Stamp::from(&fs::metadata(&source)?) == root_stamp && scan(&source)? == entries,
            "save tree changed before publication"
        );
        Ok((manifest, publication.finish()?))
    }

    fn restore(bundle: &Path, destination: &Path, manifest: &Manifest) -> Result<PathBuf> {
        verify_bundle(bundle, manifest)?;
        let parent =
            safe_directory(destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?;
        ensure!(!parent.starts_with(safe_directory(bundle)?), "save restore cannot be inside bundle");
        let publication = Publication::new(destination, manifest.validate()?)?;
        for entry in &manifest.entries {
            let target = publication.path().join(&entry.path);
            match entry.kind {
                Kind::Directory => create_dir(&target)?,
                Kind::File { size, hash } => {
                    let input = private_file(&blob_path(bundle, hash))?;
                    let mut output = create_file(&target)?;
                    ensure!(copy_blob(&input, Some(&mut output), size)? == hash, "save blob changed during restore");
                    output.sync_all()?;
                }
            }
        }
        // Regenerate only the transient root lock. It is deliberately not
        // copied or counted in the state root.
        create_file(&publication.path().join("session.lock"))?.sync_all()?;
        for entry in manifest.entries.iter().rev() {
            if entry.kind == Kind::Directory {
                sync_dir(&publication.path().join(&entry.path))?;
            }
        }
        let actual = scan(publication.path())?;
        ensure!(actual.len() == manifest.entries.len(), "restored save tree differs");
        for entry in &manifest.entries {
            let stamp = actual.get(&entry.path).context("restored save entry missing")?;
            match entry.kind {
                Kind::Directory => ensure!(stamp.mode & libc::S_IFMT == libc::S_IFDIR, "restored save type differs"),
                Kind::File { size, hash } => {
                    ensure!(
                        stamp.size == size
                            && copy_blob(&private_file(&publication.path().join(&entry.path))?, None, size)? == hash,
                        "restored save bytes differ"
                    );
                }
            }
        }
        publication.finish()
    }

    fn report(manifest: &Manifest, path: &Path, classification: &str) -> Result<()> {
        let checkpoint = manifest.checkpoint()?;
        print_document(&serde_json::json!({
            "classification": classification, "path": path, "manifest_hash": checkpoint.manifest_hash.to_string(),
            "state_root": checkpoint.state_root.to_string(), "reference_commitment": checkpoint.reference_commitment.to_string(),
            "compatibility": manifest.compatibility.to_string(), "entries": manifest.entries.len(),
            "unique_blobs": manifest.blobs().len(), "total_bytes": manifest.validate()?,
            "canonical_authority": false, "write_authority": false, "runtime_activation": false,
            "player_admission": false, "handoff_receipt_issued": false, "minecraft_gameplay_verified": false,
        }))
    }

    #[derive(Serialize)]
    struct Scope {
        world: ExclusiveTurnWorldIdV1,
        base: Hash32,
        offer: Hash32,
        source: PeerId,
        recipient: PeerId,
    }

    fn prologue(offer: &ExclusiveTurnSignedOfferV1) -> Result<Vec<u8>> {
        let mut bytes = NETWORK_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(&Scope {
            world: offer.offer.world_id,
            base: offer.offer.parent_grant,
            offer: offer.offer_hash()?,
            source: offer.offer.source,
            recipient: offer.offer.recipient,
        })?);
        Ok(bytes)
    }

    fn accept(listener: &TcpListener) -> Result<TcpStream> {
        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            remaining(deadline)?;
            match listener.accept() {
                Ok((stream, address)) => {
                    ensure!(private_ip(address.ip()), "save peer address not private");
                    return Ok(stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5).min(remaining(deadline)?))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn send_blob(channel: &mut Channel, bundle: &Path, hash: Hash32, size: u64) -> Result<()> {
        let mut file = private_file(&blob_path(bundle, hash))?;
        let mut left = size;
        let mut buffer = [0u8; CHUNK];
        let mut digest = hash_start();
        while left > 0 {
            let len = left.min(CHUNK as u64) as usize;
            file.read_exact(&mut buffer[..len])?;
            digest.update(&buffer[..len]);
            channel.send_chunk(&buffer[..len])?;
            left -= len as u64;
        }
        ensure!(file.read(&mut buffer[..1])? == 0 && hash_finish(digest) == hash, "save blob changed during transfer");
        Ok(())
    }

    fn receive_blob(channel: &mut Channel, bundle: &Path, hash: Hash32, size: u64) -> Result<()> {
        let mut file = create_file(&blob_path(bundle, hash))?;
        let mut left = size;
        let mut digest = hash_start();
        while left > 0 {
            let chunk = channel.recv_chunk(left.min(CHUNK as u64) as usize)?;
            digest.update(&chunk);
            file.write_all(&chunk)?;
            left -= chunk.len() as u64;
        }
        ensure!(hash_finish(digest) == hash, "received save blob digest differs");
        file.sync_all()?;
        Ok(())
    }

    fn serve(paths: &DataPaths, review: &Reviewed, expected: Hash32, bundle: &Path, bind: SocketAddr) -> Result<()> {
        review.confirm()?;
        ensure!(private_ip(bind.ip()), "save listener must use an explicit private address");
        let (store, journal, head) = opened(paths, &review.pin)?;
        let Phase::Prepared(offer) = &journal.phase else {
            bail!("save transfer requires exact prepared reference offer")
        };
        ensure!(
            offer.offer_hash()? == expected && offer.offer.source == journal.identity,
            "prepared save offer differs from pin"
        );
        let manifest = load_bundle(bundle)?;
        manifest.check_offer(offer)?;
        ensure!(
            store.state(journal.checkpoint)? == manifest.encode()?,
            "prepared journal does not retain exact save manifest"
        );
        verify_bundle(bundle, &manifest)?;
        let key = head
            .devices()
            .iter()
            .find(|d| d.peer_id == offer.offer.recipient)
            .context("save recipient not fixed device")?
            .public_key;
        let listener = TcpListener::bind(bind)?;
        print_document(&serde_json::json!({
            "classification": "exclusive_turn_save_listener", "address": listener.local_addr()?.to_string(),
            "world_id": head.world().to_string(), "base": head.grant().to_string(), "offer": expected.to_string(),
            "runtime_activation": false, "handoff_receipt_issued": false,
        }))?;
        let stream = accept(&listener)?;
        drop(listener);
        let deadline = Instant::now() + SESSION;
        let mut channel = authenticate(stream, &prologue(offer)?, AUTH_DOMAIN, &store.identity, key, false, deadline)?;
        channel.send(&manifest.encode()?, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
        for (hash, size) in manifest.blobs() {
            send_blob(&mut channel, bundle, hash, size)?;
        }
        // Encrypted completion marker is not the signed handoff receipt.
        // It confirms only that the recipient completed this data exchange.
        let ack = channel.recv_chunk(32)?;
        ensure!(ack == manifest.checkpoint()?.manifest_hash.0, "save transfer acknowledgment differs");
        remaining(deadline)?;
        Ok(())
    }

    fn fetch(
        paths: &DataPaths,
        review: &Reviewed,
        expected: Hash32,
        file: &Path,
        destination: &Path,
        peer: &str,
    ) -> Result<()> {
        review.confirm()?;
        let (store, journal, head) = opened(paths, &review.pin)?;
        ensure!(
            matches!(journal.phase, Phase::Replica | Phase::Released),
            "save recipient cannot fetch in current reference phase"
        );
        let offer: ExclusiveTurnSignedOfferV1 = read_document(file)?;
        verify_exclusive_turn_offer(&head, &offer)?;
        ensure!(
            offer.offer_hash()? == expected && offer.offer.recipient == journal.identity,
            "incoming save offer differs from pin"
        );
        if let Some(staged) = &journal.staged {
            ensure!(
                staged.offer == offer || offer.offer.attempt > staged.offer.offer.attempt,
                "stale save transfer offer"
            );
        }
        let (remote, address) = peer.split_once('@').context("save peer requires identity@address")?;
        let remote: PeerId = remote.parse()?;
        let address: SocketAddr = address.parse()?;
        ensure!(
            remote == offer.offer.source && private_ip(address.ip()) && address.port() != 0,
            "save peer differs or address not private"
        );
        let key =
            head.devices().iter().find(|d| d.peer_id == remote).context("save source not fixed device")?.public_key;
        // Refuse existing destinations before connecting. Publication also
        // uses a kernel no-clobber rename after complete verification.
        absent(destination)?;
        let deadline = Instant::now() + SESSION;
        let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
        let mut channel = authenticate(stream, &prologue(&offer)?, AUTH_DOMAIN, &store.identity, key, true, deadline)?;
        let bytes = channel.receive(MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
        let manifest = Manifest::decode(&bytes)?;
        manifest.check_offer(&offer)?;
        let publication = Publication::new(destination, manifest.validate()?)?;
        create_dir(&publication.path().join("blobs"))?;
        for (hash, size) in manifest.blobs() {
            receive_blob(&mut channel, publication.path(), hash, size)?;
        }
        store_manifest(publication.path(), &manifest)?;
        verify_bundle(publication.path(), &manifest)?;
        remaining(deadline)?;
        let destination = publication.finish()?;
        channel.send_chunk(&manifest.checkpoint()?.manifest_hash.0)?;
        report(&manifest, &destination, "exclusive_turn_save_data_received")
    }

    pub(super) fn run(paths: &DataPaths, command: SaveCommand) -> Result<()> {
        match command {
            SaveCommand::Capture { source, destination, compatibility, confirm } => {
                ensure!(confirm, "explicit --confirm required for offline save capture");
                let (manifest, destination) = capture(&source, &destination, compatibility)?;
                report(&manifest, &destination, "exclusive_turn_save_data_captured")
            }
            SaveCommand::Verify { bundle, digests } => {
                let manifest = load_bundle(&bundle)?;
                manifest.check_pins(&digests)?;
                verify_bundle(&bundle, &manifest)?;
                report(&manifest, &bundle, "exclusive_turn_save_data_verified")
            }
            SaveCommand::Restore { bundle, destination, digests, confirm } => {
                ensure!(confirm, "explicit --confirm required for offline save restoration");
                let manifest = load_bundle(&bundle)?;
                manifest.check_pins(&digests)?;
                let destination = restore(&bundle, &destination, &manifest)?;
                report(&manifest, &destination, "exclusive_turn_save_data_restored")
            }
            SaveCommand::Serve { review, expected_offer, bundle, bind } => {
                serve(paths, &review, expected_offer, &bundle, bind)
            }
            SaveCommand::Fetch { review, expected_offer, offer, destination, peer } => {
                fetch(paths, &review, expected_offer, &offer, &destination, &peer)
            }
        }
    }
}
