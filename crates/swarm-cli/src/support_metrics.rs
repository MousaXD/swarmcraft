//! Opt-in bounded operational observations. No log parsing, identifiers,
//! addresses, arguments, error strings, tokens, or world contents are exported.
//! No observation is a capability, authority decision, or signed audit.
use anyhow::{bail, ensure, Context, Result};
use clap::{Subcommand, ValueEnum};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::{
        mpsc::{self, SyncSender},
        OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use swarm_core::{create_private_dir_all, DataPaths};
use swarm_protocol::{WorldId, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION};

use crate::{
    game_route_api::{self, GameRouteApiOperation, GameRouteApiReply, GameRouteDiagnostic, GameRouteStatusV1},
    host_readiness::{HostReadinessReport, HostReadinessState, HOST_READINESS_MAX_AGE_MS},
    migration::{MigrationPhase, MigrationStatus},
};
use swarm_network::game_transport::{GameSessionDiagnostic, GAME_SESSION_DIAGNOSTIC_MAX_AGE_MS};
use swarm_storage::profile::{ProfileCompatibilityV1, PROFILE_READER_GENERATION};

const MAX_EVENTS: usize = 128;
const MAX_STORE_BYTES: u64 = 128 * 1024;
const MAX_CONTROL_BYTES: u64 = 256 * 1024;
const MAX_REPORT_BYTES: usize = 256 * 1024;
const MAX_DURATION_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_PAYLOAD: u64 = 1 << 40;
const MAX_COUNT: u32 = 1_000_000;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);
const QUEUE_CAPACITY: usize = 64;
const STATE_FILE: &str = "support-metrics-v1.json";
const LOCK_FILE: &str = "support-metrics-v1.lock";

