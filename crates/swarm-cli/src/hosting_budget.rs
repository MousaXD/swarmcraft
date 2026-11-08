//! Machine-local, fail-closed hosting consent and conservative memory admission.
//! A heap limit is NOT a total JVM RSS limit. See HOSTING_BUDGET_CONTRACT.md.
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::{DataPaths, PeerIdentity};

use crate::host_readiness;
use swarm_protocol::{PeerId, WorldId};

const SCHEMA_VERSION: u16 = 1;
pub const MIN_HEAP_MIB: u64 = 512;
pub const MAX_HEAP_MIB: u64 = 65_536;
const NATIVE_MIN_MIB: u64 = 1_024;
const CLIENT_HEADROOM_MIB: u64 = 1_024;
const DAEMON_HEADROOM_MIB: u64 = 256;
const OS_MIN_HEADROOM_MIB: u64 = 1_024;

/// This is local preference, never signed world authority or a remotely trusted claim.
/// Absence = player-only (deny); invalid/unreadable records also fail closed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostingConsentV1 {
    pub schema_version: u16,
    pub world_id: WorldId,
    /// Binds local consent to the current authenticated device identity.
    pub peer_id: PeerId,
    pub enabled: bool,
    pub heap_mib: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvailableMemory {
    pub physical_mib: u64,
    pub available_mib: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributionReadiness {
    pub world_id: String,
    pub consented: bool,
    pub selected_heap_mib: u64,
    pub physical_mib: Option<u64>,
    pub available_mib: Option<u64>,
    pub estimated_native_overhead_mib: u64,
    pub client_headroom_mib: u64,
    pub os_headroom_mib: u64,
    pub daemon_headroom_mib: u64,
    pub may_host_now: bool,
    pub reason: String,
    /// Heap-only JVM configuration, never a guaranteed whole-process RSS cap.
    pub hard_process_memory_cap_guaranteed: bool,
}

/// The process-wide allocation lock intentionally permits at most ONE active
/// Minecraft authority JVM per data root. It is retained from admission until
/// the Java process exits; this conservative policy prevents concurrent
/// launches from racing a single available-memory reading.
pub struct HostingAdmission {
    _exclusive_memory_lock: File,
    consent: HostingConsentV1,
}

impl HostingAdmission {
    pub fn heap_mib(&self) -> u64 {
        self.consent.heap_mib
    }

    pub fn consent(&self) -> &HostingConsentV1 {
        &self.consent
    }
}

fn prefs_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("control").join(world.to_hex()).join("hosting-consent.json")
}

fn control_path(paths: &DataPaths) -> PathBuf {
    paths.root.join("control")
}

fn open_lock(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("cannot open hosting lock {}", path.display()))
}

fn verify_record(world: WorldId, peer: PeerId, value: &HostingConsentV1) -> Result<()> {
    if value.schema_version != SCHEMA_VERSION || value.world_id != world || value.peer_id != peer {
        bail!("hosting consent schema/world/peer identity mismatch; hosting disabled");
    }
    if (!value.enabled && value.heap_mib != 0)
        || (value.enabled && !(MIN_HEAP_MIB..=MAX_HEAP_MIB).contains(&value.heap_mib))
    {
        bail!("invalid hosting consent or heap budget; hosting disabled");
    }
    if value.revision == 0 {
        bail!("missing hosting consent revision; hosting disabled");
    }
    Ok(())
}

/// Returns None only for an absent record. Corrupt, inaccessible or mismatched
/// records are errors, never converted into implicit consent.
pub fn load_consent(paths: &DataPaths, world: WorldId) -> Result<Option<HostingConsentV1>> {
    let path = prefs_path(paths, world);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read hosting consent {}", path.display())),
    };
    let consent: HostingConsentV1 = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid local hosting consent {}; refusing hosting", path.display()))?;
    // Replacing or repairing a local identity must never silently reenable
    // consent granted to a different cryptographic peer.
    let peer = PeerIdentity::load_or_create(paths)?.peer_id();
    verify_record(world, peer, &consent)?;
    Ok(Some(consent))
}

