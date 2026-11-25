//! Durable process ownership. A kernel lock alone cannot account for a JVM
//! left behind by a hard-killed supervisor. Linux records bind the PID to
//! its machine, boot, PID namespace and process birth time; legacy and
//! unsupported-platform records keep the conservative PID-only policy.
use crate::{
    host_memory_cap::{HostMemoryCap, MemoryCapIdentity, ProcessCpuReport, ProcessMemoryReport},
    machine_hosting::MachineHostingReservation,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use swarm_core::DataPaths;
use swarm_protocol::WorldId;

const LEGACY_RECORD_VERSION: u16 = 1;
const LINUX_RECORD_VERSION: u16 = 2;
const MAX_RECORD_BYTES: u64 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RuntimeProcessRecord {
    version: u16,
    controller_pid: u32,
    java_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linux: Option<LinuxRuntimeIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LinuxRuntimeIdentity {
    machine_id: String,
    boot_id: String,
    pid_namespace: u64,
    controller_start_ticks: u64,
    java_start_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cgroup: Option<MemoryCapIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ownership {
    Quiescent,
    Pending,
    #[cfg(target_os = "linux")]
    ResidualProcesses,
    Live(u32),
}

/// The account reservation outlives this guard, so both durable records
/// are published and retired while its stable kernel lock is still owned.
pub struct RuntimeProcessGuard<'a> {
    path: PathBuf,
    record: RuntimeProcessRecord,
    reservation: Option<&'a MachineHostingReservation>,
    memory_cap: Option<HostMemoryCap>,
}

impl<'a> RuntimeProcessGuard<'a> {
    #[cfg(test)]
    fn begin(paths: &DataPaths, world: WorldId) -> Result<Self> {
        Self::begin_coordinated(paths, world, None, None)
    }

    pub(crate) fn begin_coordinated(
        paths: &DataPaths,
        world: WorldId,
        reservation: Option<&'a MachineHostingReservation>,
        mut memory_cap: Option<HostMemoryCap>,
    ) -> Result<Self> {
        let path = record_path(paths, world);
        clear_quiescent_record(&path)?;
        let mut record = RuntimeProcessRecord::pending()?;
        if let Some(cap) = memory_cap.as_mut() {
            record.linux.as_mut().context("missing Linux process identity")?.cgroup = Some(cap.identity().clone());
            cap.claim_by_record();
        }
        let guard = Self { path, record, reservation, memory_cap };
        // The shared record is durable before the local record and before
        // any spawn. A crash between either write cannot create an
        // unaccounted child in a second data root.
        if let Some(shared) = reservation {
            atomic_write(&shared.record_path, &guard.record)?;
        }
        atomic_write(&guard.path, &guard.record)?;
        Ok(guard)
    }

    pub(crate) fn memory_cap(&self) -> Option<&HostMemoryCap> {
        self.memory_cap.as_ref()
    }

    pub(crate) fn verify_exit_quiescent(&self) -> Result<()> {
        if let Some(cap) = &self.memory_cap {
            anyhow::ensure!(
                !cap.identity().populated()?,
                "managed Java exited but processes remain in its cgroup; refusing checkpoint"
            );
        }
        Ok(())
    }

    /// Persist the exact spawned child identity immediately. The caller's
    /// startup guard must kill/reap an unverified child if publication fails.
    pub fn record_java(&mut self, pid: u32) -> Result<()> {
        // Mark the in-memory record first, including on inspection failure:
        // Drop must never remove pending ownership while this child lives.
        self.record.java_pid = Some(pid);
        if let Some(cap) = &self.memory_cap {
            cap.verify_child(pid)?;
        }
        #[cfg(target_os = "linux")]
        {
            let birth =
                linux_process::read_process(pid)?.context("Java exited before process identity was recorded")?;
            self.record.linux.as_mut().context("missing Linux controller identity")?.java_start_ticks =
                Some(birth.start_ticks);
        }
        atomic_write(&self.path, &self.record)
            .with_context(|| format!("cannot persist Java runtime ownership for PID {pid}"))?;
        if let Some(shared) = self.reservation {
            atomic_write(&shared.record_path, &self.record)
                .context("cannot persist account-wide Java runtime ownership")?;
        }
        Ok(())
    }
}

impl Drop for RuntimeProcessGuard<'_> {
    fn drop(&mut self) {
        let safe_to_remove = match self.record.java_pid {
            None => true, // No spawn was performed by this guard.
            Some(pid) => {
                match self.record.ownership() {
                    Ok(Ownership::Quiescent) => true,
                    // If initial identity capture failed, only a definite
                    // absence permits cleanup of our own pending record.
                    Err(_) if self.record.linux.as_ref().is_some_and(|v| v.java_start_ticks.is_none()) => {
                        matches!(process_alive(pid), Ok(false))
                    }
                    _ => false,
                }
            }
        };
        let safe_to_remove = safe_to_remove && self.verify_exit_quiescent().is_ok();
        if safe_to_remove {
            // A failed delete is conservative: the next owner will validate
            // the residual record, never infer capacity from the lock alone.
            let local_removed = remove_record(&self.path).is_ok();
            let shared_removed = self.reservation.is_none_or(|shared| remove_record(&shared.record_path).is_ok());
            if local_removed && shared_removed {
                if let Some(cap) = &mut self.memory_cap {
                    cap.release_after_record_cleanup();
                }
            }
        }
    }
}

impl RuntimeProcessRecord {
    fn pending() -> Result<Self> {
        let controller_pid = std::process::id();
        #[cfg(target_os = "linux")]
        {
            let linux = linux_process::current_identity(controller_pid)?;
            Ok(Self { version: LINUX_RECORD_VERSION, controller_pid, java_pid: None, linux: Some(linux) })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(Self { version: LEGACY_RECORD_VERSION, controller_pid, java_pid: None, linux: None })
        }
    }

    fn ownership(&self) -> Result<Ownership> {
        if self.controller_pid == 0 || self.java_pid == Some(0) {
            bail!("invalid hosting process PID");
        }
        match (self.version, &self.linux) {
            (LEGACY_RECORD_VERSION, None) => match self.java_pid {
                None => Ok(Ownership::Pending),
                Some(pid) if process_alive(pid)? => Ok(Ownership::Live(pid)),
                Some(_) => Ok(Ownership::Quiescent),
            },
            (LINUX_RECORD_VERSION, Some(identity)) => {
                #[cfg(target_os = "linux")]
                {
                    linux_process::ownership(self.java_pid, identity)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = identity;
                    bail!("Linux hosting process identity cannot be verified on this platform")
                }
            }
            _ => bail!("unsupported or malformed hosting process record; new JVM denied"),
        }
    }
}

/// Deny new admission while any world under this data root has a residual
/// process. The account-wide reservation additionally covers other roots.
pub fn refuse_unaccounted_runtimes(paths: &DataPaths) -> Result<()> {
    let control = paths.root.join("control");
    match fs::symlink_metadata(&control) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            bail!("hosting control root is not a real directory; JVM admission denied");
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("cannot inspect hosting control root"),
    }
    #[cfg(target_os = "linux")]
    crate::machine_hosting::linux::validate_ancestors(&control)?;
    for entry in fs::read_dir(&control).with_context(|| format!("cannot enumerate {}", control.display()))? {
        let entry = entry.context("cannot enumerate hosting process records")?;
        let kind = entry.file_type().context("cannot inspect hosting control entry type")?;
        if kind.is_symlink() {
            bail!("hosting control directory contains an untrusted symlink; JVM admission denied");
        }
        if kind.is_dir() {
            check_quiescent_record(&entry.path().join("runtime-process.json"))?;
        }
    }
    Ok(())
}