#[derive(Debug, Subcommand)]
pub enum SupportCommand {
    /// Inspect local metric collection preference, without creating a store.
    Status,
    /// Enable or pause best-effort local metric collection.
    Configure {
        #[arg(long, value_enum)]
        collection: Collection,
    },
    /// Delete local operational observations (not worlds, logs or Desktop history).
    Clear {
        #[arg(long)]
        confirm: bool,
    },
    /// Explicit redacted JSON report to stdout, optionally with one world observation.
    Report {
        #[arg(long)]
        confirm: bool,
        #[arg(long)]
        world: Option<WorldId>,
        /// Ask the actual daemon for fresh status (never select or open a route).
        #[arg(long, requires = "world")]
        live_route: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Collection {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    StorageOpen,
    DaemonListener,
    DaemonExit,
    ReplicaPull,
    ReplicaFinalize,
    LeasePermit,
    FreshQuorum,
    GameRoute,
    RuntimeRestore,
    RuntimePrepare,
    CheckpointVerify,
    CheckpointRestore,
    ConflictExport,
    CheckpointPublication,
    HistoricalPublication,
    DiscoveryRefresh,
    DiscoveryPreference,
    BackupExport,
    BackupImport,
    UpdateCheck,
    UpdateStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Failed,
    Deferred,
    StoragePressure,
    Stalled,
    Paused,
    Fenced,
    QuorumMissing,
    PermitRefreshed,
    RouteVerified,
    GatewayUnavailable,
    PeerLinkUnavailable,
    HostProofUnavailable,
    RouteCheckTimedOut,
    AuthorityChanged,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferCounters {
    pub payload_bytes: u64,
    pub resumed_blobs: u32,
    pub failed_requests: u32,
    pub source_count: u32,
    pub max_parallel: u32,
}

impl TransferCounters {
    pub fn bounded(payload_bytes: u64, resumed: usize, failures: usize, sources: usize, parallel: usize) -> Self {
        let cap = |n: usize| n.min(MAX_COUNT as usize) as u32;
        Self {
            payload_bytes: payload_bytes.min(MAX_PAYLOAD),
            resumed_blobs: cap(resumed),
            failed_requests: cap(failures),
            source_count: cap(sources),
            max_parallel: cap(parallel),
        }
    }
    fn valid(self) -> bool {
        self.payload_bytes <= MAX_PAYLOAD
            && [self.resumed_blobs, self.failed_requests, self.source_count, self.max_parallel]
                .into_iter()
                .all(|n| n <= MAX_COUNT)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Tree {
    Clean,
    Modified,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Build {
    version: String,
    commit: Option<String>,
    tree: Tree,
}

impl Build {
    fn current() -> Self {
        let commit = env!("SWARMCRAFT_CLI_SUPPORT_COMMIT");
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            commit: valid_commit(commit).then(|| commit.to_owned()),
            tree: match env!("SWARMCRAFT_CLI_SUPPORT_TREE") {
                "clean" if valid_commit(commit) => Tree::Clean,
                "modified" => Tree::Modified,
                _ => Tree::Unknown,
            },
        }
    }
    fn valid(&self) -> bool {
        let parts: Vec<_> = self.version.split('.').collect();
        parts.len() == 3
            && parts.iter().all(|part| !part.is_empty() && part.len() <= 10 && part.bytes().all(|b| b.is_ascii_digit()))
            && self.commit.as_deref().is_none_or(valid_commit)
            && (!matches!(self.tree, Tree::Clean) || self.commit.is_some())
    }
}

fn valid_commit(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    observed_unix_ms: u64,
    build: Build,
    metric: Metric,
    outcome: Outcome,
    elapsed_ms: Option<u64>,
    transfer: Option<TransferCounters>,
}

impl Event {
    fn new(
        metric: Metric,
        outcome: Outcome,
        started: Option<Instant>,
        transfer: Option<TransferCounters>,
    ) -> Option<Self> {
        Some(Self {
            observed_unix_ms: now_ms()?,
            build: Build::current(),
            metric,
            outcome,
            elapsed_ms: started.map(|instant| instant.elapsed().as_millis().min(MAX_DURATION_MS as u128) as u64),
            transfer,
        })
    }
    fn valid(&self) -> bool {
        let outcome_matches = match self.metric {
            Metric::ReplicaPull => {
                matches!(
                    self.outcome,
                    Outcome::Completed
                        | Outcome::Paused
                        | Outcome::Fenced
                        | Outcome::Stalled
                        | Outcome::StoragePressure
                        | Outcome::Deferred
                ) && self.transfer.is_some()
            }
            Metric::LeasePermit => matches!(
                self.outcome,
                Outcome::Fenced | Outcome::QuorumMissing | Outcome::PermitRefreshed | Outcome::Failed
            ),
            Metric::GameRoute => matches!(
                self.outcome,
                Outcome::RouteVerified
                    | Outcome::GatewayUnavailable
                    | Outcome::PeerLinkUnavailable
                    | Outcome::HostProofUnavailable
                    | Outcome::RouteCheckTimedOut
                    | Outcome::AuthorityChanged
                    | Outcome::Unavailable
            ),
            Metric::HistoricalPublication => matches!(self.outcome, Outcome::Completed | Outcome::Deferred),
            _ => matches!(self.outcome, Outcome::Completed | Outcome::Failed),
        };
        outcome_matches
            && self.observed_unix_ms > 0
            && self.build.valid()
            && self.elapsed_ms.is_none_or(|ms| ms <= MAX_DURATION_MS)
            && self.transfer.is_none_or(|t| self.metric == Metric::ReplicaPull && t.valid())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    schema: u16,
    enabled: bool,
    revision: u64,
    events: VecDeque<Event>,
    older_events_dropped: bool,
}

impl Default for Store {
    fn default() -> Self {
        Self { schema: 1, enabled: false, revision: 0, events: VecDeque::new(), older_events_dropped: false }
    }
}
impl Store {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1 && self.events.len() <= MAX_EVENTS && self.events.iter().all(Event::valid),
            "invalid operational support store"
        );
        Ok(())
    }
    fn append(&mut self, event: Event) {
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
            self.older_events_dropped = true;
        }
        self.events.push_back(event);
    }
}

fn now_ms() -> Option<u64> {
    let ms = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()?;
    (ms > 0).then_some(ms)
}

fn entry(path: &Path, directory: bool, private: bool) -> Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        !meta.file_type().is_symlink() && if directory { meta.is_dir() } else { meta.is_file() },
        "unsafe support file or directory"
    );
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.mode() & 0o022 == 0 && (directory || meta.nlink() == 1),
            "unsafe writable or hard-linked support store"
        );
        #[cfg(target_os = "linux")]
        ensure!(meta.uid() == crate::machine_hosting::linux::uid(), "support store owner mismatch");
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(true)
}

fn bounded(path: &Path, max: u64) -> Result<Option<Vec<u8>>> {
    if !entry(path, false, false)? {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "oversized support input");
    Ok(Some(bytes))
}

fn typed<T: DeserializeOwned>(path: &Path, max: u64) -> Result<Option<T>> {
    bounded(path, max)?.map(|bytes| serde_json::from_slice(&bytes).map_err(Into::into)).transpose()
}

fn load(paths: &DataPaths) -> Result<Store> {
    if !entry(&paths.root, true, true)? || !entry(&paths.root.join(STATE_FILE), false, true)? {
        return Ok(Store::default());
    }
    let state: Store = typed(&paths.root.join(STATE_FILE), MAX_STORE_BYTES)?.context("support store disappeared")?;
    state.validate()?;
    Ok(state)
}

fn lock(paths: &DataPaths, create: bool) -> Result<File> {
    entry(&paths.root, true, true)?;
    if create {
        create_private_dir_all(&paths.root)?;
    }
    ensure!(entry(&paths.root, true, true)?, "support root missing");
    let path = paths.root.join(LOCK_FILE);
    entry(&path, false, true)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    entry(&path, false, true)?;
    FileExt::try_lock_exclusive(&file).context("operational support store busy; retry")?;
    Ok(file)
}