/// Called only from an explicit local user opt-in/opt-out action. No world
/// creation, joining, daemon startup, or runtime setup calls this automatically.
/// Choosing zero explicitly revokes permission and invalidates in-flight admissions.
pub fn set_contribution(paths: &DataPaths, world: WorldId, heap_mib: u64) -> Result<HostingConsentV1> {
    if heap_mib != 0 {
        // The user's durable preference may be configured while other apps
        // are consuming RAM. Check the hard physical limit now; every actual
        // launch still checks CURRENT available headroom under admission lock.
        let memory = memory_snapshot()?;
        check_budget(
            heap_mib,
            AvailableMemory { physical_mib: memory.physical_mib, available_mib: memory.physical_mib },
        )?;
    }
    let lock = open_lock(&control_path(paths).join("hosting-consent-write.lock"))?;
    lock.lock_exclusive().context("cannot serialize hosting consent updates")?;
    let previous = load_consent(paths, world)?;
    let now_nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let next_revision = previous
        .map_or(0, |record| record.revision)
        .checked_add(1)
        .context("hosting consent revision exhausted; refusing stale update")?;
    let current = (now_nanos.min(u64::MAX as u128) as u64).max(next_revision);
    let value = HostingConsentV1 {
        schema_version: SCHEMA_VERSION,
        world_id: world,
        peer_id: PeerIdentity::load_or_create(paths)?.peer_id(),
        enabled: heap_mib != 0,
        heap_mib,
        revision: current,
    };
    // Invalidate stale runtime/candidate reports BEFORE the consent revision
    // is published, for both opt-in and revocation. If publication fails, an
    // old runtime proof still cannot be silently reused.
    host_readiness::invalidate_runtime_verification(paths, world)?;
    host_readiness::invalidate_cached_readiness(paths, world)?;
    let path = prefs_path(paths, world);
    atomic_write(&path, &value)?;
    // File lock releases when dropped.
    drop(lock);
    Ok(value)
}

fn atomic_write(path: &Path, value: &HostingConsentV1) -> Result<()> {
    let dir = path.parent().context("hosting consent path has no parent")?;
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("json.{}.{}.tmp", std::process::id(), value.revision));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        #[cfg(windows)]
        if path.exists() {
            // The write lock prevents cooperating writers from racing; the
            // absence interval fails closed for readers.
            fs::remove_file(path)?;
        }
        fs::rename(&tmp, path).context("cannot atomically publish hosting consent")?;
        #[cfg(unix)]
        File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn estimated_native_overhead_mib(heap_mib: u64) -> u64 {
    if heap_mib == 0 {
        0
    } else {
        NATIVE_MIN_MIB.max(heap_mib / 2)
    }
}

fn os_headroom_mib(physical_mib: u64) -> u64 {
    OS_MIN_HEADROOM_MIB.max(physical_mib / 8)
}

fn required_mib(heap_mib: u64, physical_mib: u64) -> Result<u64> {
    heap_mib
        .checked_add(estimated_native_overhead_mib(heap_mib))
        .and_then(|n| n.checked_add(CLIENT_HEADROOM_MIB))
        .and_then(|n| n.checked_add(DAEMON_HEADROOM_MIB))
        .and_then(|n| n.checked_add(os_headroom_mib(physical_mib)))
        .context("hosting budget arithmetic overflow")
}

pub fn check_budget(heap_mib: u64, memory: AvailableMemory) -> Result<()> {
    if !(MIN_HEAP_MIB..=MAX_HEAP_MIB).contains(&heap_mib) {
        bail!("server heap must be between {MIN_HEAP_MIB} and {MAX_HEAP_MIB} MiB (0 disables hosting)");
    }
    if memory.physical_mib == 0 || memory.available_mib == 0 || memory.available_mib > memory.physical_mib {
        bail!("cannot safely establish physical/available device memory");
    }
    let required = required_mib(heap_mib, memory.physical_mib)?;
    if memory.physical_mib < required || memory.available_mib < required {
        bail!(
            "insufficient hosting memory: selected heap {heap_mib} MiB plus native JVM, Minecraft client, daemon and OS reserves requires {required} MiB; device has {} MiB physical, {} MiB currently available",
            memory.physical_mib,
            memory.available_mib
        );
    }
    Ok(())
}

