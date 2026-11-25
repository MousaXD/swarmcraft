//! Local, non-authoritative discovery policy and minimal abuse allegations.
//! This module never opens canonical world storage, changes membership,
//! writes signed config, or authorizes a game route.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::DataPaths;
use swarm_protocol::{Hash32, WorldId};

const VERSION: u16 = 1;
const MAX_IDS: usize = 1024;
const MAX_REPORTS: usize = 128;
const MAX_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Spam,
    Impersonation,
    MaliciousContent,
    Harassment,
    MisleadingMetadata,
    Other,
}

impl FromStr for Reason {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "spam" => Self::Spam,
            "impersonation" => Self::Impersonation,
            "malicious_content" => Self::MaliciousContent,
            "harassment" => Self::Harassment,
            "misleading_metadata" => Self::MisleadingMetadata,
            "other" => Self::Other,
            _ => bail!("unsupported report category"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingReport {
    pub report_id: String,
    pub world_id: String,
    pub config_hash: String,
    pub reason: Reason,
    pub verified_unix_ms: u64,
    pub recorded_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suppression {
    pub world_id: String,
    pub reason: Reason,
    pub decided_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryPolicy {
    version: u16,
    pub hidden_worlds: BTreeSet<String>,
    pub directory_suppressions: Vec<Suppression>,
    pub reports: Vec<ListingReport>,
}

impl Default for DirectoryPolicy {
    fn default() -> Self {
        Self {
            version: VERSION,
            hidden_worlds: BTreeSet::new(),
            directory_suppressions: Vec::new(),
            reports: Vec::new(),
        }
    }
}

impl DirectoryPolicy {
    pub fn hidden(&self, world: WorldId) -> bool {
        self.hidden_worlds.contains(&world.to_string())
    }

    pub fn suppressed(&self, world: WorldId) -> bool {
        self.directory_suppressions.iter().any(|entry| entry.world_id == world.to_string())
    }

    fn validate(&self) -> Result<()> {
        if self.version != VERSION
            || self.hidden_worlds.len() > MAX_IDS
            || self.directory_suppressions.len() > MAX_IDS
            || self.reports.len() > MAX_REPORTS
        {
            bail!("unsupported or oversized directory policy");
        }
        for id in &self.hidden_worlds {
            parse_world(id)?;
        }
        let mut suppressed = BTreeSet::new();
        for entry in &self.directory_suppressions {
            parse_world(&entry.world_id)?;
            if entry.decided_unix_ms == 0 || !suppressed.insert(&entry.world_id) {
                bail!("invalid directory suppression");
            }
        }
        let mut reports = BTreeSet::new();
        for report in &self.reports {
            validate_report(report)?;
            if !reports.insert(&report.report_id) {
                bail!("duplicate local listing report");
            }
        }
        Ok(())
    }
}

fn validate_report(report: &ListingReport) -> Result<()> {
    parse_world(&report.world_id)?;
    parse_hash(&report.config_hash)?;
    if report.report_id != report_id(&report.world_id, &report.config_hash, report.reason)?
        || report.verified_unix_ms == 0
        || report.recorded_unix_ms < report.verified_unix_ms
    {
        bail!("invalid local listing report");
    }
    Ok(())
}

const REPORT_SCHEMA: &str = "swarmcraft-local-listing-report-v1";
const MAX_EXPORTED_REPORT_BYTES: u64 = 4096;

// Flat, allow-listed envelope: never carry untrusted free text, peer identity,
// private addresses or a signed invite. This is NOT proof of abusive conduct.
#[derive(Debug, Clone, Serialize)]
pub struct ExportedReport {
    schema: &'static str,
    allegation: bool,
    #[serde(flatten)]
    report: ListingReport,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IncomingReport {
    schema: String,
    allegation: bool,
    report_id: String,
    world_id: String,
    config_hash: String,
    reason: Reason,
    verified_unix_ms: u64,
    recorded_unix_ms: u64,
}

pub fn export_report(paths: &DataPaths, id: &str) -> Result<ExportedReport> {
    parse_hash(id)?;
    let policy = state(paths)?;
    let report = policy.reports.into_iter().find(|entry| entry.report_id == id).context("local report not found")?;
    Ok(ExportedReport { schema: REPORT_SCHEMA, allegation: true, report })
}

fn read_external_report(file: &Path) -> Result<ListingReport> {
    let meta = fs::symlink_metadata(file)?;
    if !meta.is_file() || meta.len() > MAX_EXPORTED_REPORT_BYTES {
        bail!("report must be a bounded regular file");
    }
    let mut bytes = Vec::new();
    File::open(file)?.take(MAX_EXPORTED_REPORT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_EXPORTED_REPORT_BYTES {
        bail!("report exceeds size limit");
    }
    let raw: IncomingReport = serde_json::from_slice(&bytes).context("invalid report envelope")?;
    if raw.schema != REPORT_SCHEMA || !raw.allegation {
        bail!("unsupported report envelope");
    }
    let report = ListingReport {
        report_id: raw.report_id,
        world_id: raw.world_id,
        config_hash: raw.config_hash,
        reason: raw.reason,
        verified_unix_ms: raw.verified_unix_ms,
        recorded_unix_ms: raw.recorded_unix_ms,
    };
    validate_report(&report)?;
    Ok(report)
}

#[derive(Debug, Serialize)]
pub struct ReportReview {
    report: ExportedReport,
    matching_current_public_listing: bool,
    reporter_allegation_verified: bool,
    automatic_decision: bool,
}

/// Operator review of an explicitly provided, untrusted allegation.
/// Only the current public listing is independently reverified; the truth
/// of the allegation is never inferred and no policy is changed.
pub async fn review_report(paths: &DataPaths, file: &Path, bootstrap: &[String]) -> Result<ReportReview> {
    let report = read_external_report(file)?;
    let result = crate::discovery::resolve_world(paths, parse_world(&report.world_id)?, bootstrap).await?;
    let matching = result.state == "found"
        && result.world.as_ref().is_some_and(|card| {
            card.visibility == "public" && card.world_id == report.world_id && card.config_hash == report.config_hash
        });
    Ok(ReportReview {
        report: ExportedReport { schema: REPORT_SCHEMA, allegation: true, report },
        matching_current_public_listing: matching,
        reporter_allegation_verified: false,
        automatic_decision: false,
    })
}

fn parse_world(value: &str) -> Result<WorldId> {
    let world: WorldId = value.parse().context("invalid world ID")?;
    if value != world.to_string() {
        bail!("world ID must be canonical");
    }
    Ok(world)
}

fn parse_hash(value: &str) -> Result<Hash32> {
    let hash: Hash32 = value.parse().context("invalid listing configuration hash")?;
    if hash.to_string() != value {
        bail!("hash must be canonical");
    }
    Ok(hash)
}

fn report_id(world: &str, config: &str, reason: Reason) -> Result<String> {
    let bytes = serde_json::to_vec(&(world, config, reason))?;
    Ok(Hash32::from_domain_bytes(b"swarmcraft/local-listing-report/v1\0", &bytes).to_string())
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?)
}

fn path(paths: &DataPaths) -> PathBuf {
    paths.root.join("directory-policy.json")
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// All mutations and the browse response boundary hold this same lock.
/// No world transaction or signed canonical state is acquired here.
pub(crate) fn lock(paths: &DataPaths) -> Result<File> {
    swarm_core::create_private_dir_all(&paths.root)?;
    let lock_path = paths.root.join("directory-policy.lock");
    require_regular_or_missing(&lock_path)?;
    let file = private_options().open(lock_path)?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

fn require_regular_or_missing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => bail!("directory policy path is not a regular file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn read_locked(paths: &DataPaths) -> Result<DirectoryPolicy> {
    require_regular_or_missing(&path(paths))?;
    let file = match File::open(path(paths)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(DirectoryPolicy::default()),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("directory policy exceeds size limit");
    }
    let state: DirectoryPolicy = serde_json::from_slice(&bytes).context("directory policy is malformed")?;
    state.validate()?;
    Ok(state)
}

pub fn state(paths: &DataPaths) -> Result<DirectoryPolicy> {
    let _lock = lock(paths)?;
    read_locked(paths)
}

fn mutate(paths: &DataPaths, action: impl FnOnce(&mut DirectoryPolicy) -> Result<()>) -> Result<DirectoryPolicy> {
    let _lock = lock(paths)?;
    let mut state = read_locked(paths)?;
    action(&mut state)?;
    state.validate()?;
    let bytes = serde_json::to_vec_pretty(&state)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("directory policy exceeds size limit");
    }
    let target = path(paths);
    let temp = target.with_extension(format!("json.{}.tmp", hex::encode(swarm_core::random_nonce())));
    let mut file = private_options().create_new(true).open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    #[cfg(unix)]
    File::open(&paths.root)?.sync_all()?;
    Ok(state)
}

pub fn set_hidden(paths: &DataPaths, world: WorldId, hidden: bool) -> Result<DirectoryPolicy> {
    mutate(paths, |state| {
        let id = world.to_string();
        if hidden {
            state.hidden_worlds.insert(id);
        } else {
            state.hidden_worlds.remove(&id);
        }
        Ok(())
    })
}

/// Operator chooses only what this publisher will return in public browse.
/// Exact-ID discovery and all canonical world protocol paths are unchanged.
pub fn set_suppression(paths: &DataPaths, world: WorldId, reason: Option<Reason>) -> Result<DirectoryPolicy> {
    mutate(paths, |state| {
        let id = world.to_string();
        state.directory_suppressions.retain(|entry| entry.world_id != id);
        if let Some(reason) = reason {
            state.directory_suppressions.push(Suppression { world_id: id, reason, decided_unix_ms: now()? });
        }
        state.directory_suppressions.sort_by(|a, b| a.world_id.cmp(&b.world_id));
        Ok(())
    })
}

pub async fn report_public_listing(
    paths: &DataPaths,
    world: WorldId,
    expected_config: Hash32,
    reason: Reason,
    bootstrap: &[String],
) -> Result<DirectoryPolicy> {
    // Never trust a renderer-supplied description/metadata or accept a report
    // against an unlisted/private ID. Reverify the current public listing.
    let result = crate::discovery::resolve_world(paths, world, bootstrap).await?;
    let card = result
        .world
        .filter(|card| {
            result.state == "found"
                && card.visibility == "public"
                && card.world_id == world.to_string()
                && card.config_hash == expected_config.to_string()
        })
        .context("the exact public listing could not be reverified; search again before reporting")?;
    let recorded = now()?;
    if card.expires_unix_ms <= recorded || card.verified_unix_ms > recorded {
        bail!("the listing observation expired before reporting");
    }
    let report = ListingReport {
        report_id: report_id(&card.world_id, &card.config_hash, reason)?,
        world_id: card.world_id,
        config_hash: card.config_hash,
        reason,
        verified_unix_ms: card.verified_unix_ms,
        recorded_unix_ms: recorded,
    };
    mutate(paths, |state| {
        if !state.reports.iter().any(|old| old.report_id == report.report_id) {
            state.reports.push(report);
        }
        Ok(())
    })
}

pub fn dismiss_report(paths: &DataPaths, report: &str) -> Result<DirectoryPolicy> {
    parse_hash(report)?;
    mutate(paths, |state| {
        state.reports.retain(|entry| entry.report_id != report);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_durable_policy_controls_and_no_canonical_state() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let world = WorldId([1; 32]);
        set_hidden(&paths, world, true).unwrap();
        let policy = state(&paths).unwrap();
        assert!(policy.hidden(world));
        assert!(!policy.suppressed(world));
        set_suppression(&paths, world, Some(Reason::Spam)).unwrap();
        set_hidden(&paths, world, false).unwrap();
        let policy = state(&paths).unwrap();
        assert!(!policy.hidden(world));
        assert!(policy.suppressed(world));
        assert!(policy.reports.is_empty());
        set_suppression(&paths, world, None).unwrap();
        assert!(!state(&paths).unwrap().suppressed(world));
        assert!(!paths.root.join("worlds").exists());
    }

    #[test]
    fn malformed_policy_fails_closed_and_cannot_be_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        fs::write(path(&paths), b"{bad").unwrap();
        assert!(state(&paths).is_err());
        assert!(set_hidden(&paths, WorldId([1; 32]), false).is_err());
        assert_eq!(fs::read(path(&paths)).unwrap(), b"{bad");
    }

    #[test]
    fn report_envelope_rejects_tampered_categories_and_unexpected_data() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("report.json");
        let world = WorldId([3; 32]).to_string();
        let config = Hash32([4; 32]).to_string();
        let report = ListingReport {
            report_id: report_id(&world, &config, Reason::Spam).unwrap(),
            world_id: world,
            config_hash: config,
            reason: Reason::Spam,
            verified_unix_ms: 1,
            recorded_unix_ms: 2,
        };
        let envelope = ExportedReport { schema: REPORT_SCHEMA, allegation: true, report: report.clone() };
        fs::write(&file, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert_eq!(read_external_report(&file).unwrap(), report);
        let mut raw = serde_json::to_value(&envelope).unwrap();
        raw["reason"] = serde_json::json!("harassment");
        fs::write(&file, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(read_external_report(&file).is_err());
        raw["reason"] = serde_json::json!("spam");
        raw["token"] = serde_json::json!("scinvite:00");
        fs::write(&file, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(read_external_report(&file).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_policy_paths_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let outside = temp.path().join("unrelated");
        fs::write(&outside, b"unchanged").unwrap();
        std::os::unix::fs::symlink(&outside, path(&paths)).unwrap();
        assert!(state(&paths).is_err());
        assert!(set_hidden(&paths, WorldId([1; 32]), true).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"unchanged");
    }

    #[test]
    fn concurrent_hide_and_suppression_preserve_both() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        let a = WorldId([1; 32]);
        let b = WorldId([2; 32]);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                set_hidden(&paths, a, true).unwrap();
            });
            scope.spawn(|| {
                set_suppression(&paths, b, Some(Reason::Impersonation)).unwrap();
            });
        });
        let policy = state(&paths).unwrap();
        assert!(policy.hidden(a));
        assert!(policy.suppressed(b));
    }
}