fn save(paths: &DataPaths, state: &Store) -> Result<()> {
    state.validate()?;
    let bytes = serde_json::to_vec(state)?;
    ensure!(bytes.len() as u64 <= MAX_STORE_BYTES, "oversized support store");
    let path = paths.root.join(STATE_FILE);
    entry(&path, false, true)?;
    let mut file = tempfile::NamedTempFile::new_in(&paths.root)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    crate::authority_permit::replace_permit_atomically(&file.into_temp_path(), &path)?;
    #[cfg(unix)]
    File::open(&paths.root)?.sync_all()?;
    Ok(())
}

fn revision(paths: &DataPaths) -> Option<u64> {
    let store = load(paths).ok()?;
    store.enabled.then_some(store.revision)
}

fn append(paths: &DataPaths, expected: u64, events: impl IntoIterator<Item = Event>) -> Result<bool> {
    // No creation or resurrection by a process with an earlier consent revision.
    if !entry(&paths.root.join(STATE_FILE), false, true)? {
        return Ok(false);
    }
    let _guard = lock(paths, false)?;
    let mut state = load(paths)?;
    if !state.enabled || state.revision != expected {
        return Ok(false);
    }
    // Never update diagnostic state in a root that has advanced to an
    // unsupported reader floor, even after an earlier opt-in.
    if !matches!(marker(paths), Availability::Observed | Availability::Missing) {
        return Ok(false);
    }
    for event in events {
        state.append(event);
    }
    save(paths, &state)?;
    Ok(true)
}

/// Finite explicit CLI operations: record only the typed result after the
/// real operation finishes. Any diagnostic failure leaves its result intact.
pub fn measure<T, E>(
    paths: &DataPaths,
    metric: Metric,
    action: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let consent = revision(paths).filter(|_| {
        // Storage-open failure may indicate unsupported legacy world
        // metadata even without a new-format marker.
        metric != Metric::StorageOpen || swarm_storage::profile::inspect(&paths.root).is_ok()
    });
    let started = Instant::now();
    let result = action();
    if let Some(expected) = consent {
        if let Some(event) =
            Event::new(metric, if result.is_ok() { Outcome::Completed } else { Outcome::Failed }, Some(started), None)
        {
            let _ = append(paths, expected, [event]);
        }
    }
    result
}

static SENDER: OnceLock<SyncSender<Event>> = OnceLock::new();

/// Long-running sidecars enqueue only bounded typed data. Disk I/O is never
/// performed in a daemon/lease/network callback. No wait on full queue.
/// The worker is best effort: abrupt exit can lose its bounded tail.
pub fn start_background(paths: &DataPaths) {
    let Some(expected) = revision(paths) else { return };
    let paths = paths.clone();
    let (sender, receiver) = mpsc::sync_channel::<Event>(QUEUE_CAPACITY);
    if SENDER.set(sender).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("support-metrics".into()).spawn(move || {
        let mut last = BTreeMap::<(Metric, Outcome), Instant>::new();
        while let Ok(first) = receiver.recv() {
            let mut events = Vec::new();
            for event in std::iter::once(first).chain(receiver.try_iter().take(31)) {
                // Lease, route and directory refresh are periodic samples,
                // not totals. Bound repetitive I/O without suppressing a
                // different outcome category from a different world.
                let sampled =
                    matches!(event.metric, Metric::LeasePermit | Metric::GameRoute | Metric::DiscoveryRefresh);
                let key = (event.metric, event.outcome);
                let time = Instant::now();
                if sampled
                    && last.get(&key).is_some_and(|prior| time.saturating_duration_since(*prior) < SAMPLE_INTERVAL)
                {
                    continue;
                }
                if sampled {
                    last.insert(key, time);
                }
                events.push(event);
            }
            if !events.is_empty() && matches!(append(&paths, expected, events), Ok(false)) {
                break;
            }
        }
    });
}

pub fn observe(metric: Metric, outcome: Outcome, started: Option<Instant>, transfer: Option<TransferCounters>) {
    if let Some(sender) = SENDER.get() {
        if let Some(event) = Event::new(metric, outcome, started, transfer) {
            let _ = sender.try_send(event);
        }
    }
}

pub fn observe_result<T, E>(metric: Metric, started: Instant, result: &std::result::Result<T, E>) {
    observe(metric, if result.is_ok() { Outcome::Completed } else { Outcome::Failed }, Some(started), None);
}