pub fn require_contribution(paths: &DataPaths, world: WorldId) -> Result<HostingConsentV1> {
    let record =
        load_consent(paths, world)?.context("hosting consent is absent; this device is player-only (0 MiB)")?;
    if !record.enabled {
        bail!("hosting consent was withdrawn; this device is player-only (0 MiB)");
    }
    check_budget(record.heap_mib, memory_snapshot()?)?;
    Ok(record)
}

/// Acquires the capacity lock BEFORE rereading consent/memory and before any
/// authority epoch mutation, runtime reset, restore, or Java spawn.
pub fn admit(paths: &DataPaths, world: WorldId) -> Result<HostingAdmission> {
    let _ = require_contribution(paths, world)?;
    let lock = open_lock(&control_path(paths).join("hosting-active-memory.lock"))?;
    lock.try_lock_exclusive().context(
        "another Minecraft hosting admission is active on this device; concurrent memory reservations are denied",
    )?;
    let consent = require_contribution(paths, world)?;
    Ok(HostingAdmission { _exclusive_memory_lock: lock, consent })
}

/// Recheck persisted consent and free headroom immediately before spawn,
/// not only before a potentially slow canonical restore.
pub fn revalidate_admission(paths: &DataPaths, world: WorldId, admission: &HostingAdmission) -> Result<()> {
    let current = load_consent(paths, world)?.context("hosting consent disappeared during runtime preparation")?;
    if current != *admission.consent() {
        bail!("hosting consent or heap changed during runtime preparation; Java launch denied");
    }
    check_budget(admission.heap_mib(), memory_snapshot()?)
}

/// Called while serving. Any revision change, revocation or inability to
/// inspect state requires a safe Fabric shutdown/checkpoint, not a JVM kill.
pub fn runtime_withdrawal(paths: &DataPaths, world: WorldId, admitted: &HostingConsentV1) -> Result<Option<String>> {
    let current = load_consent(paths, world)?;
    if current.as_ref() != Some(admitted) {
        return Ok(Some("hosting contribution changed or was withdrawn".into()));
    }
    let memory = memory_snapshot()?;
    if memory.available_mib < os_headroom_mib(memory.physical_mib) {
        return Ok(Some("system memory pressure exceeded the reserved OS headroom".into()));
    }
    Ok(None)
}

pub fn contribution_readiness(paths: &DataPaths, world: WorldId) -> Result<ContributionReadiness> {
    let consent = load_consent(paths, world)?;
    let heap = consent.as_ref().filter(|v| v.enabled).map_or(0, |v| v.heap_mib);
    let memory = memory_snapshot();
    let (physical_mib, available_mib) = match &memory {
        Ok(mem) => (Some(mem.physical_mib), Some(mem.available_mib)),
        Err(_) => (None, None),
    };
    let (may_host_now, reason) = if heap == 0 {
        (false, "Player-only; no Minecraft hosting consent".to_string())
    } else {
        match memory {
            Ok(mem) => match check_budget(heap, mem) {
                Ok(()) => (
                    true,
                    "Consented and passes current conservative memory admission; must be rechecked at launch".into(),
                ),
                Err(error) => (false, error.to_string()),
            },
            Err(error) => (false, format!("device memory is not measurable; hosting blocked: {error}")),
        }
    };
    Ok(ContributionReadiness {
        world_id: world.to_string(),
        consented: heap != 0,
        selected_heap_mib: heap,
        physical_mib,
        available_mib,
        estimated_native_overhead_mib: estimated_native_overhead_mib(heap),
        client_headroom_mib: CLIENT_HEADROOM_MIB,
        os_headroom_mib: physical_mib.map_or(OS_MIN_HEADROOM_MIB, os_headroom_mib),
        daemon_headroom_mib: DAEMON_HEADROOM_MIB,
        may_host_now,
        reason,
        hard_process_memory_cap_guaranteed: false,
    })
}

