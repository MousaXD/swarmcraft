//! Explicit, opt-in signed update delivery. Never executes installers, changes
//! live authority, accepts EULA, or rewrites canonical worlds.
use anyhow::{bail, ensure, Context, Result};
use clap::{Subcommand, ValueEnum};
use ed25519_dalek::{Signature, VerifyingKey};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swarm_core::{create_private_dir_all, DataPaths};
use swarm_protocol::{PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION};
use swarm_storage::profile::PROFILE_READER_GENERATION;

const MANIFEST: &str = "update-manifest.json";
const SIGNATURE: &str = "update-manifest.sig";
const MANIFEST_DOMAIN: &[u8] = b"SwarmCraft update manifest v1\0";
const PACKAGE_DOMAIN: &str = "SwarmCraft update package v1\n";
const MAX_METADATA: u64 = 64 * 1024;
const MAX_PACKAGE: u64 = 1024 * 1024 * 1024;
const STAGING_FREE_RESERVE: u64 = 256 * 1024 * 1024;
const MAX_VALIDITY: u64 = 30 * 24 * 60 * 60;
const REPOSITORY: &str = "https://github.com/MousaXD/swarmcraft/releases/download";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Off,
    Stable,
    Preview,
}

impl Channel {
    fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Stable => "stable",
            Self::Preview => "preview",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LinuxFormat {
    Deb,
    Appimage,
}

#[derive(Debug, Subcommand)]
pub enum UpdateCommand {
    /// Read local update preference and trust pins; no network or profile activation.
    Status,
    /// Explicitly opt in to one channel or opt out, retaining replay protection.
    Configure {
        #[arg(long, value_enum)]
        channel: Channel,
        /// Out-of-band authenticated Ed25519 public key (64 lowercase hex characters).
        #[arg(long, requires = "expected_fingerprint")]
        public_key: Option<String>,
        /// Independently authenticated SHA-256 of the 32 raw public key bytes.
        #[arg(long, requires = "public_key")]
        expected_fingerprint: Option<String>,
    },
    /// Verify a signed manifest and its exact compatible package offer.
    Check {
        #[arg(long, conflicts_with = "online", required_unless_present = "online")]
        feed_dir: Option<PathBuf>,
        /// Explicitly fetch only the configured official channel feed.
        #[arg(long)]
        online: bool,
        /// Explicit package format on Linux; defaults to Debian.
        #[arg(long, value_enum)]
        linux_format: Option<LinuxFormat>,
    },
    /// Download/copy, authenticate and stage the package; never run it.
    Stage {
        #[arg(long, conflicts_with = "online", required_unless_present = "online")]
        feed_dir: Option<PathBuf>,
        #[arg(long)]
        online: bool,
        #[arg(long, value_enum)]
        linux_format: Option<LinuxFormat>,
    },
    /// Reverify a staged package immediately before separate, manual installation.
    VerifyStaged {
        #[arg(long, value_enum)]
        linux_format: Option<LinuxFormat>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HighWater {
    sequence: u64,
    manifest_sha256: String,
    version: String,
    commit: String,
    checked_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustPin {
    public_key: String,
    fingerprint: String,
    high_water: Option<HighWater>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u16,
    channel: Channel,
    stable: Option<TrustPin>,
    preview: Option<TrustPin>,
}

impl Default for State {
    fn default() -> Self {
        Self { schema: 1, channel: Channel::Off, stable: None, preview: None }
    }
}

impl State {
    fn pin(&self, channel: Channel) -> Option<&TrustPin> {
        match channel {
            Channel::Stable => self.stable.as_ref(),
            Channel::Preview => self.preview.as_ref(),
            Channel::Off => None,
        }
    }

    fn pin_mut(&mut self, channel: Channel) -> Result<&mut Option<TrustPin>> {
        match channel {
            Channel::Stable => Ok(&mut self.stable),
            Channel::Preview => Ok(&mut self.preview),
            Channel::Off => bail!("off has no update trust pin"),
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "unsupported update preferences schema");
        for pin in [&self.stable, &self.preview].into_iter().flatten() {
            validate_pin(&pin.public_key, &pin.fingerprint)?;
            if let Some(water) = &pin.high_water {
                ensure!(water.sequence > 0 && water.checked_unix > 0, "invalid update replay record");
                lower_hex(&water.manifest_sha256, 64)?;
                lower_hex(&water.commit, 40)?;
                version(&water.version)?;
            }
        }
        if let (Some(stable), Some(preview)) = (&self.stable, &self.preview) {
            ensure!(stable.public_key != preview.public_key, "stable and preview must use distinct trust keys");
        }
        ensure!(self.channel == Channel::Off || self.pin(self.channel).is_some(), "enabled update channel has no pin");
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u16,
    channel: Channel,
    sequence: u64,
    issued_unix: u64,
    expires_unix: u64,
    version: String,
    minimum_client_version: String,
    commit: String,
    release_tag: String,
    protocol_version: u16,
    storage_schema_version: u16,
    reader_generation: u16,
    packages: Vec<Package>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    target: String,
    file: String,
    bytes: u64,
    sha256: String,
    signature: String,
}

struct Offer {
    manifest: Manifest,
    package: Package,
    raw: Vec<u8>,
    signature: Vec<u8>,
    digest: String,
}

fn lower_hex(value: &str, len: usize) -> Result<()> {
    ensure!(
        value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid lowercase hex field"
    );
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn validate_pin(public_key: &str, fingerprint: &str) -> Result<VerifyingKey> {
    lower_hex(public_key, 64)?;
    lower_hex(fingerprint, 64)?;
    let raw: [u8; 32] = hex::decode(public_key)?.try_into().map_err(|_| anyhow::anyhow!("invalid key length"))?;
    ensure!(sha256(&raw) == fingerprint, "public key differs from independently authenticated fingerprint");
    let key = VerifyingKey::from_bytes(&raw).context("invalid update public key")?;
    ensure!(!key.is_weak(), "weak update public key");
    Ok(key)
}

fn verify(key: &VerifyingKey, message: &[u8], hex_signature: &str) -> Result<()> {
    lower_hex(hex_signature, 128)?;
    let signature = Signature::from_slice(&hex::decode(hex_signature)?)?;
    key.verify_strict(message, &signature).context("update signature verification failed")
}

fn version(value: &str) -> Result<[u32; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    ensure!(parts.len() == 3 && value.len() <= 32, "expected exact X.Y.Z application version");
    let mut result = [0; 3];
    for (part, number) in parts.into_iter().zip(result.iter_mut()) {
        ensure!(
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && (part.len() == 1 || !part.starts_with('0')),
            "noncanonical application version"
        );
        *number = part.parse().context("invalid application version component")?;
    }
    Ok(result)
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH).context("system clock predates Unix epoch")?.as_secs())
}

fn extension(target: &str) -> Result<&'static str> {
    match target {
        "linux-x86_64-deb" => Ok(".deb"),
        "linux-x86_64-appimage" => Ok(".AppImage"),
        "windows-x86_64-nsis" => Ok(".exe"),
        "macos-aarch64-dmg" | "macos-x86_64-dmg" => Ok(".dmg"),
        _ => bail!("unsupported update package target"),
    }
}

fn native_target(format: Option<LinuxFormat>) -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH, format) {
        ("linux", "x86_64", None | Some(LinuxFormat::Deb)) => Ok("linux-x86_64-deb"),
        ("linux", "x86_64", Some(LinuxFormat::Appimage)) => Ok("linux-x86_64-appimage"),
        ("windows", "x86_64", None) => Ok("windows-x86_64-nsis"),
        ("macos", "aarch64", None) => Ok("macos-aarch64-dmg"),
        ("macos", "x86_64", None) => Ok("macos-x86_64-dmg"),
        _ => bail!("this installation platform/format has no supported update target"),
    }
}

fn package_message(manifest: &Manifest, package: &Package) -> Vec<u8> {
    format!(
        "{PACKAGE_DOMAIN}{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        manifest.channel.label(),
        manifest.version,
        manifest.commit,
        manifest.release_tag,
        package.target,
        package.file,
        package.bytes,
        package.sha256
    )
    .into_bytes()
}

fn validate_package(manifest: &Manifest, package: &Package, key: &VerifyingKey) -> Result<()> {
    let ext = extension(&package.target)?;
    let lower = package.file.to_ascii_lowercase();
    ensure!(
        package.file.len() <= 180
            && !package.file.is_empty()
            && package.file.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            && lower.starts_with("swarmcraft")
            && package.file.ends_with(ext)
            && package.file.contains(&format!("_{}_", manifest.version)),
        "unsafe or mismatched update package filename"
    );
    let arch = match package.target.as_str() {
        "linux-x86_64-deb" => lower.contains("amd64") || lower.contains("x86_64"),
        "linux-x86_64-appimage" => lower.contains("amd64") || lower.contains("x86_64"),
        "windows-x86_64-nsis" => lower.contains("x64") && lower.ends_with("-setup.exe"),
        "macos-aarch64-dmg" => lower.contains("arm64") || lower.contains("aarch64"),
        "macos-x86_64-dmg" => lower.contains("x86_64") || lower.contains("x64"),
        _ => false,
    };
    ensure!(arch, "update package filename does not match architecture");
    ensure!((1..=MAX_PACKAGE).contains(&package.bytes), "invalid update package size");
    lower_hex(&package.sha256, 64)?;
    verify(key, &package_message(manifest, package), &package.signature)
        .context("invalid detached package signature")?;
    Ok(())
}

fn validate_time(manifest: &Manifest, time: u64) -> Result<()> {
    ensure!(
        manifest.issued_unix <= time
            && time < manifest.expires_unix
            && manifest.expires_unix.saturating_sub(manifest.issued_unix) <= MAX_VALIDITY,
        "update manifest is expired, future-dated or exceeds its maximum lifetime"
    );
    Ok(())
}

fn validate_offer(
    state: &State,
    raw: Vec<u8>,
    signature: Vec<u8>,
    target: &str,
    installed: &str,
    time: u64,
) -> Result<Offer> {
    ensure!(state.channel != Channel::Off, "updates are disabled");
    let pin = state.pin(state.channel).context("no authenticated channel pin")?;
    let key = validate_pin(&pin.public_key, &pin.fingerprint)?;
    let mut message = MANIFEST_DOMAIN.to_vec();
    message.extend_from_slice(&raw);
    let signature_text = std::str::from_utf8(&signature)?.trim();
    verify(&key, &message, signature_text).context("untrusted update manifest")?;
    let manifest: Manifest = serde_json::from_slice(&raw).context("invalid signed update manifest")?;
    ensure!(
        manifest.schema == 1 && manifest.channel == state.channel && manifest.sequence > 0,
        "update schema or selected channel mismatch"
    );
    validate_time(&manifest, time)?;
    ensure!(
        manifest.protocol_version == PROTOCOL_VERSION
            && manifest.storage_schema_version == STORAGE_SCHEMA_VERSION
            && manifest.reader_generation == PROFILE_READER_GENERATION,
        "update requires incompatible protocol, storage schema or profile reader; use reviewed migration procedure"
    );
    let offered = version(&manifest.version)?;
    let running = version(installed)?;
    let minimum = version(&manifest.minimum_client_version)?;
    ensure!(
        offered > running && minimum <= running && minimum < offered,
        "update is not a strictly newer compatible application version"
    );
    lower_hex(&manifest.commit, 40)?;
    let expected_tag = match manifest.channel {
        Channel::Stable => format!("v{}", manifest.version),
        Channel::Preview => format!("main-snapshot-{}", manifest.commit),
        Channel::Off => bail!("disabled channel cannot publish an update"),
    };
    ensure!(manifest.release_tag == expected_tag, "update release tag/channel mismatch");
    let digest = sha256(&raw);
    if let Some(water) = &pin.high_water {
        ensure!(time >= water.checked_unix, "local clock moved backwards since last accepted update");
        ensure!(manifest.sequence >= water.sequence, "update manifest sequence rollback");
        if manifest.sequence == water.sequence {
            ensure!(digest == water.manifest_sha256, "update sequence reused for a different manifest");
        }
        let old = version(&water.version)?;
        ensure!(offered >= old, "update version rollback");
        if state.channel == Channel::Stable && offered == old {
            ensure!(manifest.commit == water.commit, "stable version changed source commit");
        }
    }
    ensure!((1..=5).contains(&manifest.packages.len()), "invalid update package inventory");
    let mut targets = std::collections::BTreeSet::new();
    let mut filenames = std::collections::BTreeSet::new();
    for package in &manifest.packages {
        ensure!(
            targets.insert(&package.target) && filenames.insert(package.file.to_ascii_lowercase()),
            "duplicate update package target or filename"
        );
        validate_package(&manifest, package, &key)?;
    }
    let package = manifest
        .packages
        .iter()
        .find(|package| package.target == target)
        .context("signed update has no package for this platform/format")?
        .clone();
    Ok(Offer { manifest, package, raw, signature, digest })
}

fn entry(path: &Path, directory: bool) -> Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        !meta.file_type().is_symlink() && if directory { meta.is_dir() } else { meta.is_file() },
        "unsafe update file or directory"
    );
    Ok(true)
}