/// Scoped replay-publication check while the world runtime lock is held.
pub(crate) fn refuse_unaccounted_world_runtime(paths: &DataPaths, world: WorldId) -> Result<()> {
    let control = paths.root.join("control");
    let world_control = control.join(world.to_hex());
    for directory in [&control, &world_control] {
        match fs::symlink_metadata(directory) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                bail!("untrusted runtime control directory {}; replay denied", directory.display());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).with_context(|| format!("cannot inspect {}", directory.display())),
        }
    }
    check_quiescent_record(&world_control.join("runtime-process.json"))
}

fn record_exists(path: &Path) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        crate::machine_hosting::linux::validate_file(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
                bail!("untrusted hosting process record {}; JVM admission denied", path.display());
            }
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("cannot inspect hosting process record"),
        }
    }
}

fn read_record(path: &Path) -> Result<Option<RuntimeProcessRecord>> {
    if !record_exists(path)? {
        return Ok(None);
    }
    // Once observed, removal or replacement cannot become an absent record.
    let file =
        File::open(path).with_context(|| format!("cannot read observed hosting process record {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_RECORD_BYTES {
        bail!("unreadable hosting process record at {}; new JVM denied", path.display());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .with_context(|| format!("unreadable hosting process record at {}; new JVM denied", path.display()))
}

fn check_quiescent_record(path: &Path) -> Result<()> {
    let Some(record) = read_record(path)? else { return Ok(()) };
    match record.ownership()? {
        Ownership::Quiescent => Ok(()),
        Ownership::Pending => {
            bail!("ambiguous prior Minecraft launch at {}; refusing additional JVM admission", path.display())
        }
        #[cfg(target_os = "linux")]
        Ownership::ResidualProcesses => {
            bail!("previous managed JVM left live cgroup descendants; refusing additional admission")
        }
        Ownership::Live(pid) => bail!(
            "unaccounted Minecraft process PID {pid} remains alive in {}; refusing additional JVM admission",
            path.display()
        ),
    }
}

/// Used only with the appropriate exclusive runtime/account lock.
pub(crate) fn clear_quiescent_record(path: &Path) -> Result<()> {
    let Some(record) = read_record(path)? else { return Ok(()) };
    match record.ownership()? {
        Ownership::Quiescent => remove_record(path),
        Ownership::Pending => bail!("a previous authority launch ended before Java ownership was durably recorded; refusing unsafe runtime reset"),
        #[cfg(target_os = "linux")]
        Ownership::ResidualProcesses => bail!("previous managed JVM left live cgroup descendants; refusing runtime reset"),
        Ownership::Live(pid) => bail!("previous Minecraft Java process PID {pid} is still alive; refusing to reset its runtime directory"),
    }
}

pub(crate) fn runtime_present(paths: &DataPaths, world: WorldId) -> Result<bool> {
    let Some(record) = read_record(&record_path(paths, world))? else { return Ok(false) };
    Ok(record.ownership()? != Ownership::Quiescent)
}

/// Local observation only; a durable record alone does not certify a live cap.
pub(crate) fn observed_memory(paths: &DataPaths, world: WorldId) -> Result<Option<ProcessMemoryReport>> {
    let Some(record) = read_record(&record_path(paths, world))? else { return Ok(None) };
    match record.ownership()? {
        Ownership::Live(pid) => match record.linux.as_ref().and_then(|identity| identity.cgroup.as_ref()) {
            Some(cap) => cap.observe(pid).map(Some),
            None => Ok(Some(ProcessMemoryReport::HeapOnly {})),
        },
        #[cfg(target_os = "linux")]
        Ownership::ResidualProcesses => bail!("previous managed JVM left live cgroup descendants"),
        Ownership::Pending | Ownership::Quiescent => Ok(None),
    }
}

/// Active CPU accounting requires the exact live process and cgroup policy.
pub(crate) fn observed_cpu(paths: &DataPaths, world: WorldId) -> Result<Option<ProcessCpuReport>> {
    let Some(record) = read_record(&record_path(paths, world))? else { return Ok(None) };
    match record.ownership()? {
        Ownership::Live(pid) => match record.linux.as_ref().and_then(|identity| identity.cgroup.as_ref()) {
            Some(cap) => cap.observe_cpu(pid).map(Some),
            None => Ok(Some(ProcessCpuReport::Unspecified {})),
        },
        #[cfg(target_os = "linux")]
        Ownership::ResidualProcesses => bail!("previous managed JVM left live cgroup descendants"),
        #[cfg(target_os = "linux")]
        Ownership::Pending => {
            let Some(identity) = record.linux.as_ref() else { return Ok(None) };
            let Some(cap) = identity.cgroup.as_ref() else { return Ok(None) };
            let observation = cap.observe_reserved_cpu()?;
            if observation.is_some() {
                anyhow::ensure!(
                    linux_process::controller_live(record.controller_pid, identity)?,
                    "pending CPU admission controller is no longer verifiably alive"
                );
            }
            Ok(observation)
        }
        #[cfg(not(target_os = "linux"))]
        Ownership::Pending => Ok(None),
        Ownership::Quiescent => Ok(None),
    }
}

fn remove_record(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            #[cfg(unix)]
            File::open(path.parent().context("hosting record has no parent")?)?.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("cannot clear quiescent hosting process record"),
    }
}

fn record_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("control").join(world.to_hex()).join("runtime-process.json")
}