pub fn route_outcome(status: &GameRouteStatusV1) -> Outcome {
    if status.route_verified {
        return Outcome::RouteVerified;
    }
    match status.diagnostic {
        Some(GameRouteDiagnostic::GatewayUnavailable) => Outcome::GatewayUnavailable,
        Some(GameRouteDiagnostic::PeerLinkUnavailable) => Outcome::PeerLinkUnavailable,
        Some(GameRouteDiagnostic::HostProofUnavailable) => Outcome::HostProofUnavailable,
        Some(GameRouteDiagnostic::RouteCheckTimedOut) => Outcome::RouteCheckTimedOut,
        Some(GameRouteDiagnostic::AuthorityChanged) => Outcome::AuthorityChanged,
        None => Outcome::Unavailable,
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Availability {
    Missing,
    Unavailable,
    Stale,
    Observed,
    Incompatible,
}

#[derive(Debug, Serialize)]
struct Readiness {
    availability: Availability,
    age_ms: Option<u64>,
    state: Option<HostReadinessState>,
    matching_peer_count: Option<usize>,
    exact_replica_receipt_count: Option<usize>,
    preferred_total_copies: Option<u16>,
}

impl Readiness {
    fn unavailable(availability: Availability) -> Self {
        Self {
            availability,
            age_ms: None,
            state: None,
            matching_peer_count: None,
            exact_replica_receipt_count: None,
            preferred_total_copies: None,
        }
    }
}

fn readiness(paths: &DataPaths, world: WorldId, time: u64) -> Readiness {
    let directory = paths.root.join("control").join(world.to_hex());
    if !safe_chain(&paths.root, &directory) {
        return Readiness::unavailable(Availability::Unavailable);
    }
    let report: HostReadinessReport =
        match typed::<HostReadinessReport>(&directory.join("host-readiness.json"), MAX_CONTROL_BYTES) {
            Ok(Some(report)) if report.world_id == world.to_string() => report,
            Ok(None) => return Readiness::unavailable(Availability::Missing),
            _ => return Readiness::unavailable(Availability::Unavailable),
        };
    let Some(age) = time.checked_sub(report.generated_unix_ms) else {
        return Readiness::unavailable(Availability::Unavailable);
    };
    if report.generated_unix_ms == 0 || age > HOST_READINESS_MAX_AGE_MS {
        return Readiness { age_ms: Some(age.min(MAX_DURATION_MS)), ..Readiness::unavailable(Availability::Stale) };
    }
    let coverage = report.checkpoint.as_ref().and_then(|p| p.replica_coverage.as_ref());
    Readiness {
        availability: Availability::Observed,
        age_ms: Some(age),
        state: Some(report.state),
        matching_peer_count: coverage.map(|c| c.connected_matching_status_peers.len().min(MAX_COUNT as usize)),
        exact_replica_receipt_count: coverage.map(|c| c.connected_exact_receipt_peers.len().min(MAX_COUNT as usize)),
        preferred_total_copies: coverage.map(|c| c.preferred_total_copies),
    }
}

#[derive(Debug, Serialize)]
struct MigrationObservation {
    availability: Availability,
    last_recorded_phase: Option<MigrationPhase>,
    recorded_failure: Option<bool>,
    historical_only: bool,
}

fn migration_observation(paths: &DataPaths, world: WorldId) -> MigrationObservation {
    let directory = paths.root.join("control").join(world.to_hex());
    let result = if safe_chain(&paths.root, &directory) {
        typed::<MigrationStatus>(&directory.join("migration-status.json"), MAX_CONTROL_BYTES)
    } else {
        Err(anyhow::anyhow!("unsafe control directory"))
    };
    let (availability, phase, failure) = match result {
        Ok(Some(status)) if status.world_id == world.to_string() => {
            (Availability::Observed, Some(status.phase), Some(status.failure_reason.is_some()))
        }
        Ok(None) => (Availability::Missing, None, None),
        _ => (Availability::Unavailable, None, None),
    };
    MigrationObservation { availability, last_recorded_phase: phase, recorded_failure: failure, historical_only: true }
}

fn safe_chain(root: &Path, child: &Path) -> bool {
    let Ok(relative) = child.strip_prefix(root) else { return false };
    let mut cursor = root.to_path_buf();
    if entry(&cursor, true, true).is_err() {
        return false;
    }
    for segment in relative.components() {
        cursor.push(segment);
        if entry(&cursor, true, true).is_err() {
            return false;
        }
    }
    true
}

fn marker(paths: &DataPaths) -> Availability {
    if entry(&paths.root, true, true).is_err() {
        return Availability::Unavailable;
    }
    match typed::<ProfileCompatibilityV1>(&paths.root.join("profile-compatibility.json"), 16 * 1024) {
        Ok(None) => Availability::Missing,
        Ok(Some(record))
            if record.format_schema == 1
                && record.minimum_reader_generation == PROFILE_READER_GENERATION
                && record.protocol_version == PROTOCOL_VERSION
                && record.storage_schema_version == STORAGE_SCHEMA_VERSION =>
        {
            Availability::Observed
        }
        Ok(Some(_)) => Availability::Incompatible,
        Err(_) => Availability::Unavailable,
    }
}

#[derive(Debug, Serialize)]
struct PublicationPreference {
    availability: Availability,
    locally_paused: Option<bool>,
    remote_publication_confirmed: bool,
}
fn publication(paths: &DataPaths, world: WorldId) -> PublicationPreference {
    let dir = paths.root.join("worlds").join(world.to_hex()).join("metadata");
    let (availability, paused) = if !safe_chain(&paths.root, &dir) {
        (Availability::Unavailable, None)
    } else if !entry(&dir.join("world.json"), false, false).unwrap_or(false) {
        (Availability::Missing, None)
    } else {
        match bounded(&dir.join("discovery-listing.paused"), 64) {
            Ok(None) => (Availability::Observed, Some(false)),
            Ok(Some(bytes)) if bytes == b"swarmcraft-discovery-paused-v1\n" => (Availability::Observed, Some(true)),
            _ => (Availability::Unavailable, None),
        }
    };
    PublicationPreference { availability, locally_paused: paused, remote_publication_confirmed: false }
}

#[derive(Debug, Serialize)]
struct RouteObservation {
    availability: Availability,
    outcome: Option<Outcome>,
    proof_fresh_at_observation: bool,
    recent_session: Option<GameSessionDiagnostic>,
}

fn project_route(status: &GameRouteStatusV1, world: WorldId, time: u64) -> RouteObservation {
    if status.world_id != world.to_string()
        || (!status.route_verified
            && (status.state != "unavailable"
                || status.quorum_verified
                || status.hosting_consent_verified
                || status.server_ready
                || status.local_gateway.is_some()))
    {
        return RouteObservation {
            availability: Availability::Unavailable,
            outcome: None,
            proof_fresh_at_observation: false,
            recent_session: None,
        };
    }
    let fresh = status.state == "verified"
        && status.route_verified
        && status.quorum_verified
        && status.hosting_consent_verified
        && status.server_ready
        && status.diagnostic.is_none()
        && status.epoch.is_some_and(|value| value > 0)
        && status.fencing_token.is_some_and(|value| value > 0)
        && status.authority_peer_id.is_some()
        && status.snapshot_hash.is_some()
        && status
            .local_gateway
            .as_deref()
            .and_then(|value| value.parse::<std::net::SocketAddrV4>().ok())
            .is_some_and(|address| *address.ip() == std::net::Ipv4Addr::LOCALHOST && address.port() > 0)
        && status.verified_at_unix_ms.is_some_and(|t| t > 0 && t <= time)
        && status.valid_until_unix_ms.is_some_and(|until| until > time);
    let outcome = if status.route_verified && !fresh { None } else { Some(route_outcome(status)) };
    RouteObservation {
        availability: if status.route_verified && !fresh { Availability::Stale } else { Availability::Observed },
        outcome,
        proof_fresh_at_observation: fresh,
        recent_session: status.recent_session.filter(|s| s.age_ms < GAME_SESSION_DIAGNOSTIC_MAX_AGE_MS),
    }
}

fn route(paths: &DataPaths, world: WorldId) -> RouteObservation {
    let result = tokio::runtime::Runtime::new().ok().and_then(|runtime| {
        runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(24),
                game_route_api::call_live_daemon(paths, GameRouteApiOperation::Status, world),
            )
            .await
            .ok()?
            .ok()
        })
    });
    if let Some(GameRouteApiReply::Status { value }) = result {
        return project_route(&value, world, now_ms().unwrap_or(0));
    }
    RouteObservation {
        availability: Availability::Unavailable,
        outcome: None,
        proof_fresh_at_observation: false,
        recent_session: None,
    }
}