fn local_control(path: &Path, directory: bool) -> Result<bool> {
    if !entry(path, directory)? {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::symlink_metadata(path)?;
        ensure!(
            meta.mode() & 0o022 == 0 && (directory || meta.nlink() == 1),
            "unsafe writable or hard-linked update trust state"
        );
        #[cfg(target_os = "linux")]
        ensure!(meta.uid() == crate::machine_hosting::linux::uid(), "update trust state has different owner");
    }
    Ok(true)
}

fn read_bounded(mut stream: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "update metadata exceeds size bound");
    Ok(bytes)
}

fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(entry(path, false)?, "missing update file");
    read_bounded(File::open(path)?, limit)
}

fn state_path(paths: &DataPaths) -> PathBuf {
    paths.root.join("updates/state.json")
}

fn load_state(paths: &DataPaths) -> Result<State> {
    if !local_control(&paths.root, true)?
        || !local_control(&paths.root.join("updates"), true)?
        || !local_control(&state_path(paths), false)?
    {
        return Ok(State::default());
    }
    let state: State = serde_json::from_slice(&read_file(&state_path(paths), MAX_METADATA)?)
        .context("invalid local update preferences; updates fail closed")?;
    state.validate()?;
    Ok(state)
}

fn lock(paths: &DataPaths) -> Result<File> {
    local_control(&paths.root, true)?;
    let directory = paths.root.join("updates");
    local_control(&directory, true)?;
    create_private_dir_all(&directory)?;
    local_control(&directory, true)?;
    let path = directory.join("state.lock");
    local_control(&path, false)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    local_control(&path, false)?;
    FileExt::try_lock_exclusive(&file).context("another update operation is in progress; retry after it exits")?;
    Ok(file)
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn save_state(paths: &DataPaths, state: &State) -> Result<()> {
    state.validate()?;
    let path = state_path(paths);
    local_control(&path, false)?;
    let parent = path.parent().context("update state parent missing")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&serde_json::to_vec_pretty(state)?)?;
    temp.as_file().sync_all()?;
    let temporary_path = temp.into_temp_path();
    crate::authority_permit::replace_permit_atomically(&temporary_path, &path)?;
    sync_directory(parent)
}