fn atomic_write(path: &Path, record: &RuntimeProcessRecord) -> Result<()> {
    let parent = path.parent().context("runtime process record has no parent directory")?;
    #[cfg(target_os = "linux")]
    crate::machine_hosting::linux::validate_ancestors(parent)?;
    swarm_core::create_private_dir_all(parent)?;
    record_exists(path)?;
    let temporary =
        path.with_extension(format!("json.{}.{}.tmp", std::process::id(), hex::encode(swarm_core::random_nonce())));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(record)?)?;
        file.sync_all()?;
        drop(file);
        // In particular, never delete a durable pending record first on
        // Windows. A crash in a remove/rename gap would lose ownership.
        crate::authority_permit::replace_permit_atomically(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.context("cannot publish durable runtime process ownership")
}

#[cfg(target_os = "linux")]
mod linux_process {
    use super::*;

    pub(super) struct ProcStat {
        pub(super) start_ticks: u64,
        state: u8,
    }

    fn parse_stat(value: &str, pid: u32) -> Result<ProcStat> {
        // comm may contain spaces, parentheses and newlines. The final ')'
        // delimits it; fields after it start at field 3 (state).
        let (head, tail) = value.rsplit_once(')').context("malformed Linux process stat")?;
        let prefix = head.split_once(" (").context("malformed Linux process stat PID")?.0;
        anyhow::ensure!(prefix.parse::<u32>()? == pid, "Linux process stat PID mismatch");
        let fields = tail.split_ascii_whitespace().collect::<Vec<_>>();
        let state = fields.first().context("Linux process stat has no state")?.as_bytes();
        anyhow::ensure!(state.len() == 1, "invalid Linux process state");
        let start_ticks = fields.get(19).context("Linux process stat has no start time")?.parse()?;
        Ok(ProcStat { start_ticks, state: state[0] })
    }