#[derive(Debug, Serialize)]
struct SelectedWorld {
    readiness_cache: Readiness,
    migration_record: MigrationObservation,
    discovery_preference: PublicationPreference,
    live_route: Option<RouteObservation>,
}

#[derive(Default, Serialize)]
struct Summary {
    retained_observations: usize,
    outcomes: BTreeMap<Outcome, usize>,
    timed_samples: usize,
    p50_elapsed_ms: Option<u64>,
    p95_elapsed_ms: Option<u64>,
}

fn summarize(events: &VecDeque<Event>) -> BTreeMap<Metric, Summary> {
    let mut output = BTreeMap::<Metric, Summary>::new();
    let mut times = BTreeMap::<Metric, Vec<u64>>::new();
    for event in events {
        let row = output.entry(event.metric).or_default();
        row.retained_observations += 1;
        *row.outcomes.entry(event.outcome).or_default() += 1;
        if let Some(ms) = event.elapsed_ms {
            times.entry(event.metric).or_default().push(ms);
        }
    }
    for (metric, mut values) in times {
        values.sort_unstable();
        let row = output.get_mut(&metric).expect("observed metric");
        row.timed_samples = values.len();
        row.p50_elapsed_ms = Some(values[(values.len() * 50).div_ceil(100) - 1]);
        row.p95_elapsed_ms = Some(values[(values.len() * 95).div_ceil(100) - 1]);
    }
    output
}