fn configure(
    paths: &DataPaths,
    channel: Channel,
    public_key: Option<String>,
    fingerprint: Option<String>,
) -> Result<State> {
    let _guard = lock(paths)?;
    let mut state = load_state(paths)?;
    if channel == Channel::Off {
        ensure!(public_key.is_none() && fingerprint.is_none(), "opt-out cannot modify pins");
    } else if let (Some(public_key), Some(fingerprint)) = (public_key, fingerprint) {
        validate_pin(&public_key, &fingerprint)?;
        let slot = state.pin_mut(channel)?;
        if let Some(existing) = slot {
            ensure!(
                existing.public_key == public_key && existing.fingerprint == fingerprint,
                "channel pin replacement is not supported; preserve replay history and obtain a reviewed key rotation"
            );
        } else {
            *slot = Some(TrustPin { public_key, fingerprint, high_water: None });
        }
    } else {
        ensure!(
            state.pin(channel).is_some(),
            "first opt-in needs an independently authenticated channel public key and fingerprint"
        );
    }
    state.channel = channel;
    save_state(paths, &state)?;
    Ok(state)
}

fn allowed_release_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && match url.host_str() {
            Some("github.com") => {
                url.path().starts_with("/MousaXD/swarmcraft/releases/download/") && url.query().is_none()
            }
            Some(
                "release-assets.githubusercontent.com"
                | "objects.githubusercontent.com"
                | "github-releases.githubusercontent.com",
            ) => true,
            _ => false,
        }
}