    pub(super) fn read_process(pid: u32) -> Result<Option<ProcStat>> {
        if pid == 0 || pid > i32::MAX as u32 {
            bail!("invalid Linux process PID");
        }
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(value) => parse_stat(&value, pid).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A hidden/unmounted procfs is not evidence of process death.
                if process_alive(pid)? {
                    bail!("live PID cannot be inspected in Linux procfs");
                }
                Ok(None)
            }
            Err(error) => Err(error).context("cannot inspect Linux process birth identity"),
        }
    }

    fn valid_boot_id(value: &str) -> bool {
        value.len() == 36
            && value.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
    }

    fn context() -> Result<(String, String, u64)> {
        use std::os::unix::fs::MetadataExt;
        let machine = fs::read_to_string("/etc/machine-id").context("Linux machine identity unavailable")?;
        let machine = machine.trim().to_owned();
        anyhow::ensure!(
            machine.len() == 32 && machine.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid Linux machine identity"
        );
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").context("Linux boot identity unavailable")?;
        let boot = boot.trim().to_owned();
        anyhow::ensure!(valid_boot_id(&boot), "invalid Linux boot identity");
        let namespace = fs::metadata("/proc/self/ns/pid").context("Linux PID namespace unavailable")?.ino();
        anyhow::ensure!(namespace != 0, "invalid Linux PID namespace");
        Ok((machine, boot, namespace))
    }

    pub(super) fn current_identity(controller_pid: u32) -> Result<LinuxRuntimeIdentity> {
        let (machine_id, boot_id, pid_namespace) = context()?;
        let controller = read_process(controller_pid)?.context("cannot inspect own controller process")?;
        Ok(LinuxRuntimeIdentity {
            machine_id,
            boot_id,
            pid_namespace,
            controller_start_ticks: controller.start_ticks,
            java_start_ticks: None,
            cgroup: None,
        })
    }

    pub(super) fn controller_live(pid: u32, identity: &LinuxRuntimeIdentity) -> Result<bool> {
        Ok(read_process(pid)?.is_some_and(|stat| {
            stat.start_ticks == identity.controller_start_ticks && !matches!(stat.state, b'Z' | b'X' | b'x')
        }))
    }

    pub(super) fn ownership(pid: Option<u32>, identity: &LinuxRuntimeIdentity) -> Result<Ownership> {
        let (machine, boot, namespace) = context()?;
        anyhow::ensure!(identity.machine_id == machine, "hosting record belongs to another Linux machine");
        anyhow::ensure!(
            identity.pid_namespace != 0 && valid_boot_id(&identity.boot_id),
            "invalid recorded Linux process context"
        );
        anyhow::ensure!(pid.is_some() == identity.java_start_ticks.is_some(), "incomplete Java birth identity");
        if identity.boot_id != boot {
            // No process from an earlier boot of this local machine survives,
            // including an interrupted launch that never published its PID.
            return Ok(Ownership::Quiescent);
        }
        anyhow::ensure!(
            identity.pid_namespace == namespace,
            "cannot prove process absence across Linux PID namespaces"
        );
        let Some(pid) = pid else { return Ok(Ownership::Pending) };
        if read_process(pid)?.is_some_and(|stat| {
            Some(stat.start_ticks) == identity.java_start_ticks && !matches!(stat.state, b'Z' | b'X' | b'x')
        }) {
            return Ok(Ownership::Live(pid));
        }
        if let Some(cap) = &identity.cgroup {
            if cap.populated()? {
                return Ok(Ownership::ResidualProcesses);
            }
        }
        Ok(Ownership::Quiescent)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn process_stat_parser_accepts_parentheses_spaces_and_newlines_in_comm() {
            let mut fields = vec!["0"; 24];
            fields[0] = "S";
            fields[19] = "1234567";
            let value = format!("17 (a ) weird\ncomm) {}", fields.join(" "));
            let stat = parse_stat(&value, 17).unwrap();
            assert_eq!(stat.start_ticks, 1_234_567);
            assert_eq!(stat.state, b'S');
            assert!(parse_stat(&value, 18).is_err());
        }
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> Result<bool> {
    const ESRCH: i32 = 3;
    const EPERM: i32 = 1;
    extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    if pid == 0 || pid > i32::MAX as u32 {
        bail!("invalid process PID");
    }
    // Signal zero probes existence only; it never terminates a process.
    let result = unsafe { kill(pid as i32, 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(ESRCH) => Ok(false),
        Some(EPERM) => Ok(true),
        _ => Err(error.into()),
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> Result<bool> {
    type Handle = *mut std::ffi::c_void;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn CloseHandle(object: Handle) -> i32;
    }
    if pid == 0 {
        bail!("invalid process PID");
    }
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
            return Ok(false);
        }
        // Access denied or any other indeterminate result fails closed.
        return Err(error).context("cannot inspect Windows runtime process");
    }
    // A wait on the kernel process object avoids the STILL_ACTIVE (259)
    // exit-code ambiguity; the process handle remains valid during the wait.
    let result = unsafe { WaitForSingleObject(handle, 0) };
    let error = std::io::Error::last_os_error();
    unsafe { CloseHandle(handle) };
    match result {
        WAIT_OBJECT_0 => Ok(false),
        WAIT_TIMEOUT => Ok(true),
        _ => Err(error).context("cannot verify Windows runtime process liveness"),
    }
}

#[cfg(not(any(unix, windows)))]
fn process_alive(_pid: u32) -> Result<bool> {
    bail!("process-liveness proof is not implemented on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> WorldId {
        WorldId([0x66; 32])
    }

    #[test]
    fn live_orphan_in_another_world_blocks_new_jvm_admission() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let prior_world = world();
        let path = record_path(&paths, prior_world);
        atomic_write(
            &path,
            &RuntimeProcessRecord {
                version: LEGACY_RECORD_VERSION,
                controller_pid: 1,
                java_pid: Some(std::process::id()),
                linux: None,
            },
        )
        .unwrap();
        let err = refuse_unaccounted_runtimes(&paths).unwrap_err();
        assert!(err.to_string().contains("unaccounted Minecraft process"));
    }

    #[test]
    fn ambiguous_or_corrupt_or_symlinked_process_records_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let path = record_path(&paths, world());
        atomic_write(
            &path,
            &RuntimeProcessRecord { version: LEGACY_RECORD_VERSION, controller_pid: 1, java_pid: None, linux: None },
        )
        .unwrap();
        assert!(refuse_unaccounted_runtimes(&paths)
            .unwrap_err()
            .to_string()
            .contains("ambiguous prior Minecraft launch"));
        fs::write(&path, b"{not-json").unwrap();
        assert!(refuse_unaccounted_runtimes(&paths).is_err());

        fs::remove_file(&path).unwrap();
        assert!(refuse_unaccounted_runtimes(&paths).is_ok());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = paths.root.join("control").join("untrusted-link");
            symlink(temp.path(), &link).unwrap();
            assert!(refuse_unaccounted_runtimes(&paths).unwrap_err().to_string().contains("untrusted symlink"));
            fs::remove_file(link).unwrap();

            let dangling = temp.path().join("missing-runtime-record");
            symlink(dangling, &path).unwrap();
            assert!(refuse_unaccounted_runtimes(&paths)
                .unwrap_err()
                .to_string()
                .contains("untrusted hosting process record"));
            fs::remove_file(&path).unwrap();

            let control_root = paths.root.join("control");
            let original = paths.root.join("original-control");
            fs::rename(&control_root, &original).unwrap();
            symlink(&original, &control_root).unwrap();
            assert!(refuse_unaccounted_runtimes(&paths)
                .unwrap_err()
                .to_string()
                .contains("hosting control root is not a real directory"));
        }
    }

    #[test]
    fn current_process_record_blocks_runtime_reset() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let path = record_path(&paths, world());
        atomic_write(
            &path,
            &RuntimeProcessRecord {
                version: LEGACY_RECORD_VERSION,
                controller_pid: std::process::id(),
                java_pid: Some(std::process::id()),
                linux: None,
            },
        )
        .unwrap();
        let error = RuntimeProcessGuard::begin(&paths, world()).err().expect("live PID must block");
        assert!(error.to_string().contains("still alive"));
    }

    #[test]
    fn ambiguous_pending_record_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let path = record_path(&paths, world());
        atomic_write(
            &path,
            &RuntimeProcessRecord { version: LEGACY_RECORD_VERSION, controller_pid: 1, java_pid: None, linux: None },
        )
        .unwrap();
        let error = RuntimeProcessGuard::begin(&paths, world()).err().expect("ambiguous record must block");
        assert!(error.to_string().contains("before Java ownership was durably recorded"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn birth_identity_distinguishes_pid_reuse_and_namespace_ambiguity() {
        let mut record = RuntimeProcessRecord::pending().unwrap();
        let pid = std::process::id();
        record.java_pid = Some(pid);
        let current = linux_process::read_process(pid).unwrap().unwrap();
        record.linux.as_mut().unwrap().java_start_ticks = Some(current.start_ticks);
        assert_eq!(record.ownership().unwrap(), Ownership::Live(pid));
        // Same live PID, different birth identity: the old Java no longer
        // exists. No signal is ever sent to the replacement process.
        record.linux.as_mut().unwrap().java_start_ticks = Some(current.start_ticks + 1);
        assert_eq!(record.ownership().unwrap(), Ownership::Quiescent);
        record.linux.as_mut().unwrap().pid_namespace ^= 1;
        assert!(record.ownership().is_err(), "another PID namespace is not evidence of death");
        record.linux.as_mut().unwrap().machine_id = "0".repeat(32);
        assert!(record.ownership().is_err(), "foreign machine record must fail closed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pending_launch_requires_same_boot_investigation_but_reboot_proves_absence() {
        let mut record = RuntimeProcessRecord::pending().unwrap();
        assert_eq!(record.ownership().unwrap(), Ownership::Pending);
        let linux = record.linux.as_mut().unwrap();
        linux.boot_id = if linux.boot_id == "00000000-0000-0000-0000-000000000001" {
            "00000000-0000-0000-0000-000000000002".into()
        } else {
            "00000000-0000-0000-0000-000000000001".into()
        };
        assert_eq!(record.ownership().unwrap(), Ownership::Quiescent);
    }

    #[cfg(target_os = "linux")]
    struct TestChild(std::process::Child);

    #[cfg(target_os = "linux")]
    impl Drop for TestChild {
        fn drop(&mut self) {
            // Only child handles created by this test are terminated.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn account_reservation_crash_worker() {
        // Test harness only. The production executable has no environment
        // override for its account-wide reservation directory.
        let Some(root) = std::env::var_os("SWARMCRAFT_TEST_RESERVATION_ROOT") else { return };
        let root = PathBuf::from(root);
        let reservation = MachineHostingReservation::acquire_at(&root.join("account")).unwrap();
        let paths = DataPaths::from_root(root.join("first-data-root"));
        let mut guard = RuntimeProcessGuard::begin_coordinated(&paths, world(), Some(&reservation), None).unwrap();
        if let Ok(pid) = std::env::var("SWARMCRAFT_TEST_SURVIVING_CHILD") {
            guard.record_java(pid.parse().unwrap()).unwrap();
        }
        fs::write(root.join("worker-ready"), b"ready").unwrap();
        // Parent kills this controller, so neither Rust Drop nor unlock
        // cleanup runs. A separate test-owned child represents Java.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn account_reservation_blocks_second_data_root_after_controller_hard_kill() {
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("account");
        let first = DataPaths::from_root(temp.path().join("first-data-root"));
        let second = DataPaths::from_root(temp.path().join("second-data-root"));

        // Keep a real OS child handle in the test parent so cleanup is safe
        // even on failed assertions; the worker records this surviving
        // process as its JVM, then suffers an actual hard controller death.
        let mut java = TestChild(Command::new("sleep").arg("120").spawn().unwrap());
        let pid = java.0.id();
        let mut controller = TestChild(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "runtime_process_guard::tests::account_reservation_crash_worker", "--nocapture"])
                .env("SWARMCRAFT_TEST_RESERVATION_ROOT", temp.path())
                .env("SWARMCRAFT_TEST_SURVIVING_CHILD", pid.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(8);
        while !temp.path().join("worker-ready").exists() {
            assert!(controller.0.try_wait().unwrap().is_none(), "reservation worker exited early");
            assert!(Instant::now() < deadline, "reservation worker did not publish ownership");
            std::thread::sleep(Duration::from_millis(20));
        }
        let account_record = directory.join("runtime-process.json");
        let local_record = record_path(&first, world());
        assert_eq!(read_record(&account_record).unwrap(), read_record(&local_record).unwrap());
        assert_eq!(read_record(&account_record).unwrap().unwrap().ownership().unwrap(), Ownership::Live(pid));
        assert!(MachineHostingReservation::acquire_at(&directory).is_err(), "live lock was bypassed");
        controller.0.kill().unwrap();
        controller.0.wait().unwrap();
        assert!(java.0.try_wait().unwrap().is_none(), "surviving Java fixture unexpectedly exited");
        assert!(refuse_unaccounted_runtimes(&first).is_err());
        // The second data root contains no process record whatsoever. Its
        // account-wide admission must still refuse while the old JVM lives.
        assert!(refuse_unaccounted_runtimes(&second).is_ok());
        let refused = MachineHostingReservation::acquire_at(&directory).err().unwrap();
        assert!(format!("{refused:#}").contains("still alive"));
        assert_eq!(read_record(&account_record).unwrap().unwrap().java_pid, Some(pid));

        java.0.kill().unwrap();
        java.0.wait().unwrap();
        let replacement = MachineHostingReservation::acquire_at(&directory).unwrap();
        assert!(refuse_unaccounted_runtimes(&first).is_ok());
        assert!(!account_record.exists());
        {
            let _guard = RuntimeProcessGuard::begin_coordinated(&second, world(), Some(&replacement), None).unwrap();
            assert!(account_record.exists());
            assert!(record_path(&second, world()).exists());
        }
        assert!(!account_record.exists());
        drop(replacement);
        assert!(MachineHostingReservation::acquire_at(&directory).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn interrupted_pending_reservation_is_not_repaired_by_switching_roots() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("account");
        let first = DataPaths::from_root(temp.path().join("first-data-root"));
        let reservation = MachineHostingReservation::acquire_at(&directory).unwrap();
        let guard = RuntimeProcessGuard::begin_coordinated(&first, world(), Some(&reservation), None).unwrap();
        std::mem::forget(guard);
        drop(reservation);
        assert!(MachineHostingReservation::acquire_at(&directory).is_err());
        assert!(refuse_unaccounted_runtimes(&first).is_err());
    }

    #[test]
    fn clean_unspawned_guard_removes_pending_record() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let path = record_path(&paths, world());
        {
            let _guard = RuntimeProcessGuard::begin(&paths, world()).unwrap();
            assert!(path.is_file());
        }
        assert!(!path.exists());
    }
}