fn change(paths: &DataPaths, enabled: Option<bool>, clear: bool) -> Result<Store> {
    // The support preference itself is never a migration or bypass.
    swarm_storage::profile::inspect(&paths.root)?;
    let _guard = lock(paths, true)?;
    let mut state = load(paths)?;
    if clear || enabled.is_some_and(|value| value != state.enabled) {
        state.revision = state.revision.checked_add(1).context("support consent revision exhausted")?;
    }
    if clear {
        state.events.clear();
        state.older_events_dropped = false;
    }
    if let Some(value) = enabled {
        state.enabled = value;
    }
    save(paths, &state)?;
    Ok(state)
}

fn report(paths: &DataPaths, state: &Store, world: Option<WorldId>, live: bool) -> Result<serde_json::Value> {
    let time = now_ms().context("system time unavailable")?;
    let selected = world.map(|world| SelectedWorld {
        readiness_cache: readiness(paths, world, time),
        migration_record: migration_observation(paths, world),
        discovery_preference: publication(paths, world),
        live_route: live.then(|| route(paths, world)),
    });
    Ok(serde_json::json!({
        "schema":"swarmcraft-operational-support-v1", "generated_unix_ms": time,
        "build": Build::current(), "protocol_version": PROTOCOL_VERSION,
        "storage_schema_version": STORAGE_SCHEMA_VERSION, "reader_generation": PROFILE_READER_GENERATION,
        "profile_marker_only": marker(paths), "collection_enabled":state.enabled,
        "history_scope":"all_worlds_local_process_observations_not_selected_world",
        "retained_count":state.events.len(), "older_events_dropped":state.older_events_dropped,
        "best_effort_incomplete_possible":true, "periodic_samples_not_totals":true,
        "summary":summarize(&state.events), "events":state.events,
        "selected_world_observation":selected, "authorizes_hosting_or_play":false,
        "automatically_uploaded":false
    }))
}