enum Feed {
    Local(PathBuf),
    Online(reqwest::blocking::Client),
}

impl Feed {
    fn new(directory: Option<PathBuf>, online: bool) -> Result<Self> {
        match (directory, online) {
            (Some(path), false) => {
                ensure!(entry(&path, true)?, "local update feed is not a real directory");
                Ok(Self::Local(path))
            }
            (None, true) => Ok(Self::Online(
                reqwest::blocking::Client::builder()
                    .https_only(true)
                    .no_proxy()
                    .connect_timeout(Duration::from_secs(15))
                    .timeout(Duration::from_secs(180))
                    .redirect(reqwest::redirect::Policy::custom(|attempt| {
                        if attempt.previous().len() < 4 && allowed_release_url(attempt.url()) {
                            attempt.follow()
                        } else {
                            attempt.error("untrusted or excessive update redirect")
                        }
                    }))
                    .build()?,
            )),
            _ => bail!("select exactly one explicit update source"),
        }
    }

    fn open(&self, tag: &str, name: &str) -> Result<Box<dyn Read>> {
        match self {
            Self::Local(path) => {
                let file = path.join(name);
                ensure!(entry(&file, false)?, "missing or unsafe local update asset");
                Ok(Box::new(File::open(file)?))
            }
            Self::Online(client) => {
                let url = reqwest::Url::parse(&format!("{REPOSITORY}/{tag}/{name}"))?;
                ensure!(allowed_release_url(&url), "untrusted update asset URL");
                // CDN redirects can carry temporary signed query strings.
                // Never retain or print their URLs in transport error reports.
                let response = client
                    .get(url)
                    .send()
                    .map_err(reqwest::Error::without_url)?
                    .error_for_status()
                    .map_err(reqwest::Error::without_url)?;
                ensure!(allowed_release_url(response.url()), "untrusted final update URL");
                Ok(Box::new(response))
            }
        }
    }

