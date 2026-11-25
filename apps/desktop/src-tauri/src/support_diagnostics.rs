//! Opt-in local action history and allow-listed diagnostic export.
//! Never serialize CLI arguments, stdout/stderr, error text, environment,
//! world names, save data, peer addresses, credentials or raw logs.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use swarm_core::DataPaths;
use swarm_protocol::WorldId;
use tauri::{State, WebviewWindow};

const VERSION: u16 = 1;
const MAX_EVENTS: usize = 256;
const MAX_STORE_BYTES: u64 = 256 * 1024;
const PREVIEW_LIFETIME: Duration = Duration::from_secs(300);
const STORE_ERROR: &str = "Local support history is unavailable. No raw diagnostics were exported.";
static MISSED_EVENTS: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    CreateWorld,
    ImportWorld,
    JoinRequest,
    LeaveRequest,
    CreateInvite,
    RevokeInvite,
    VerifyReplica,
    ReplicaPreference,
    HostingPreference,
    ConfigureRuntime,
    InstallRuntime,
    RepairRuntime,
    VerifyRuntime,
    Play,
    StopWorld,
    WakeRequest,
    TransferRequest,
    TransferStep,
    RecoveryCopy,
    ExportCheckpoint,
    RecoverCheckpoint,
    ExportConflict,
    ModAcquire,
    ModAdd,
    ModRemove,
    PublicationPreference,
    StartNetworking,
    StopNetworking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    ReturnedOk,
    ReturnedError,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathClass {
    Direct,
    Relay,
    DirectAndRelay,
    Limited,
    Unknown,
    Unavailable,
}

/// This is a coarse *last reported node path*, not a current game route,
/// reachability guarantee, membership proof or authorization.
pub fn classify_path(bytes: &[u8]) -> PathClass {
    let Ok(raw) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return PathClass::Unavailable;
    };
    let (Some(state), Some(direct), Some(relay)) = (
        raw.get("state").and_then(|v| v.as_str()),
        raw.get("direct_connectivity").and_then(|v| v.as_bool()),
        raw.get("relay_connectivity").and_then(|v| v.as_bool()),
    ) else {
        return PathClass::Unavailable;
    };
    match (state, direct, relay) {
        ("direct_reachable" | "hole_punched", true, true) | ("relay_connected", true, true) => {
            PathClass::DirectAndRelay
        }
        ("direct_reachable" | "hole_punched", true, false) => PathClass::Direct,
        ("relay_connected", false, true) => PathClass::Relay,
        ("relay_required" | "private_unreachable" | "bootstrap_unavailable" | "no_viable_path", false, false) => {
            PathClass::Limited
        }
        ("nat_status_unknown", false, false) => PathClass::Unknown,
        _ => PathClass::Unavailable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TreeState {
    Clean,
    Modified,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildStamp {
    version: Option<String>,
    commit: Option<String>,
    tree: TreeState,
}

fn valid_version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()))
}

impl BuildStamp {
    fn current() -> Self {
        let commit = env!("SWARMCRAFT_SUPPORT_COMMIT");
        let version = env!("CARGO_PKG_VERSION");
        Self {
            version: valid_version(version).then(|| version.into()),
            commit: (commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| commit.to_ascii_lowercase()),
            tree: match env!("SWARMCRAFT_SUPPORT_TREE") {
                "clean" if !commit.is_empty() => TreeState::Clean,
                "modified" => TreeState::Modified,
                _ => TreeState::Unknown,
            },
        }
    }
    fn valid(&self) -> bool {
        self.version.as_deref().is_none_or(valid_version)
            && self
                .commit
                .as_ref()
                .is_none_or(|c| c.len() == 40 && c.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            && (self.tree != TreeState::Clean || self.commit.is_some())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    unix_ms: u64,
    action: Action,
    outcome: Outcome,
    build: BuildStamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalEvent {
    // For on-device filtering only. Never included in the export projection.
    world: Option<WorldId>,
    event: Event,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    version: u16,
    enabled: bool,
    revision: u64,
    events: Vec<LocalEvent>,
    dropped: bool,
}

impl Default for History {
    fn default() -> Self {
        Self { version: VERSION, enabled: false, revision: 0, events: Vec::new(), dropped: false }
    }
}

impl History {
    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION
            || self.events.len() > MAX_EVENTS
            || self.events.iter().any(|r| r.event.unix_ms == 0 || !r.event.build.valid())
        {
            return Err(STORE_ERROR.into());
        }
        Ok(())
    }
    fn advance(&mut self) -> Result<(), String> {
        self.revision = self.revision.checked_add(1).ok_or(STORE_ERROR)?;
        Ok(())
    }
}

fn now_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| d.as_millis().try_into().ok())
        .filter(|t| *t > 0)
        .ok_or_else(|| "System time is unavailable".into())
}