#[cfg(target_os = "linux")]
pub fn memory_snapshot() -> Result<AvailableMemory> {
    let text = fs::read_to_string("/proc/meminfo").context("cannot read Linux /proc/meminfo")?;
    fn mib(text: &str, name: &str) -> Result<u64> {
        let line = text
            .lines()
            .find(|line| line.starts_with(name))
            .with_context(|| format!("missing Linux {name} memory measurement"))?;
        let kb: u64 = line.split_whitespace().nth(1).context("missing KiB count")?.parse()?;
        Ok(kb / 1_024)
    }
    let mut physical = mib(&text, "MemTotal:")?;
    let mut available = mib(&text, "MemAvailable:")?;
    let max_file = Path::new("/sys/fs/cgroup/memory.max");
    let current_file = Path::new("/sys/fs/cgroup/memory.current");
    if max_file.exists() {
        let raw = fs::read_to_string(max_file).context("cannot read cgroup memory.max")?;
        let raw = raw.trim();
        if raw != "max" {
            let limit_bytes: u64 = raw.parse().context("invalid cgroup memory.max")?;
            let used_bytes: u64 =
                fs::read_to_string(current_file).context("cannot read cgroup memory.current")?.trim().parse()?;
            let limit_mib = limit_bytes / (1024 * 1024);
            physical = physical.min(limit_mib);
            available = available.min(limit_bytes.saturating_sub(used_bytes) / (1024 * 1024));
        }
    }
    Ok(AvailableMemory { physical_mib: physical, available_mib: available.min(physical) })
}

#[cfg(windows)]
pub fn memory_snapshot() -> Result<AvailableMemory> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_pagefile: u64,
        available_pagefile: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    }
    let mut status = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        load: 0,
        total_physical: 0,
        available_physical: 0,
        total_pagefile: 0,
        available_pagefile: 0,
        total_virtual: 0,
        available_virtual: 0,
        available_extended_virtual: 0,
    };
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot determine Windows physical memory");
    }
    Ok(AvailableMemory {
        physical_mib: status.total_physical / (1024 * 1024),
        available_mib: status.available_physical / (1024 * 1024),
    })
}