    fn offer(&self, state: &State, target: &str, installed: &str) -> Result<Offer> {
        let index = format!("swarmcraft-updates-{}", state.channel.label());
        let raw = read_bounded(self.open(&index, MANIFEST)?, MAX_METADATA)?;
        let signature = read_bounded(self.open(&index, SIGNATURE)?, 256)?;
        validate_offer(state, raw, signature, target, installed, now()?)
    }
}

fn advance(paths: &DataPaths, state: &mut State, offer: &Offer, time: u64) -> Result<()> {
    validate_time(&offer.manifest, time)?;
    let pin = state.pin_mut(state.channel)?.as_mut().context("no active update pin")?;
    ensure!(
        pin.high_water.as_ref().is_none_or(|water| time >= water.checked_unix),
        "local clock moved backwards since last accepted update"
    );
    pin.high_water = Some(HighWater {
        sequence: offer.manifest.sequence,
        manifest_sha256: offer.digest.clone(),
        version: offer.manifest.version.clone(),
        commit: offer.manifest.commit.clone(),
        checked_unix: time,
    });
    save_state(paths, state)
}

fn verified_copy(mut source: impl Read, mut output: impl Write, package: &Package) -> Result<()> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0; 128 * 1024];
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        bytes = bytes.checked_add(n as u64).context("update size overflow")?;
        ensure!(bytes <= package.bytes, "update package exceeds signed byte count");
        hash.update(&buffer[..n]);
        output.write_all(&buffer[..n])?;
    }
    ensure!(
        bytes == package.bytes && hex::encode(hash.finalize()) == package.sha256,
        "update package bytes do not match signed size and SHA-256"
    );
    Ok(())
}

fn stage_path(paths: &DataPaths, offer: &Offer) -> PathBuf {
    paths.root.join("updates/staged").join(format!(
        "{}-{}-{}-{}",
        offer.manifest.channel.label(),
        offer.manifest.sequence,
        offer.digest,
        offer.package.target
    ))
}

fn private_new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

/// Preserve the normal storage reserve even while staging a signed package.
/// This is best-effort physical-space admission, not a hard reservation.
struct ReserveWriter<'a> {
    file: &'a mut File,
    parent: &'a Path,
}

impl Write for ReserveWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if fs2::available_space(self.parent)?.saturating_sub(STAGING_FREE_RESERVE) < bytes.len() as u64 {
            return Err(io::Error::other("update staging would exhaust the local free-space reserve"));
        }
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn reverify_directory(directory: &Path, state: &State, target: &str, installed: &str, time: u64) -> Result<Offer> {
    ensure!(local_control(directory, true)?, "no staged package for latest accepted update");
    ensure!(
        local_control(&directory.join(MANIFEST), false)? && local_control(&directory.join(SIGNATURE), false)?,
        "staged signed metadata missing"
    );
    let raw = read_file(&directory.join(MANIFEST), MAX_METADATA)?;
    let signature = read_file(&directory.join(SIGNATURE), 256)?;
    let offer = validate_offer(state, raw, signature, target, installed, time)?;
    let package = directory.join(&offer.package.file);
    ensure!(local_control(&package, false)?, "staged package missing");
    verified_copy(File::open(package)?, io::sink(), &offer.package)?;
    Ok(offer)
}