fn history_path(paths: &DataPaths) -> PathBuf {
    paths.root.join("support-history-v1.json")
}

fn regular_or_missing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err(STORE_ERROR.into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(STORE_ERROR.into()),
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn lock(paths: &DataPaths) -> Result<File, String> {
    swarm_core::create_private_dir_all(&paths.root).map_err(|_| STORE_ERROR)?;
    let path = paths.root.join("support-history-v1.lock");
    regular_or_missing(&path)?;
    let file = private_options().create(true).truncate(false).open(path).map_err(|_| STORE_ERROR)?;
    // Best effort and bounded. History failures must never block world safety.
    let deadline = Instant::now() + Duration::from_millis(150);
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err(STORE_ERROR.into()),
        }
    }
}

fn load_locked(paths: &DataPaths) -> Result<History, String> {
    let path = history_path(paths);
    regular_or_missing(&path)?;
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(History::default()),
        Err(_) => return Err(STORE_ERROR.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_STORE_BYTES + 1).read_to_end(&mut bytes).map_err(|_| STORE_ERROR)?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(STORE_ERROR.into());
    }
    let state: History = serde_json::from_slice(&bytes).map_err(|_| STORE_ERROR)?;
    state.validate()?;
    Ok(state)
}

fn save_locked(paths: &DataPaths, state: &History) -> Result<(), String> {
    state.validate()?;
    let bytes = serde_json::to_vec(state).map_err(|_| STORE_ERROR)?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(STORE_ERROR.into());
    }
    let target = history_path(paths);
    let temp = target.with_extension(format!("{}.tmp", hex::encode(swarm_core::random_nonce())));
    let mut file = private_options().create_new(true).open(&temp).map_err(|_| STORE_ERROR)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &target)?;
        #[cfg(unix)]
        File::open(&paths.root)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|_| STORE_ERROR.into())
}

fn paths() -> Result<DataPaths, String> {
    DataPaths::discover().map_err(|_| STORE_ERROR.into())
}

struct Pending {
    paths: DataPaths,
    revision: u64,
    world: Option<WorldId>,
    action: Action,
}

impl Pending {
    fn begin(paths: DataPaths, action: Action, world: Option<WorldId>) -> Result<Option<Self>, String> {
        // No history has ever been enabled: do not create a store or lock
        // as a side effect of normal world actions.
        match fs::symlink_metadata(history_path(&paths)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(meta) if meta.is_file() => {}
            _ => return Err(STORE_ERROR.into()),
        }
        let _guard = lock(&paths)?;
        let history = load_locked(&paths)?;
        Ok(history.enabled.then_some(Self { paths, revision: history.revision, world, action }))
    }

    fn finish(self, outcome: Outcome) -> Result<(), String> {
        let _guard = lock(&self.paths)?;
        let mut history = load_locked(&self.paths)?;
        // A prior command cannot resurrect cleared or disabled collection.
        if !history.enabled || history.revision != self.revision {
            return Ok(());
        }
        if history.events.len() == MAX_EVENTS {
            history.events.remove(0);
            history.dropped = true;
        }
        history.events.push(LocalEvent {
            world: self.world,
            event: Event { unix_ms: now_ms()?, action: self.action, outcome, build: BuildStamp::current() },
        });
        save_locked(&self.paths, &history)
    }
}

pub struct ActionGuard(Option<Pending>);