pub fn run(paths: &DataPaths, command: SupportCommand) -> Result<()> {
    let state = match command {
        SupportCommand::Report { confirm, world, live_route } => {
            ensure!(confirm, "explicit redacted report consent is required");
            ensure!(!live_route || world.is_some(), "live route observation requires a selected world");
            let state = load(paths)
                .map_err(|_| anyhow::anyhow!("operational support store unavailable; no raw data exported"))?;
            let result = report(paths, &state, world, live_route)?;
            let json = serde_json::to_string_pretty(&result)?;
            ensure!(json.len() <= MAX_REPORT_BYTES, "operational report exceeds output bound");
            println!("{json}");
            return Ok(());
        }
        SupportCommand::Status => load(paths),
        SupportCommand::Configure { collection } => change(paths, Some(matches!(collection, Collection::On)), false),
        SupportCommand::Clear { confirm } => {
            if !confirm {
                bail!("explicit clear confirmation required");
            }
            change(paths, None, true)
        }
    }
    .map_err(|_| anyhow::anyhow!("operational support store unavailable or busy; no raw data exported"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "collection_enabled":state.enabled, "retained_count":state.events.len(),
            "older_events_dropped":state.older_events_dropped,
            "restart_long_running_sidecars_after_collection_change":true, "automatically_uploaded":false
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_readiness::{CheckpointProgress, ReplicaCoverage};
    use swarm_network::game_transport::GameSessionDiagnosticState;

    fn paths(temp: &tempfile::TempDir) -> DataPaths {
        DataPaths::from_root(temp.path().join("profile"))
    }

    fn event(metric: Metric, outcome: Outcome) -> Event {
        Event::new(metric, outcome, Some(Instant::now()), None).unwrap()
    }

    #[test]
    fn disabled_report_and_measurement_do_not_initialize_a_profile() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let state = load(&paths).unwrap();
        assert!(!state.enabled);
        let outcome = measure(&paths, Metric::CheckpointRestore, || Err::<(), _>("original error"));
        assert_eq!(outcome, Err("original error"));
        let report = report(&paths, &state, None, false).unwrap();
        assert_eq!(report["retained_count"], 0);
        assert_eq!(report["automatically_uploaded"], false);
        assert_eq!(report["profile_marker_only"], "missing");
        assert!(!paths.root.exists());
    }

    #[test]
    fn bounded_retention_summary_and_revision_barrier() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let first = change(&paths, Some(true), false).unwrap();
        for index in 0..MAX_EVENTS + 5 {
            let mut observation = event(Metric::CheckpointVerify, Outcome::Completed);
            observation.elapsed_ms = Some(index as u64);
            assert!(append(&paths, first.revision, [observation]).unwrap());
        }
        let state = load(&paths).unwrap();
        assert_eq!(state.events.len(), MAX_EVENTS);
        assert!(state.older_events_dropped);
        assert_eq!(state.events.front().unwrap().elapsed_ms, Some(5));
        let summary = summarize(&state.events);
        assert_eq!(summary[&Metric::CheckpointVerify].timed_samples, MAX_EVENTS);
        assert_eq!(summary[&Metric::CheckpointVerify].p50_elapsed_ms, Some(68));
        assert_eq!(summary[&Metric::CheckpointVerify].p95_elapsed_ms, Some(126));
        let paused = change(&paths, Some(false), false).unwrap();
        assert_eq!(paused.events.len(), MAX_EVENTS);
        assert!(!append(&paths, first.revision, [event(Metric::CheckpointVerify, Outcome::Failed)]).unwrap());
        let enabled = change(&paths, Some(true), false).unwrap();
        assert!(!append(&paths, first.revision, [event(Metric::CheckpointVerify, Outcome::Failed)]).unwrap());
        change(&paths, None, true).unwrap();
        assert!(!append(&paths, enabled.revision, [event(Metric::CheckpointVerify, Outcome::Failed)]).unwrap());
        assert!(load(&paths).unwrap().events.is_empty());
    }

    #[test]
    fn malformed_busy_and_incompatible_stores_cannot_change_operation_result() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let state = change(&paths, Some(true), false).unwrap();
        let state_path = paths.root.join(STATE_FILE);
        let before = fs::read(&state_path).unwrap();
        let guard = lock(&paths, false).unwrap();
        let returned = measure(&paths, Metric::CheckpointRestore, || Ok::<_, &str>(42));
        assert_eq!(returned, Ok(42));
        assert_eq!(fs::read(&state_path).unwrap(), before);
        drop(guard);
        let marker_file = paths.root.join("profile-compatibility.json");
        fs::write(
            &marker_file,
            br#"{"format_schema":1,"minimum_reader_generation":2,"storage_schema_version":1,"protocol_version":1}"#,
        )
        .unwrap();
        assert!(!append(&paths, state.revision, [event(Metric::CheckpointVerify, Outcome::Completed)]).unwrap());
        assert!(change(&paths, Some(false), false).is_err());
        assert_eq!(fs::read(&state_path).unwrap(), before);
        assert_eq!(report(&paths, &load(&paths).unwrap(), None, false).unwrap()["profile_marker_only"], "incompatible");
        fs::remove_file(marker_file).unwrap();
        fs::write(&state_path, b"secret raw log data").unwrap();
        assert!(load(&paths).is_err());
        assert_eq!(measure(&paths, Metric::CheckpointRestore, || Ok::<_, ()>(7)), Ok(7));
        assert_eq!(fs::read(&state_path).unwrap(), b"secret raw log data");
        fs::write(&state_path, vec![b'x'; MAX_STORE_BYTES as usize + 1]).unwrap();
        assert!(load(&paths).is_err());
    }

    #[test]
    fn typed_report_excludes_raw_control_records_and_drops_stale_or_mismatched_world_cache() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let world = WorldId([37; 32]);
        let other = WorldId([38; 32]);
        let dir = paths.root.join("control").join(world.to_hex());
        create_private_dir_all(&dir).unwrap();
        let time = now_ms().unwrap();
        let mut cached = HostReadinessReport {
            world_id: world.to_string(),
            generated_unix_ms: time,
            local_peer_id: "PRIVATE_PEER_CANARY".into(),
            current_host_peer_id: Some("PRIVATE_HOST_CANARY".into()),
            state: HostReadinessState::Syncing,
            safe_to_shutdown: false,
            successor_peer_id: None,
            handoff_candidate_peer_id: None,
            world_data_replicated: false,
            detail: "PRIVATE_INVITE_TOKEN 192.0.2.12 /secret/user/world".into(),
            peers: vec![],
            checkpoint: Some(CheckpointProgress {
                snapshot_number: 9,
                manifest_hash: "PRIVATE_CHECKPOINT_CANARY".into(),
                logical_bytes: Some(123),
                last_replica_ack: None,
                replica_coverage: Some(ReplicaCoverage {
                    membership_hash: "PRIVATE_MEMBERSHIP_CANARY".into(),
                    epoch_hash: "PRIVATE_EPOCH_CANARY".into(),
                    preferred_total_copies: 3,
                    connected_matching_status_peers: vec!["PRIVATE_PEER_CANARY".into()],
                    connected_exact_receipt_peers: vec!["PRIVATE_HOST_CANARY".into()],
                }),
            }),
        };
        let status = MigrationStatus {
            world_id: world.to_string(),
            authority_peer_id: Some("PRIVATE_PEER_CANARY".into()),
            epoch: Some(1),
            fencing_token: Some(2),
            trigger: None,
            phase: MigrationPhase::Failed,
            runtime_ready: false,
            game_endpoint: Some("127.0.0.1:25777".into()),
            snapshot_hash: Some("PRIVATE_CHECKPOINT_CANARY".into()),
            failure_reason: Some("SECRET_CREDENTIAL_CANARY".into()),
        };
        let readiness_path = dir.join("host-readiness.json");
        fs::write(&readiness_path, serde_json::to_vec(&cached).unwrap()).unwrap();
        fs::write(dir.join("migration-status.json"), serde_json::to_vec(&status).unwrap()).unwrap();
        let data = report(&paths, &Store::default(), Some(world), false).unwrap();
        let encoded = serde_json::to_string(&data).unwrap();
        for secret in [
            "PRIVATE_",
            "SECRET_CREDENTIAL",
            "192.0.2.12",
            "/secret/user/world",
            "127.0.0.1:25777",
            &world.to_string(),
            &world.to_hex(),
        ] {
            assert!(!encoded.contains(secret), "redaction failed: {secret}");
        }
        let observed = &data["selected_world_observation"];
        assert_eq!(observed["readiness_cache"]["state"], "syncing");
        assert_eq!(observed["readiness_cache"]["exact_replica_receipt_count"], 1);
        assert_eq!(observed["migration_record"]["last_recorded_phase"], "failed");
        assert_eq!(observed["migration_record"]["recorded_failure"], true);
        assert_eq!(observed["migration_record"]["historical_only"], true);
        cached.generated_unix_ms = time - HOST_READINESS_MAX_AGE_MS - 1;
        fs::write(&readiness_path, serde_json::to_vec(&cached).unwrap()).unwrap();
        let old = readiness(&paths, world, time);
        assert!(
            matches!(old.availability, Availability::Stale)
                && old.state.is_none()
                && old.exact_replica_receipt_count.is_none()
        );
        cached.generated_unix_ms = time + 1;
        fs::write(&readiness_path, serde_json::to_vec(&cached).unwrap()).unwrap();
        assert!(matches!(readiness(&paths, world, time).availability, Availability::Unavailable));
        cached.generated_unix_ms = time;
        cached.world_id = other.to_string();
        fs::write(&readiness_path, serde_json::to_vec(&cached).unwrap()).unwrap();
        assert!(matches!(readiness(&paths, world, time).availability, Availability::Unavailable));
    }

    #[test]
    fn route_projection_is_expiry_bound_and_never_copies_gateway_or_identifiers() {
        let world = WorldId([42; 32]);
        let mut route = GameRouteStatusV1::unavailable_with_reason(world, GameRouteDiagnostic::HostProofUnavailable);
        route.recent_session =
            Some(GameSessionDiagnostic { state: GameSessionDiagnosticState::RouteRefused, age_ms: 1 });
        let value = serde_json::to_value(project_route(&route, world, 100)).unwrap();
        assert_eq!(value["outcome"], "host_proof_unavailable");
        assert_eq!(value["recent_session"]["state"], "route_refused");
        assert_eq!(value["proof_fresh_at_observation"], false);
        route.recent_session.as_mut().unwrap().age_ms = GAME_SESSION_DIAGNOSTIC_MAX_AGE_MS;
        assert!(project_route(&route, world, 100).recent_session.is_none());
        route.route_verified = true;
        route.quorum_verified = true;
        route.hosting_consent_verified = true;
        route.server_ready = true;
        route.state = "verified".into();
        route.diagnostic = None;
        route.epoch = Some(1);
        route.fencing_token = Some(1);
        route.authority_peer_id = Some("PRIVATE_AUTHORITY".into());
        route.snapshot_hash = Some("PRIVATE_CHECKPOINT".into());
        route.local_gateway = Some("127.0.0.1:25777".into());
        route.verified_at_unix_ms = Some(99);
        route.valid_until_unix_ms = Some(101);
        let fresh = project_route(&route, world, 100);
        assert!(fresh.proof_fresh_at_observation);
        let encoded = serde_json::to_string(&fresh).unwrap();
        assert!(!encoded.contains("PRIVATE_") && !encoded.contains("25777"));
        let expired = project_route(&route, world, 101);
        assert!(matches!(expired.availability, Availability::Stale) && expired.outcome.is_none());
        assert!(!project_route(&route, WorldId([43; 32]), 100).proof_fresh_at_observation);
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_symlinked_store_and_control_file_are_refused() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        create_private_dir_all(&paths.root).unwrap();
        let secret = temp.path().join("secret");
        fs::write(&secret, b"private canary").unwrap();
        symlink(&secret, paths.root.join(STATE_FILE)).unwrap();
        assert!(load(&paths).is_err());
        assert!(change(&paths, Some(true), false).is_err());
        let world = WorldId([3; 32]);
        let dir = paths.root.join("control").join(world.to_hex());
        create_private_dir_all(&dir).unwrap();
        symlink(&secret, dir.join("migration-status.json")).unwrap();
        assert!(matches!(migration_observation(&paths, world).availability, Availability::Unavailable));
        assert_eq!(fs::read(&secret).unwrap(), b"private canary");
    }
}