fn stage(paths: &DataPaths, state: &State, feed: &Feed, offer: &Offer, target: &str, time: u64) -> Result<PathBuf> {
    let directory = stage_path(paths, offer);
    let parent = directory.parent().context("staging parent missing")?;
    local_control(parent, true)?;
    create_private_dir_all(parent)?;
    local_control(parent, true)?;
    if local_control(&directory, true)? {
        let existing = reverify_directory(&directory, state, target, env!("CARGO_PKG_VERSION"), time)?;
        ensure!(existing.digest == offer.digest, "staged manifest differs from reviewed offer");
        return Ok(directory.join(&offer.package.file));
    }
    ensure!(
        fs2::available_space(parent)?.saturating_sub(STAGING_FREE_RESERVE)
            >= offer.package.bytes.saturating_add(MAX_METADATA * 2),
        "insufficient free space for verified update staging"
    );
    // Same-filesystem private temporary directory. It is removed on handled
    // failure. Crash leftovers are never interpreted as an approved package.
    let temp = tempfile::Builder::new().prefix(".update-").tempdir_in(parent)?;
    let path = temp.path().join(&offer.package.file);
    let mut output = private_new_file(&path)?;
    verified_copy(
        feed.open(&offer.manifest.release_tag, &offer.package.file)?,
        ReserveWriter { file: &mut output, parent },
        &offer.package,
    )?;
    output.sync_all()?;
    drop(output);
    for (name, bytes) in [(MANIFEST, &offer.raw), (SIGNATURE, &offer.signature)] {
        let mut file = private_new_file(&temp.path().join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    sync_directory(temp.path())?;
    let finished = now()?;
    validate_time(&offer.manifest, finished)?;
    ensure!(finished >= time, "local clock moved backwards during update staging");
    ensure!(!entry(&directory, true)?, "update staging destination unexpectedly exists");
    fs::rename(temp.path(), &directory)?;
    sync_directory(parent)?;
    Ok(directory.join(&offer.package.file))
}

fn receipt(offer: &Offer, path: Option<PathBuf>) -> serde_json::Value {
    serde_json::json!({
        "channel": offer.manifest.channel, "version": offer.manifest.version,
        "commit": offer.manifest.commit, "sequence": offer.manifest.sequence.to_string(),
        "manifest_sha256": offer.digest, "expires_unix": offer.manifest.expires_unix,
        "target": offer.package.target, "package_file": offer.package.file,
        "package_sha256": offer.package.sha256, "package_bytes": offer.package.bytes,
        "manifest_signature_verified": true, "detached_package_signature_verified": true,
        "package_bytes_verified": path.is_some(), "staged_package": path,
        "automatic_installation": false,
        "next_action": "Stop all SwarmCraft processes save-first, preserve a verified checkpoint and offline profile backup, reverify staged bytes and use the native package installer manually. Stable native OS signing must also be verified."
    })
}

pub fn run(paths: &DataPaths, command: UpdateCommand) -> Result<()> {
    let result = match command {
        UpdateCommand::Status => {
            let state = load_state(paths)?;
            serde_json::json!({"preferences":state, "application_version":env!("CARGO_PKG_VERSION"),
                "automatic_checks":false, "automatic_installation":false})
        }
        UpdateCommand::Configure { channel, public_key, expected_fingerprint } => {
            serde_json::json!({"preferences":configure(paths, channel, public_key, expected_fingerprint)?,
                "automatic_checks":false, "automatic_installation":false})
        }
        UpdateCommand::Check { feed_dir, online, linux_format } => {
            // Opt-out is checked before creating directories, opening a feed
            // or touching the user's canonical profile.
            ensure!(load_state(paths)?.channel != Channel::Off, "updates are disabled");
            let _guard = lock(paths)?;
            let mut state = load_state(paths)?;
            ensure!(state.channel != Channel::Off, "updates are disabled");
            let target = native_target(linux_format)?;
            swarm_storage::profile::inspect(&paths.root)?;
            let feed = Feed::new(feed_dir, online)?;
            let offer = feed.offer(&state, target, env!("CARGO_PKG_VERSION"))?;
            let time = now()?;
            advance(paths, &mut state, &offer, time)?;
            receipt(&offer, None)
        }
        UpdateCommand::Stage { feed_dir, online, linux_format } => {
            ensure!(load_state(paths)?.channel != Channel::Off, "updates are disabled");
            let _guard = lock(paths)?;
            let mut state = load_state(paths)?;
            ensure!(state.channel != Channel::Off, "updates are disabled");
            let target = native_target(linux_format)?;
            swarm_storage::profile::inspect(&paths.root)?;
            let feed = Feed::new(feed_dir, online)?;
            let offer = feed.offer(&state, target, env!("CARGO_PKG_VERSION"))?;
            let time = now()?;
            // Persist the accepted signed offer before downloading. A failed
            // transfer cannot permit a previous offer to roll back this pin.
            advance(paths, &mut state, &offer, time)?;
            let path = stage(paths, &state, &feed, &offer, target, time)?;
            receipt(&offer, Some(path))
        }
        UpdateCommand::VerifyStaged { linux_format } => {
            ensure!(load_state(paths)?.channel != Channel::Off, "updates are disabled");
            let _guard = lock(paths)?;
            let state = load_state(paths)?;
            let water =
                state.pin(state.channel).and_then(|pin| pin.high_water.as_ref()).context("no accepted update offer")?;
            swarm_storage::profile::inspect(&paths.root)?;
            let target = native_target(linux_format)?;
            let parent = paths.root.join("updates/staged");
            ensure!(local_control(&parent, true)?, "no staged package");
            let directory = parent.join(format!(
                "{}-{}-{}-{}",
                state.channel.label(),
                water.sequence,
                water.manifest_sha256,
                target
            ));
            let offer = reverify_directory(&directory, &state, target, env!("CARGO_PKG_VERSION"), now()?)?;
            ensure!(offer.digest == water.manifest_sha256, "staged manifest differs from latest accepted offer");
            receipt(&offer, Some(directory.join(&offer.package.file)))
        }
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;

    const TIME: u64 = 1_800_000_000;
    const TARGET: &str = "linux-x86_64-deb";

    fn key_and_state(channel: Channel) -> (SigningKey, State) {
        let key = SigningKey::generate(&mut OsRng);
        let raw = key.verifying_key().to_bytes();
        let mut state = State { channel, ..Default::default() };
        *state.pin_mut(channel).unwrap() =
            Some(TrustPin { public_key: hex::encode(raw), fingerprint: sha256(&raw), high_water: None });
        (key, state)
    }

    fn fixture(channel: Channel) -> Manifest {
        Manifest {
            schema: 1,
            channel,
            sequence: 7,
            issued_unix: TIME - 10,
            expires_unix: TIME + 100,
            version: "0.5.2".into(),
            minimum_client_version: "0.5.1".into(),
            commit: "ab".repeat(20),
            release_tag: if channel == Channel::Stable {
                "v0.5.2".into()
            } else {
                format!("main-snapshot-{}", "ab".repeat(20))
            },
            protocol_version: 1,
            storage_schema_version: 1,
            reader_generation: 1,
            packages: vec![Package {
                target: TARGET.into(),
                file: "SwarmCraft_0.5.2_amd64.deb".into(),
                bytes: 5,
                sha256: sha256(b"hello"),
                signature: String::new(),
            }],
        }
    }

    fn sign_packages(manifest: &mut Manifest, key: &SigningKey) {
        let messages: Vec<_> = manifest.packages.iter().map(|package| package_message(manifest, package)).collect();
        for (package, message) in manifest.packages.iter_mut().zip(messages) {
            package.signature = hex::encode(key.sign(&message).to_bytes());
        }
    }

    fn signed_raw(manifest: &Manifest, key: &SigningKey) -> (Vec<u8>, Vec<u8>) {
        let raw = serde_json::to_vec(manifest).unwrap();
        let mut message = MANIFEST_DOMAIN.to_vec();
        message.extend_from_slice(&raw);
        (raw, hex::encode(key.sign(&message).to_bytes()).into_bytes())
    }

    fn offer(state: &State, manifest: &Manifest, key: &SigningKey) -> Result<Offer> {
        let (raw, sig) = signed_raw(manifest, key);
        validate_offer(state, raw, sig, TARGET, "0.5.1", TIME)
    }

    #[test]
    fn manifest_and_package_use_distinct_domains_and_pins() {
        let (key, state) = key_and_state(Channel::Stable);
        let mut manifest = fixture(Channel::Stable);
        sign_packages(&mut manifest, &key);
        assert!(offer(&state, &manifest, &key).is_ok());
        let (wrong, _) = key_and_state(Channel::Stable);
        assert!(offer(&state, &manifest, &wrong).is_err());
        let (mut raw, sig) = signed_raw(&manifest, &key);
        raw.push(b' ');
        assert!(validate_offer(&state, raw, sig, TARGET, "0.5.1", TIME).is_err());
        manifest.packages[0].sha256 = sha256(b"other");
        assert!(offer(&state, &manifest, &key).is_err(), "resigned manifest cannot substitute unsigned package");
    }

    #[test]
    fn signed_but_incompatible_metadata_is_rejected() {
        let (key, state) = key_and_state(Channel::Stable);
        type Alter = fn(&mut Manifest);
        let alterations: [Alter; 11] = [
            |m| m.protocol_version = 2,
            |m| m.storage_schema_version = 2,
            |m| m.reader_generation = 2,
            |m| m.minimum_client_version = "0.5.2".into(),
            |m| m.release_tag = "main-latest".into(),
            |m| m.version = "0.5.1".into(),
            |m| m.version = "00.5.2".into(),
            |m| m.expires_unix = TIME,
            |m| m.issued_unix = TIME + 1,
            |m| m.expires_unix = TIME + MAX_VALIDITY,
            |m| m.channel = Channel::Preview,
        ];
        for alter in alterations {
            let mut m = fixture(Channel::Stable);
            alter(&mut m);
            sign_packages(&mut m, &key);
            assert!(offer(&state, &m, &key).is_err(), "{m:?}");
        }
    }

    #[test]
    fn signed_inventory_rejects_unsupported_paths_duplicates_targets_and_sizes() {
        let (key, state) = key_and_state(Channel::Preview);
        type Alter = fn(&mut Manifest);
        let alterations: [Alter; 7] = [
            |m| m.packages[0].file = "../SwarmCraft_0.5.2_amd64.deb".into(),
            |m| m.packages[0].file = "SwarmCraft_0.5.2_arm64.deb".into(),
            |m| m.packages[0].bytes = MAX_PACKAGE + 1,
            |m| m.packages[0].bytes = 0,
            |m| m.packages.push(m.packages[0].clone()),
            |m| {
                m.packages[0].target = "windows-x86_64-nsis".into();
                m.packages[0].file = "SwarmCraft_0.5.2_x64-setup.exe".into();
            },
            |m| m.packages[0].target = "linux-aarch64-deb".into(),
        ];
        for alter in alterations {
            let mut m = fixture(Channel::Preview);
            alter(&mut m);
            sign_packages(&mut m, &key);
            assert!(offer(&state, &m, &key).is_err(), "{m:?}");
        }
    }

    #[test]
    fn replay_record_blocks_sequence_equivocation_version_rollback_and_clock_regression() {
        let (key, mut state) = key_and_state(Channel::Stable);
        let mut m = fixture(Channel::Stable);
        sign_packages(&mut m, &key);
        let accepted = offer(&state, &m, &key).unwrap();
        state.stable.as_mut().unwrap().high_water = Some(HighWater {
            sequence: m.sequence,
            manifest_sha256: accepted.digest,
            version: m.version.clone(),
            commit: m.commit.clone(),
            checked_unix: TIME,
        });
        assert!(offer(&state, &m, &key).is_ok());
        let mut rollback = m.clone();
        rollback.sequence -= 1;
        assert!(offer(&state, &rollback, &key).is_err());
        let mut equivocation = m.clone();
        equivocation.expires_unix += 1;
        assert!(offer(&state, &equivocation, &key).is_err());
        let mut substitution = m.clone();
        substitution.sequence += 1;
        substitution.commit = "cd".repeat(20);
        sign_packages(&mut substitution, &key);
        assert!(offer(&state, &substitution, &key).is_err());
        let (raw, sig) = signed_raw(&m, &key);
        assert!(validate_offer(&state, raw, sig, TARGET, "0.5.1", TIME - 1).is_err());
        state.stable.as_mut().unwrap().high_water.as_mut().unwrap().version = "0.5.3".into();
        m.sequence += 1;
        assert!(offer(&state, &m, &key).is_err());
    }

    #[test]
    fn raw_metadata_has_strict_fields_and_bounded_input() {
        let (key, state) = key_and_state(Channel::Stable);
        let mut m = fixture(Channel::Stable);
        sign_packages(&mut m, &key);
        let mut raw = serde_json::to_vec(&m).unwrap();
        raw.pop();
        raw.extend_from_slice(b",\"sequence\":7}");
        let mut msg = MANIFEST_DOMAIN.to_vec();
        msg.extend(&raw);
        let sig = hex::encode(key.sign(&msg).to_bytes()).into_bytes();
        assert!(validate_offer(&state, raw, sig, TARGET, "0.5.1", TIME).is_err());
        assert!(read_bounded(io::repeat(1), MAX_METADATA).is_err());
    }

    #[test]
    fn verifies_actual_byte_stream_and_rejects_truncation_extra_and_corruption() {
        let p = fixture(Channel::Stable).packages.remove(0);
        let mut output = Vec::new();
        verified_copy(&b"hello"[..], &mut output, &p).unwrap();
        assert_eq!(output, b"hello");
        for bad in [&b"hell"[..], &b"hello!"[..], &b"jello"[..]] {
            assert!(verified_copy(bad, io::sink(), &p).is_err());
        }
    }

    #[test]
    fn online_allowlist_rejects_credentials_http_ports_and_other_repositories() {
        for url in [
            "https://github.com/MousaXD/swarmcraft/releases/download/v0.5.2/SwarmCraft_0.5.2_amd64.deb",
            "https://release-assets.githubusercontent.com/github-production-release-asset/a?sig=opaque",
        ] {
            assert!(allowed_release_url(&reqwest::Url::parse(url).unwrap()));
        }
        for url in [
            "http://github.com/MousaXD/swarmcraft/releases/download/v0.5.2/x",
            "https://user@github.com/MousaXD/swarmcraft/releases/download/v0.5.2/x",
            "https://github.com:8443/MousaXD/swarmcraft/releases/download/v0.5.2/x",
            "https://github.com/other/project/releases/download/v0.5.2/x",
            "https://github.com.evil.example/MousaXD/swarmcraft/releases/download/v0.5.2/x",
            "https://evil.example/x",
            "https://github.com/MousaXD/swarmcraft/releases/download/v0.5.2/x#fragment",
        ] {
            assert!(!allowed_release_url(&reqwest::Url::parse(url).unwrap()), "{url}");
        }
    }
}