impl ActionGuard {
    pub fn begin(action: Action, world: Option<&str>) -> Self {
        // Only the validated typed world identity is retained locally.
        // A malformed/hostile string never enters the history.
        if cfg!(test) {
            return Self(None);
        }
        let world = world
            .filter(|id| matches!(id.len(), 64 | 72))
            .and_then(|id| crate::hosting_contribution_commands::validate_world_id(id).ok());
        match paths().and_then(|paths| Pending::begin(paths, action, world)) {
            Ok(pending) => Self(pending),
            Err(_) => {
                MISSED_EVENTS.store(true, Ordering::Relaxed);
                Self(None)
            }
        }
    }
    pub fn associate_created_world(&mut self, id: &str) {
        if let Some(pending) = self.0.as_mut() {
            if id.len() == 72 && id.starts_with("scworld:") {
                pending.world = crate::hosting_contribution_commands::validate_world_id(id).ok();
            }
        }
    }
    pub fn finish<T, E>(mut self, result: &Result<T, E>) {
        self.record(if result.is_ok() { Outcome::ReturnedOk } else { Outcome::ReturnedError });
    }
    fn record(&mut self, outcome: Outcome) {
        if let Some(pending) = self.0.take() {
            if pending.finish(outcome).is_err() {
                MISSED_EVENTS.store(true, Ordering::Relaxed);
            }
        }
    }
}
impl Drop for ActionGuard {
    fn drop(&mut self) {
        self.record(Outcome::Interrupted);
    }
}

/// Only allow-listed event projections cross the renderer/export boundary.
#[derive(Debug, Serialize)]
pub struct HistoryView {
    enabled: bool,
    retained_count: usize,
    older_events_dropped: bool,
    recording_may_be_incomplete: bool,
    events: Vec<Event>,
}

fn view(history: &History, world: Option<WorldId>) -> HistoryView {
    HistoryView {
        enabled: history.enabled,
        retained_count: history.events.iter().filter(|r| world.is_none() || r.world == world).count(),
        older_events_dropped: history.dropped,
        recording_may_be_incomplete: MISSED_EVENTS.load(Ordering::Relaxed),
        events: history
            .events
            .iter()
            .filter(|r| world.is_none() || r.world == world)
            .map(|r| r.event.clone())
            .collect(),
    }
}

fn scope(world: Option<String>) -> Result<Option<WorldId>, String> {
    world
        .as_deref()
        .map(|id| {
            if id.len() != 72 || !id.starts_with("scworld:") {
                return Err("A full selected World ID is required".into());
            }
            crate::hosting_contribution_commands::validate_world_id(id)
        })
        .transpose()
}

#[tauri::command]
pub fn support_history(world: Option<String>) -> Result<HistoryView, String> {
    let paths = paths()?;
    let _guard = lock(&paths)?;
    Ok(view(&load_locked(&paths)?, scope(world)?))
}

fn change_history(paths: &DataPaths, enabled: Option<bool>, clear: bool) -> Result<History, String> {
    let _guard = lock(paths)?;
    let mut history = load_locked(paths)?;
    if clear {
        history.events.clear();
        history.dropped = false;
        history.advance()?;
    }
    if let Some(enabled) = enabled {
        if history.enabled != enabled {
            history.advance()?;
            history.enabled = enabled;
        }
    }
    save_locked(paths, &history)?;
    if clear {
        MISSED_EVENTS.store(false, Ordering::Relaxed);
    }
    Ok(history)
}

#[derive(Default)]
pub struct SupportPreviewCache(Mutex<Option<Prepared>>);

impl SupportPreviewCache {
    fn invalidate(&self) -> Result<(), String> {
        self.0.lock().map_err(|_| STORE_ERROR)?.take();
        Ok(())
    }
}

#[tauri::command]
pub fn support_recording(
    cache: State<'_, SupportPreviewCache>,
    enabled: bool,
    confirm: bool,
    world: Option<String>,
) -> Result<HistoryView, String> {
    if !confirm {
        return Err("Explicit confirmation is required".into());
    }
    let world = scope(world)?;
    cache.invalidate()?;
    Ok(view(&change_history(&paths()?, Some(enabled), false)?, world))
}