#[cfg(target_os = "macos")]
pub fn memory_snapshot() -> Result<AvailableMemory> {
    use std::process::Command;
    let total = Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .context("cannot query macOS physical memory")?;
    if !total.status.success() {
        bail!("macOS physical memory query failed");
    }
    let physical_bytes: u64 = String::from_utf8(total.stdout)?.trim().parse()?;
    let stats = Command::new("/usr/bin/vm_stat").output().context("cannot query macOS free memory")?;
    if !stats.status.success() {
        bail!("macOS available memory query failed");
    }
    let stdout = String::from_utf8(stats.stdout)?;
    let header = stdout.lines().next().context("empty vm_stat output")?;
    let page_size: u64 = header
        .split("page size of ")
        .nth(1)
        .and_then(|part| part.split_whitespace().next())
        .context("missing vm_stat page size")?
        .parse()?;
    let pages = |name: &str| -> Result<u64> {
        let line = stdout.lines().find(|line| line.starts_with(name)).context("vm_stat field missing")?;
        Ok(line.split(':').nth(1).context("vm_stat value missing")?.trim().trim_end_matches('.').parse()?)
    };
    // Deliberately excludes purgeable/compressed memory to avoid optimistic reuse.
    let free = pages("Pages free:")?.saturating_add(pages("Pages inactive:")?);
    let physical = physical_bytes / (1024 * 1024);
    Ok(AvailableMemory {
        physical_mib: physical,
        available_mib: free.saturating_mul(page_size).checked_div(1024 * 1024).unwrap_or(0).min(physical),
    })
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub fn memory_snapshot() -> Result<AvailableMemory> {
    Err(anyhow::anyhow!("hosting memory admission unsupported on this operating system"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn memory(total: u64, available: u64) -> AvailableMemory {
        AvailableMemory { physical_mib: total, available_mib: available }
    }
    #[test]
    fn zero_is_not_a_host_budget() {
        assert!(check_budget(0, memory(32_768, 20_000)).is_err());
        assert!(check_budget(4_096, memory(32_768, 20_000)).is_ok());
        assert!(check_budget(4_096, memory(8_192, 8_192)).is_err());
        assert!(check_budget(1_024, memory(8_192, 700)).is_err());
    }
    #[test]
    fn overflow_and_unknown_memory_fail_closed() {
        assert!(check_budget(u64::MAX, memory(32_768, 30_000)).is_err());
        assert!(check_budget(512, memory(0, 0)).is_err());
        assert!(check_budget(512, memory(4_000, 5_000)).is_err());
    }
    #[test]
    fn defaults_to_player_only_and_revocation_persists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let world = WorldId([31; 32]);
        assert!(load_consent(&paths, world).unwrap().is_none());
        assert!(!contribution_readiness(&paths, world).unwrap().may_host_now);
        let revoked = set_contribution(&paths, world, 0).unwrap();
        assert!(!revoked.enabled);
        assert_eq!(load_consent(&paths, world).unwrap(), Some(revoked));
        assert!(require_contribution(&paths, world).is_err());
    }
    #[test]
    fn exhausted_consent_revision_never_wraps_or_reuses_previous_authority() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let world = WorldId([35; 32]);
        let peer_id = PeerIdentity::load_or_create(&paths).unwrap().peer_id();
        let previous = HostingConsentV1 {
            schema_version: SCHEMA_VERSION,
            world_id: world,
            peer_id,
            enabled: false,
            heap_mib: 0,
            revision: u64::MAX,
        };
        atomic_write(&prefs_path(&paths, world), &previous).unwrap();
        let error = set_contribution(&paths, world, 0).unwrap_err();
        assert!(error.to_string().contains("revision exhausted"));
        assert_eq!(load_consent(&paths, world).unwrap(), Some(previous));
    }

    #[test]
    fn consent_is_scoped_to_the_current_peer_identity() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let world = WorldId([34; 32]);
        let current_peer = PeerIdentity::load_or_create(&paths).unwrap().peer_id();
        let consent = HostingConsentV1 {
            schema_version: SCHEMA_VERSION,
            world_id: world,
            peer_id: PeerId([0xFF; 32]),
            enabled: true,
            heap_mib: 512,
            revision: 1,
        };
        assert_ne!(consent.peer_id, current_peer);
        atomic_write(&prefs_path(&paths, world), &consent).unwrap();
        let error = load_consent(&paths, world).unwrap_err();
        assert!(error.to_string().contains("identity mismatch"));
        assert!(admit(&paths, world).is_err());
    }

    #[test]
    fn malformed_record_never_becomes_consent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let world = WorldId([33; 32]);
        let path = prefs_path(&paths, world);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{ definitely not json").unwrap();
        assert!(load_consent(&paths, world).is_err());
        assert!(admit(&paths, world).is_err());
    }
    #[test]
    fn opt_out_invalidates_an_active_admission_generation() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let world = WorldId([37; 32]);
        let first = set_contribution(&paths, world, 512).unwrap();
        // The consent revision check is deterministic regardless of free RAM
        // on a heavily loaded CI machine. A real admit() must still reject
        // insufficient free RAM, which is tested in check_budget().
        let lock = open_lock(&control_path(&paths).join("hosting-active-memory.lock")).unwrap();
        lock.try_lock_exclusive().unwrap();
        let admission = HostingAdmission { _exclusive_memory_lock: lock, consent: first.clone() };
        assert_eq!(first, *admission.consent());
        set_contribution(&paths, world, 0).unwrap();
        assert!(revalidate_admission(&paths, world, &admission).is_err());
        assert!(runtime_withdrawal(&paths, world, admission.consent()).unwrap().is_some());
        drop(admission);
        assert!(admit(&paths, world).is_err());
    }
    #[test]
    fn concurrent_host_admission_is_serialized() {
        // Exercises a live file lock without requiring Java or a synthetic memory reading.
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let lock_path = control_path(&paths).join("hosting-active-memory.lock");
        let first = open_lock(&lock_path).unwrap();
        first.try_lock_exclusive().unwrap();
        let second = open_lock(&lock_path).unwrap();
        assert!(second.try_lock_exclusive().is_err());
    }
}
