//! Offline, separately verified save bundles. No canonical storage, JVM,
//! hosting grant or runtime permit is created. Only cooperative local
//! devices with a stopped Minecraft save are in the trust model.
use super::{Reviewed, Store};
use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use swarm_core::DataPaths;
use swarm_protocol::{ExclusiveTurnCheckpointV1, ExclusiveTurnSignedOfferV1, ExclusiveTurnWorldIdV1, Hash32, PeerId};

#[derive(Debug, Args)]
pub struct Digests {
    #[arg(long)]
    expected_manifest: Hash32,
    #[arg(long)]
    expected_state_root: Hash32,
    #[arg(long)]
    expected_compatibility: Hash32,
}

#[derive(Debug, Args)]
pub struct CoreArtifacts {
    /// Extracted Minecraft server JAR, not the bootstrap/bundler JAR.
    #[arg(long)]
    minecraft_jar: PathBuf,
    #[arg(long)]
    fabric_loader_jar: PathBuf,
    #[arg(long)]
    bridge_jar: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum SaveCommand {
    /// Draft a new save-backed reference policy, not a playable world.
    Draft {
        #[arg(long = "device-key", required = true)]
        device_keys: Vec<String>,
        #[arg(long)]
        initial_holder: PeerId,
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Approve the exact save-backed genesis only after verifying every blob.
    Approve {
        #[arg(long)]
        draft: PathBuf,
        #[arg(long)]
        expected_world: ExclusiveTurnWorldIdV1,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        confirm: bool,
    },
    /// Initialize a fresh save-backed reference journal with a complete bundle.
    Init {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Replace the held save-backed reference checkpoint with verified data.
    Checkpoint {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Materialize the exact held checkpoint into a new offline directory.
    RestoreHeld {
        #[command(flatten)]
        review: Reviewed,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        destination: PathBuf,
    },
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
    /// Derive candidate review pins from a complete bundle; not an independent authenticity check.
    Inspect {
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Review a signed save offer against an exact historical base and complete bundle, without staging or granting a turn.
    InspectOffer {
        #[command(flatten)]
        pin: super::Pin,
        #[arg(long)]
        history: PathBuf,
        #[arg(long)]
        offer: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Derive a candidate compatibility pin from three exact core JARs; does not verify a runnable installation.
    RuntimePin {
        #[command(flatten)]
        artifacts: CoreArtifacts,
    },
    /// Check exact pinned offline save bytes, core JARs and level.dat version; no launch permission.
    VerifyRuntime {
        #[arg(long)]
        bundle: PathBuf,
        #[command(flatten)]
        digests: Digests,
        #[command(flatten)]
        artifacts: CoreArtifacts,
    },
    /// Check the exact held reference grant/checkpoint against the core JAR pin and level.dat; no launch permission.
    VerifyHeldRuntime {
        #[command(flatten)]
        pin: super::Pin,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[command(flatten)]
        artifacts: CoreArtifacts,
    },
    /// Derive a candidate pin from an exact offline Linux runtime package.
    RuntimePackagePin {
        #[arg(long)]
        package: PathBuf,
    },
    /// Verify exact offline save bytes, their version, and the pinned runtime package inventory.
    VerifyRuntimePackage {
        #[arg(long)]
        bundle: PathBuf,
        #[command(flatten)]
        digests: Digests,
        #[arg(long)]
        package: PathBuf,
    },
    /// Verify the exact local held reference grant and retained save against the runtime package.
    VerifyHeldRuntimePackage {
        #[command(flatten)]
        pin: super::Pin,
        #[arg(long)]
        expected_checkpoint: Hash32,
        #[arg(long)]
        package: PathBuf,
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
        bundle: Option<PathBuf>,
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
        destination: Option<PathBuf>,
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
pub(super) use linux::verify_held_package;
#[cfg(target_os = "linux")]
pub(super) use linux::{retain_bundle, verify_offer_bundle, verify_retained};

#[cfg(not(target_os = "linux"))]
pub(super) fn verify_retained(
    _store: &Store,
    _checkpoint: ExclusiveTurnCheckpointV1,
    _compatibility: Hash32,
    _bytes: &[u8],
) -> Result<()> {
    anyhow::bail!("save-backed reference storage is Linux only")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn verify_offer_bundle(_store: &Store, _offer: &ExclusiveTurnSignedOfferV1) -> Result<()> {
    anyhow::bail!("save-backed reference storage is Linux only")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn retain_bundle(
    _store: &Store,
    _source: &std::path::Path,
    _checkpoint: ExclusiveTurnCheckpointV1,
    _compatibility: Hash32,
    _bytes: &[u8],
) -> Result<()> {
    anyhow::bail!("save-backed reference storage is Linux only")
}

#[cfg(target_os = "linux")]
mod linux {
    use super::super::{
        check_dir, create_dir, draft_genesis, initialize_reference, open_regular, opened, pin_grant, print_document,
        private_file, read_document, replace_checkpoint, report as journal_report, sign_approval, sync_dir, Phase,
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
    use swarm_cli::exclusive_turn_compatibility::{
        inspect_core_runtime, verify_level_version, CoreRuntimePin, MAX_CORE_JAR_BYTES, MAX_LEVEL_COMPRESSED_BYTES,
    };
    use swarm_cli::exclusive_turn_runtime_package::{inspect_runtime_package, RuntimePackagePin};
    use swarm_consensus::exclusive_turn::{verify_exclusive_turn_history, verify_exclusive_turn_offer};
    use swarm_protocol::{
        exclusive_turn_reference_commitment, ExclusiveTurnGenesisV1, ExclusiveTurnHistoryV1,
        EXCLUSIVE_TURN_SAVE_FORMAT_V2, MAX_EXCLUSIVE_TURN_REFERENCE_BYTES,
    };

    const MAX_ENTRIES: usize = 4096;
    const MAX_FILE: u64 = 512 * 1024 * 1024;
    const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
    const MAX_RETAINED_BUNDLES: usize = 16;
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

    fn retained_path(store: &Store, checkpoint: ExclusiveTurnCheckpointV1) -> Result<PathBuf> {
        let root = store.dir.join("save-bundles");
        check_dir(&root)?;
        Ok(root.join(checkpoint.reference_commitment.to_string()))
    }

    fn check_retention_capacity(destination: &Path) -> Result<()> {
        let root = destination.parent().context("missing managed save bundle root")?;
        check_dir(root)?;
        let count = fs::read_dir(root)?.take(MAX_RETAINED_BUNDLES).collect::<std::io::Result<Vec<_>>>()?.len();
        ensure!(count < MAX_RETAINED_BUNDLES, "save-backed retained bundle limit reached");
        Ok(())
    }

    fn verify_exact(
        bundle: &Path,
        checkpoint: ExclusiveTurnCheckpointV1,
        compatibility: Hash32,
        bytes: &[u8],
    ) -> Result<Manifest> {
        let manifest = load_bundle(bundle)?;
        ensure!(
            manifest.checkpoint()? == checkpoint
                && manifest.compatibility == compatibility
                && manifest.encode()? == bytes,
            "managed save bundle differs from exact checkpoint and compatibility"
        );
        verify_bundle(bundle, &manifest)?;
        Ok(manifest)
    }

    pub(in super::super) fn verify_retained(
        store: &Store,
        checkpoint: ExclusiveTurnCheckpointV1,
        compatibility: Hash32,
        bytes: &[u8],
    ) -> Result<()> {
        let path = retained_path(store, checkpoint)?;
        verify_exact(&path, checkpoint, compatibility, bytes).context("complete retained save bundle is required")?;
        // Recover a prior successful rename whose parent fsync or output
        // failed before acknowledging the durable bundle.
        sync_dir(path.parent().context("missing managed save parent")?)
    }

    pub(in super::super) fn verify_offer_bundle(store: &Store, offer: &ExclusiveTurnSignedOfferV1) -> Result<()> {
        let path = retained_path(store, offer.offer.checkpoint)?;
        let manifest = load_bundle(&path).context("complete retained save bundle is required")?;
        manifest.check_offer(offer)?;
        verify_retained(store, offer.offer.checkpoint, offer.offer.compatibility, &manifest.encode()?)
    }

    pub(in super::super) fn retain_bundle(
        store: &Store,
        source: &Path,
        checkpoint: ExclusiveTurnCheckpointV1,
        compatibility: Hash32,
        bytes: &[u8],
    ) -> Result<()> {
        let manifest = verify_exact(source, checkpoint, compatibility, bytes)?;
        let destination = retained_path(store, checkpoint)?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => return verify_retained(store, checkpoint, compatibility, bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        check_retention_capacity(&destination)?;
        let publication = Publication::new(&destination, manifest.validate()?)?;
        create_dir(&publication.path().join("blobs"))?;
        for (hash, size) in manifest.blobs() {
            let input = private_file(&blob_path(source, hash))?;
            let mut output = create_file(&blob_path(publication.path(), hash))?;
            ensure!(copy_blob(&input, Some(&mut output), size)? == hash, "save blob changed during retention");
            output.sync_all()?;
        }
        store_manifest(publication.path(), &manifest)?;
        verify_bundle(publication.path(), &manifest)?;
        publication.finish()?;
        verify_retained(store, checkpoint, compatibility, bytes)
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

    fn report_value(manifest: &Manifest, path: &Path, classification: &str) -> Result<serde_json::Value> {
        let checkpoint = manifest.checkpoint()?;
        Ok(serde_json::json!({
            "classification": classification, "path": path, "manifest_hash": checkpoint.manifest_hash.to_string(),
            "state_root": checkpoint.state_root.to_string(), "reference_commitment": checkpoint.reference_commitment.to_string(),
            "compatibility": manifest.compatibility.to_string(), "entries": manifest.entries.len(),
            "unique_blobs": manifest.blobs().len(), "total_bytes": manifest.validate()?,
            "canonical_authority": false, "write_authority": false, "runtime_activation": false,
            "player_admission": false, "handoff_receipt_issued": false, "minecraft_gameplay_verified": false,
        }))
    }

    fn report(manifest: &Manifest, path: &Path, classification: &str) -> Result<()> {
        print_document(&report_value(manifest, path, classification)?)
    }

    fn core_pin(artifacts: &CoreArtifacts) -> Result<CoreRuntimePin> {
        let read = |path: &Path| -> Result<Vec<u8>> {
            safe_directory(path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?;
            let file = source_file(path)?;
            super::super::read_bounded(file, MAX_CORE_JAR_BYTES)
        };
        inspect_core_runtime(
            &read(&artifacts.minecraft_jar)?,
            &read(&artifacts.fabric_loader_jar)?,
            &read(&artifacts.bridge_jar)?,
        )
    }

    fn runtime_report(pin: &CoreRuntimePin) -> Result<serde_json::Value> {
        Ok(serde_json::json!({
            "compatibility": pin.fingerprint()?.to_string(),
            "minecraft_version": pin.minecraft_version,
            "fabric_loader_version": pin.fabric_loader_version,
            "bridge_version": pin.bridge_version,
            "declared_java_major": pin.declared_java_major,
            "expected_data_version": pin.data_version,
            "minecraft_jar_hash": pin.minecraft_jar.to_string(),
            "fabric_loader_jar_hash": pin.fabric_loader_jar.to_string(),
            "bridge_jar_hash": pin.bridge_jar.to_string(),
            "core_jar_metadata_verified": true,
            "signed_compatibility_verified": false,
            "independent_checkpoint_pins_verified": false,
            "bundle_contents_verified": false,
            "level_data_version_verified": false,
            "local_held_reference_verified": false,
            "current_grant_verified": false,
            "installed_runtime_verified": false,
            "java_runtime_verified": false,
            "complete_save_semantics_verified": false,
            "save_barrier_verified": false,
            "resource_enforcement_verified": false,
            "canonical_authority": false, "write_authority": false,
            "runtime_activation": false, "player_admission": false,
            "minecraft_gameplay_verified": false, "handoff_receipt_issued": false,
        }))
    }

    fn verify_runtime_bundle(
        bundle: &Path,
        manifest: &Manifest,
        artifacts: &CoreArtifacts,
    ) -> Result<serde_json::Value> {
        let pin = core_pin(artifacts)?;
        ensure!(
            manifest.compatibility == pin.fingerprint()?,
            "core runtime artifact pin differs from save compatibility"
        );
        verify_level_bundle(bundle, manifest, pin.data_version, runtime_report(&pin)?)
    }

    fn package_report(package: &RuntimePackagePin) -> Result<serde_json::Value> {
        let mut report = runtime_report(&package.core)?;
        report["compatibility"] = package.fingerprint.to_string().into();
        report["core_compatibility"] = package.core.fingerprint()?.to_string().into();
        report["package_tree_hash"] = package.tree_hash.to_string().into();
        report["package_files"] = package.files.into();
        report["package_entries"] = package.entries.into();
        report["package_bytes"] = package.bytes.into();
        report["minecraft_library_count"] = package.minecraft_libraries.into();
        report["fabric_library_count"] = package.fabric_libraries.into();
        report["java_release_version"] = package.java_version.clone().into();
        report["embedded_fabric_api_version"] = package.embedded_fabric_api_version.clone().into();
        report["package_inventory_verified"] = true.into();
        report["declared_dependency_files_verified"] = true.into();
        report["artifact_publisher_verified"] = false.into();
        report["java_executed"] = false.into();
        report["package_platform"] = format!("linux-{}", std::env::consts::ARCH).into();
        Ok(report)
    }

    fn verify_package_bundle(bundle: &Path, manifest: &Manifest, path: &Path) -> Result<serde_json::Value> {
        let package = inspect_runtime_package(path)?;
        ensure!(manifest.compatibility == package.fingerprint, "runtime package pin differs from save compatibility");
        verify_level_bundle(bundle, manifest, package.core.data_version, package_report(&package)?)
    }

    pub(in super::super) fn verify_held_package(
        store: &Store,
        journal: &super::super::Journal,
        head: &swarm_consensus::exclusive_turn::VerifiedExclusiveTurnHead,
        package: &Path,
    ) -> Result<serde_json::Value> {
        ensure!(
            head.save_backed() && journal.phase == Phase::Held && head.holder() == journal.identity,
            "runtime package preflight requires exact held save-backed reference turn"
        );
        let bundle = retained_path(store, journal.checkpoint)?;
        let manifest =
            verify_exact(&bundle, journal.checkpoint, head.compatibility(), &store.state(journal.checkpoint)?)?;
        verify_package_bundle(&bundle, &manifest, package)
    }

    fn verify_level_bundle(
        bundle: &Path,
        manifest: &Manifest,
        expected_version: i32,
        mut result: serde_json::Value,
    ) -> Result<serde_json::Value> {
        verify_bundle(bundle, manifest)?;
        let entry = manifest.entries.iter().find(|entry| entry.path == "level.dat").context("missing level.dat")?;
        let Kind::File { size, hash } = entry.kind else { bail!("level.dat is not a file") };
        ensure!(size <= MAX_LEVEL_COMPRESSED_BYTES as u64, "level.dat compressed size out of bounds");
        let bytes = super::super::read_bounded(
            private_file(&bundle.join("blobs").join(hash.to_string()))?,
            MAX_LEVEL_COMPRESSED_BYTES,
        )?;
        ensure!(
            bytes.len() as u64 == size && Hash32::from_domain_bytes(BLOB_DOMAIN, &bytes) == hash,
            "level.dat bytes changed after bundle verification"
        );
        let version = verify_level_version(&bytes, expected_version)?;
        let checkpoint = manifest.checkpoint()?;
        result["manifest_hash"] = checkpoint.manifest_hash.to_string().into();
        result["state_root"] = checkpoint.state_root.to_string().into();
        result["reference_commitment"] = checkpoint.reference_commitment.to_string().into();
        result["bundle_contents_verified"] = true.into();
        result["level_data_version"] = version.into();
        result["level_data_version_verified"] = true.into();
        Ok(result)
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

    fn receive_blob(channel: &mut Channel, bundle: Option<&Path>, hash: Hash32, size: u64) -> Result<()> {
        let mut file = bundle.map(|p| create_file(&blob_path(p, hash))).transpose()?;
        let mut left = size;
        let mut digest = hash_start();
        while left > 0 {
            let chunk = channel.recv_chunk(left.min(CHUNK as u64) as usize)?;
            digest.update(&chunk);
            if let Some(file) = &mut file {
                file.write_all(&chunk)?;
            }
            left -= chunk.len() as u64;
        }
        ensure!(hash_finish(digest) == hash, "received save blob digest differs");
        if let Some(file) = file {
            file.sync_all()?;
        }
        Ok(())
    }

    fn serve(
        paths: &DataPaths,
        review: &Reviewed,
        expected: Hash32,
        bundle: Option<&Path>,
        bind: SocketAddr,
    ) -> Result<()> {
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
        let managed;
        let bundle = if head.save_backed() {
            ensure!(bundle.is_none(), "save-backed source uses only its managed retained bundle");
            managed = retained_path(&store, journal.checkpoint)?;
            managed.as_path()
        } else {
            bundle.context("opaque reference source requires --bundle")?
        };
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
        destination: Option<&Path>,
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
        // A save-backed recipient can publish only into its own
        // content-addressed store. An exact already-retained bundle
        // may be reverified on a lost completion marker, never replaced.
        let managed;
        let destination = if head.save_backed() {
            ensure!(destination.is_none(), "save-backed fetch uses only its managed retained bundle");
            managed = retained_path(&store, offer.offer.checkpoint)?;
            managed.as_path()
        } else {
            destination.context("opaque reference fetch requires --destination")?
        };
        let retained = match fs::symlink_metadata(destination) {
            Ok(_) if head.save_backed() => {
                verify_offer_bundle(&store, &offer)?;
                true
            }
            Ok(_) => bail!("save destination already exists; no replacement permitted"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if head.save_backed() {
                    check_retention_capacity(destination)?;
                }
                false
            }
            Err(e) => return Err(e.into()),
        };
        let deadline = Instant::now() + SESSION;
        let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
        let mut channel = authenticate(stream, &prologue(&offer)?, AUTH_DOMAIN, &store.identity, key, true, deadline)?;
        let bytes = channel.receive(MAX_EXCLUSIVE_TURN_REFERENCE_BYTES)?;
        let manifest = Manifest::decode(&bytes)?;
        manifest.check_offer(&offer)?;
        let publication = if retained {
            ensure!(load_bundle(destination)? == manifest, "retained save manifest differs");
            None
        } else {
            let publication = Publication::new(destination, manifest.validate()?)?;
            create_dir(&publication.path().join("blobs"))?;
            Some(publication)
        };
        for (hash, size) in manifest.blobs() {
            receive_blob(&mut channel, publication.as_ref().map(Publication::path), hash, size)?;
        }
        if let Some(publication) = publication {
            store_manifest(publication.path(), &manifest)?;
            verify_bundle(publication.path(), &manifest)?;
            remaining(deadline)?;
            publication.finish()?;
        }
        if head.save_backed() {
            verify_retained(&store, offer.offer.checkpoint, head.compatibility(), &bytes)?;
        }
        remaining(deadline)?;
        channel.send_chunk(&manifest.checkpoint()?.manifest_hash.0)?;
        report(&manifest, destination, "exclusive_turn_save_data_received")
    }

    pub(super) fn run(paths: &DataPaths, command: SaveCommand) -> Result<()> {
        match command {
            SaveCommand::Draft { device_keys, initial_holder, bundle } => {
                let manifest = load_bundle(&bundle)?;
                verify_bundle(&bundle, &manifest)?;
                print_document(&draft_genesis(
                    EXCLUSIVE_TURN_SAVE_FORMAT_V2,
                    device_keys,
                    initial_holder,
                    manifest.compatibility,
                    manifest.checkpoint()?,
                )?)
            }
            SaveCommand::Approve { draft, expected_world, bundle, confirm } => {
                ensure!(confirm, "explicit --confirm required for reviewed save genesis approval");
                let genesis: ExclusiveTurnGenesisV1 = read_document(&draft)?;
                ensure!(
                    genesis.save_backed() && genesis.world_id()? == expected_world,
                    "save-backed genesis differs from expected pin"
                );
                let manifest = load_bundle(&bundle)?;
                let bytes = manifest.encode()?;
                verify_exact(&bundle, genesis.initial_checkpoint, genesis.compatibility, &bytes)?;
                print_document(&sign_approval(paths, &genesis, expected_world, &bytes)?)
            }
            SaveCommand::Init { review, history, bundle } => {
                review.confirm()?;
                let history: ExclusiveTurnHistoryV1 = read_document(&history)?;
                let genesis = &history.creation.genesis;
                ensure!(genesis.save_backed(), "save init requires a save-backed genesis");
                let bytes = load_bundle(&bundle)?.encode()?;
                verify_exact(&bundle, genesis.initial_checkpoint, genesis.compatibility, &bytes)?;
                initialize_reference(paths, &review, history, &bytes, Some(&bundle))
            }
            SaveCommand::Checkpoint { review, expected_checkpoint, bundle } => {
                review.confirm()?;
                let (store, mut journal, head) = opened(paths, &review.pin)?;
                ensure!(
                    head.save_backed() && journal.phase == Phase::Held,
                    "save checkpoint requires held save-backed reference turn"
                );
                ensure!(
                    journal.checkpoint.reference_commitment == expected_checkpoint,
                    "local checkpoint differs from expected pin"
                );
                let manifest = load_bundle(&bundle)?;
                let checkpoint = manifest.checkpoint()?;
                let bytes = manifest.encode()?;
                retain_bundle(&store, &bundle, checkpoint, head.compatibility(), &bytes)?;
                replace_checkpoint(&store, &mut journal, &head, expected_checkpoint, checkpoint, &bytes)?;
                print_document(&journal_report(&head, Some(&journal)))
            }
            SaveCommand::RestoreHeld { review, expected_checkpoint, destination } => {
                review.confirm()?;
                let (store, journal, head) = opened(paths, &review.pin)?;
                ensure!(
                    head.save_backed() && journal.phase == Phase::Held && head.holder() == journal.identity,
                    "restore-held requires exact held save-backed reference turn"
                );
                ensure!(
                    journal.checkpoint.reference_commitment == expected_checkpoint,
                    "local checkpoint differs from expected pin"
                );
                let bundle = retained_path(&store, journal.checkpoint)?;
                let manifest = load_bundle(&bundle)?;
                verify_exact(&bundle, journal.checkpoint, head.compatibility(), &store.state(journal.checkpoint)?)?;
                // Never materialize inside the managed authority profile.
                let parent = safe_directory(
                    destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")),
                )?;
                ensure!(
                    !parent.starts_with(safe_directory(&paths.root)?),
                    "offline restoration cannot target authority profile"
                );
                let destination = restore(&bundle, &destination, &manifest)?;
                report(&manifest, &destination, "exclusive_turn_held_save_data_restored")
            }
            SaveCommand::Capture { source, destination, compatibility, confirm } => {
                ensure!(confirm, "explicit --confirm required for offline save capture");
                let (manifest, destination) = capture(&source, &destination, compatibility)?;
                report(&manifest, &destination, "exclusive_turn_save_data_captured")
            }
            SaveCommand::Inspect { bundle } => {
                let manifest = load_bundle(&bundle)?;
                verify_bundle(&bundle, &manifest)?;
                let mut result = report_value(&manifest, &bundle, "exclusive_turn_save_candidate_review")?;
                result["bundle_contents_verified"] = true.into();
                result["base_history_pins_verified"] = false.into();
                result["bundle_matches_signed_offer"] = false.into();
                result["signed_offer_verified"] = false.into();
                result["current_grant_verified"] = false.into();
                result["current_offer_verified"] = false.into();
                print_document(&result)
            }
            SaveCommand::InspectOffer { pin, history, offer, bundle } => {
                let history = read_document(&history)?;
                let head = verify_exclusive_turn_history(pin.expected_world, &history)?;
                pin_grant(&head, pin.expected_grant)?;
                ensure!(head.save_backed(), "offer review requires a save-backed reference history");
                let offer: ExclusiveTurnSignedOfferV1 = read_document(&offer)?;
                verify_exclusive_turn_offer(&head, &offer)?;
                let manifest = load_bundle(&bundle)?;
                manifest.check_offer(&offer)?;
                verify_bundle(&bundle, &manifest)?;
                let mut result = report_value(&manifest, &bundle, "exclusive_turn_signed_save_offer_review")?;
                result["bundle_contents_verified"] = true.into();
                result["base_history_pins_verified"] = true.into();
                result["bundle_matches_signed_offer"] = true.into();
                result["signed_offer_verified"] = true.into();
                // An offline history cannot prove that this offer was not canceled or superseded.
                result["current_grant_verified"] = false.into();
                result["current_offer_verified"] = false.into();
                result["world_id"] = head.world().to_string().into();
                result["base_grant"] = head.grant().to_string().into();
                result["offer_hash"] = offer.offer_hash()?.to_string().into();
                result["source"] = offer.offer.source.to_string().into();
                result["recipient"] = offer.offer.recipient.to_string().into();
                result["base_epoch"] = offer.offer.base_epoch.into();
                result["base_fence"] = offer.offer.base_fence.into();
                result["next_epoch"] = offer.offer.next_epoch.into();
                result["next_fence"] = offer.offer.next_fence.into();
                result["attempt"] = offer.offer.attempt.into();
                print_document(&result)
            }
            SaveCommand::RuntimePin { artifacts } => {
                let mut result = runtime_report(&core_pin(&artifacts)?)?;
                result["classification"] = "exclusive_turn_candidate_core_runtime_pin".into();
                print_document(&result)
            }
            SaveCommand::VerifyRuntime { bundle, digests, artifacts } => {
                let manifest = load_bundle(&bundle)?;
                manifest.check_pins(&digests)?;
                let mut result = verify_runtime_bundle(&bundle, &manifest, &artifacts)?;
                result["classification"] = "exclusive_turn_offline_core_runtime_preflight".into();
                result["independent_checkpoint_pins_verified"] = true.into();
                print_document(&result)
            }
            SaveCommand::VerifyHeldRuntime { pin, expected_checkpoint, artifacts } => {
                let (store, journal, head) = opened(paths, &pin)?;
                ensure!(
                    head.save_backed() && journal.phase == Phase::Held && head.holder() == journal.identity,
                    "runtime preflight requires exact held save-backed reference turn"
                );
                ensure!(
                    journal.checkpoint.reference_commitment == expected_checkpoint,
                    "local checkpoint differs from expected pin"
                );
                let bundle = retained_path(&store, journal.checkpoint)?;
                let manifest = load_bundle(&bundle)?;
                verify_exact(&bundle, journal.checkpoint, head.compatibility(), &store.state(journal.checkpoint)?)?;
                let mut result = verify_runtime_bundle(&bundle, &manifest, &artifacts)?;
                result["classification"] = "exclusive_turn_held_core_runtime_preflight".into();
                result["signed_compatibility_verified"] = true.into();
                result["independent_checkpoint_pins_verified"] = true.into();
                result["local_held_reference_verified"] = true.into();
                result["world_id"] = head.world().to_string().into();
                result["grant"] = head.grant().to_string().into();
                result["epoch"] = head.epoch().into();
                result["fence"] = head.fence().into();
                print_document(&result)
            }
            SaveCommand::RuntimePackagePin { package } => {
                let mut result = package_report(&inspect_runtime_package(&package)?)?;
                result["classification"] = "exclusive_turn_candidate_runtime_package_pin".into();
                print_document(&result)
            }
            SaveCommand::VerifyRuntimePackage { bundle, digests, package } => {
                let manifest = load_bundle(&bundle)?;
                manifest.check_pins(&digests)?;
                let mut result = verify_package_bundle(&bundle, &manifest, &package)?;
                result["classification"] = "exclusive_turn_offline_runtime_package_preflight".into();
                result["independent_checkpoint_pins_verified"] = true.into();
                print_document(&result)
            }
            SaveCommand::VerifyHeldRuntimePackage { pin, expected_checkpoint, package } => {
                let (store, journal, head) = opened(paths, &pin)?;
                ensure!(
                    head.save_backed() && journal.phase == Phase::Held && head.holder() == journal.identity,
                    "runtime package preflight requires exact held save-backed reference turn"
                );
                ensure!(
                    journal.checkpoint.reference_commitment == expected_checkpoint,
                    "local checkpoint differs from expected pin"
                );
                let mut result = verify_held_package(&store, &journal, &head, &package)?;
                result["classification"] = "exclusive_turn_held_runtime_package_preflight".into();
                result["signed_compatibility_verified"] = true.into();
                result["independent_checkpoint_pins_verified"] = true.into();
                result["local_held_reference_verified"] = true.into();
                result["world_id"] = head.world().to_string().into();
                result["grant"] = head.grant().to_string().into();
                result["epoch"] = head.epoch().into();
                result["fence"] = head.fence().into();
                print_document(&result)
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
                serve(paths, &review, expected_offer, bundle.as_deref(), bind)
            }
            SaveCommand::Fetch { review, expected_offer, offer, destination, peer } => {
                fetch(paths, &review, expected_offer, &offer, destination.as_deref(), &peer)
            }
        }
    }
}