#[tauri::command]
pub fn support_clear_history(
    cache: State<'_, SupportPreviewCache>,
    confirm: bool,
    world: Option<String>,
) -> Result<HistoryView, String> {
    if !confirm {
        return Err("Explicit confirmation is required".into());
    }
    let world = scope(world)?;
    cache.invalidate()?;
    Ok(view(&change_history(&paths()?, None, true)?, world))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ScopeLabel {
    AllWorlds,
    SelectedWorld,
}

#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    generated_unix_ms: u64,
    desktop_build: BuildStamp,
    last_reported_node_path: PathClass,
    scope: ScopeLabel,
    history: HistoryView,
}

struct Prepared {
    token: String,
    issued: Instant,
    revision: u64,
    filename: String,
    json: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    token: String,
    report: Report,
}

fn prepare_report(
    paths: &DataPaths,
    world: Option<WorldId>,
    path_class: PathClass,
) -> Result<(Prepared, Preview), String> {
    let _guard = lock(paths)?;
    let history = load_locked(paths)?;
    let timestamp = now_ms()?;
    let token = hex::encode(swarm_core::random_nonce());
    let report = Report {
        schema: "swarmcraft-redacted-support-v1",
        generated_unix_ms: timestamp,
        desktop_build: BuildStamp::current(),
        last_reported_node_path: path_class,
        scope: if world.is_some() { ScopeLabel::SelectedWorld } else { ScopeLabel::AllWorlds },
        history: view(&history, world),
    };
    let mut json = serde_json::to_vec_pretty(&report).map_err(|_| STORE_ERROR)?;
    json.push(b'\n');
    let prepared = Prepared {
        token: token.clone(),
        issued: Instant::now(),
        revision: history.revision,
        filename: format!("swarmcraft-support-{timestamp}-{}.json", &token[..12]),
        json,
    };
    Ok((prepared, Preview { token, report }))
}

#[tauri::command]
pub fn support_preview(
    cache: State<'_, SupportPreviewCache>,
    processes: State<'_, crate::RuntimeProcesses>,
    world: Option<String>,
    confirm: bool,
) -> Result<Preview, String> {
    if !confirm {
        return Err("Explicit preview consent is required".into());
    }
    let world = scope(world)?;
    let (prepared, preview) = prepare_report(&paths()?, world, processes.support_path_class())?;
    *cache.0.lock().map_err(|_| STORE_ERROR)? = Some(prepared);
    Ok(preview)
}

fn take_preview(cache: &SupportPreviewCache, token: &str) -> Result<Prepared, String> {
    let mut cache = cache.0.lock().map_err(|_| STORE_ERROR)?;
    if !cache.as_ref().is_some_and(|p| p.token == token && p.issued.elapsed() < PREVIEW_LIFETIME) {
        return Err("Preview expired or changed. Generate a new preview.".into());
    }
    cache.take().ok_or_else(|| STORE_ERROR.into())
}

fn write_report(paths: &DataPaths, folder: &Path, report: &Prepared) -> Result<String, String> {
    let _guard = lock(paths)?;
    if load_locked(paths)?.revision != report.revision || report.issued.elapsed() >= PREVIEW_LIFETIME {
        return Err("History changed or the preview expired. Generate a new preview.".into());
    }
    // Never overwrite an existing file. Only the native folder selection
    // supplies a destination, and the basename is generated by Rust.
    let folder = folder.canonicalize().map_err(|_| "The chosen folder is unavailable")?;
    let private_root = paths.root.canonicalize().map_err(|_| STORE_ERROR)?;
    if !folder.is_dir() || folder.starts_with(&private_root) {
        return Err("Choose an export folder outside SwarmCraft's private data directory".into());
    }
    let destination = folder.join(&report.filename);
    let mut file = private_options()
        .create_new(true)
        .open(&destination)
        .map_err(|_| "Could not create a new report in the chosen folder".to_owned())?;
    let result = file.write_all(&report.json).and_then(|_| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(&destination);
        return Err("Report could not be saved".into());
    }
    Ok(report.filename.clone())
}

#[tauri::command]
pub async fn support_save(
    window: WebviewWindow,
    cache: State<'_, SupportPreviewCache>,
    token: String,
    confirm: bool,
) -> Result<Option<String>, String> {
    if !confirm {
        return Err("Explicit export consent is required".into());
    }
    let report = take_preview(&cache, &token)?;
    let folder = crate::import_picker::choose_folder(window, "Choose folder for redacted support report").await?;
    folder.map(|folder| write_report(&paths()?, &folder, &report)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("swarmcraft-support-test-{}", hex::encode(swarm_core::random_nonce())));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn paths(&self) -> DataPaths {
            DataPaths::from_root(&self.0)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn consent_is_off_and_pending_cannot_cross_clear_or_disable() {
        let dir = TestDir::new();
        let paths = dir.paths();
        assert!(Pending::begin(paths.clone(), Action::Play, None).unwrap().is_none());
        change_history(&paths, Some(true), false).unwrap();
        let pending = Pending::begin(paths.clone(), Action::Play, None).unwrap().unwrap();
        change_history(&paths, Some(false), false).unwrap();
        change_history(&paths, Some(true), false).unwrap();
        pending.finish(Outcome::ReturnedOk).unwrap();
        assert!(load_locked(&paths).unwrap().events.is_empty());
        let pending = Pending::begin(paths.clone(), Action::Play, None).unwrap().unwrap();
        change_history(&paths, None, true).unwrap();
        pending.finish(Outcome::ReturnedError).unwrap();
        assert!(load_locked(&paths).unwrap().events.is_empty());
    }

    #[test]
    fn scoped_export_does_not_contain_world_ids_or_raw_data() {
        let dir = TestDir::new();
        let paths = dir.paths();
        let a = WorldId([0xab; 32]);
        let b = WorldId([0xcd; 32]);
        change_history(&paths, Some(true), false).unwrap();
        Pending::begin(paths.clone(), Action::CreateInvite, Some(a))
            .unwrap()
            .unwrap()
            .finish(Outcome::ReturnedOk)
            .unwrap();
        Pending::begin(paths.clone(), Action::RecoveryCopy, Some(b))
            .unwrap()
            .unwrap()
            .finish(Outcome::ReturnedError)
            .unwrap();
        Pending::begin(paths.clone(), Action::JoinRequest, None)
            .unwrap()
            .unwrap()
            .finish(Outcome::Interrupted)
            .unwrap();
        let (prepared, preview) = prepare_report(&paths, Some(a), classify_path(br#"{"state":"relay_connected","direct_connectivity":false,"relay_connectivity":true,"local_addresses":["/ip4/10.5.6.7"],"last_failure":"scinvite:deadbeef SECRET /home/person/world"}"#)).unwrap();
        assert_eq!(preview.report.history.events.len(), 1);
        assert_eq!(preview.report.last_reported_node_path, PathClass::Relay);
        let output = String::from_utf8(prepared.json.clone()).unwrap();
        for secret in [
            a.to_string(),
            b.to_string(),
            "abababab".into(),
            "cdcdcdcd".into(),
            "10.5.6.7".into(),
            "scinvite:".into(),
            "SECRET".into(),
            "/home/person".into(),
            "local_addresses".into(),
            "last_failure".into(),
            "recovery_copy".into(),
        ] {
            assert!(!output.contains(&secret), "unexpected exported field");
        }
        let destination = TestDir::new();
        assert!(write_report(&paths, &dir.0, &prepared).is_err());
        let filename = write_report(&paths, &destination.0, &prepared).unwrap();
        assert_eq!(fs::read(destination.0.join(filename)).unwrap(), prepared.json);
        assert!(write_report(&paths, &destination.0, &prepared).is_err());
        change_history(&paths, None, true).unwrap();
        let another = TestDir::new();
        assert!(write_report(&paths, &another.0, &prepared).is_err());
    }

    #[test]
    fn bounds_and_corrupt_store_fail_closed() {
        let dir = TestDir::new();
        let paths = dir.paths();
        let mut history = change_history(&paths, Some(true), false).unwrap();
        history.events = (0..MAX_EVENTS)
            .map(|_| LocalEvent {
                world: None,
                event: Event {
                    unix_ms: 1,
                    action: Action::Play,
                    outcome: Outcome::ReturnedOk,
                    build: BuildStamp::current(),
                },
            })
            .collect();
        save_locked(&paths, &history).unwrap();
        Pending::begin(paths.clone(), Action::StopWorld, None).unwrap().unwrap().finish(Outcome::ReturnedOk).unwrap();
        let bounded = load_locked(&paths).unwrap();
        assert_eq!(bounded.events.len(), MAX_EVENTS);
        assert!(bounded.dropped);
        assert_eq!(bounded.events.last().unwrap().event.action, Action::StopWorld);
        fs::write(
            history_path(&paths),
            br#"{"version":1,"enabled":true,"revision":1,"events":[],"dropped":false,"raw":"scinvite:secret"}"#,
        )
        .unwrap();
        assert!(prepare_report(&paths, None, PathClass::Unknown).is_err());
        assert!(change_history(&paths, Some(false), false).is_err());
    }

    #[test]
    fn concurrent_completions_preserve_distinct_events_and_scoped_count() {
        let dir = TestDir::new();
        let paths = dir.paths();
        change_history(&paths, Some(true), false).unwrap();
        let a = WorldId([1; 32]);
        let b = WorldId([2; 32]);
        let first = Pending::begin(paths.clone(), Action::Play, Some(a)).unwrap().unwrap();
        let second = Pending::begin(paths.clone(), Action::VerifyReplica, Some(b)).unwrap().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| first.finish(Outcome::ReturnedOk).unwrap());
            scope.spawn(|| second.finish(Outcome::ReturnedError).unwrap());
        });
        let history = load_locked(&paths).unwrap();
        assert_eq!(history.events.len(), 2);
        let selected = view(&history, Some(a));
        assert_eq!(selected.retained_count, 1);
        assert_eq!(selected.events[0].action, Action::Play);
        assert_eq!(selected.events[0].outcome, Outcome::ReturnedOk);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_store_cannot_be_read_or_overwritten() {
        let dir = TestDir::new();
        let paths = dir.paths();
        let other = dir.0.join("unrelated");
        fs::write(&other, b"unrelated data").unwrap();
        std::os::unix::fs::symlink(&other, history_path(&paths)).unwrap();
        assert!(load_locked(&paths).is_err());
        assert!(change_history(&paths, Some(true), false).is_err());
        assert_eq!(fs::read(&other).unwrap(), b"unrelated data");
    }

    #[test]
    fn path_projection_is_allowlisted_and_inconsistent_input_is_unavailable() {
        assert_eq!(
            classify_path(br#"{"state":"direct_reachable","direct_connectivity":true,"relay_connectivity":false}"#),
            PathClass::Direct
        );
        assert_eq!(
            classify_path(br#"{"state":"relay_connected","direct_connectivity":false,"relay_connectivity":true}"#),
            PathClass::Relay
        );
        assert_eq!(
            classify_path(br#"{"state":"direct_reachable","direct_connectivity":true,"relay_connectivity":true}"#),
            PathClass::DirectAndRelay
        );
        assert_eq!(
            classify_path(br#"{"state":"relay_required","direct_connectivity":false,"relay_connectivity":false}"#),
            PathClass::Limited
        );
        assert_eq!(
            classify_path(br#"{"state":"nat_status_unknown","direct_connectivity":false,"relay_connectivity":false}"#),
            PathClass::Unknown
        );
        assert_eq!(
            classify_path(br#"{"state":"direct_reachable","direct_connectivity":false,"relay_connectivity":false}"#),
            PathClass::Unavailable
        );
        assert_eq!(classify_path(b"scinvite:deadbeef"), PathClass::Unavailable);
    }

    #[test]
    fn preview_token_is_exact_one_time_and_expiring() {
        let dir = TestDir::new();
        let (prepared, preview) = prepare_report(&dir.paths(), None, PathClass::Unknown).unwrap();
        let cache = SupportPreviewCache(Mutex::new(Some(prepared)));
        assert!(take_preview(&cache, "invalid").is_err());
        assert!(take_preview(&cache, &preview.token).is_ok());
        assert!(take_preview(&cache, &preview.token).is_err());
        let (mut prepared, preview) = prepare_report(&dir.paths(), None, PathClass::Unknown).unwrap();
        prepared.issued = Instant::now() - PREVIEW_LIFETIME;
        *cache.0.lock().unwrap() = Some(prepared);
        assert!(take_preview(&cache, &preview.token).is_err());
    }
}
