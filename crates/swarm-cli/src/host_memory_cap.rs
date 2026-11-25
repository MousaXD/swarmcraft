//! Optional Linux delegated cgroup-v2 memory enforcement for the managed JVM
//! and its descendants. No changes to the operator's parent controllers,
//! no attachment after exec, and no fallback on a configured cap failure.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Command};

pub const HOST_CGROUP_ROOT_ENV: &str = "SWARMCRAFT_HOST_CGROUP_ROOT";
const MIB: u64 = 1024 * 1024;
pub const MIN_CPU_MILLICORES: u64 = 250;
pub const MAX_CPU_MILLICORES: u64 = 64_000;
const CPU_PERIOD_US: u64 = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessCpuReport {
    Unspecified {},
    LinuxCgroupConfigured {
        limit_millicores: u64,
    },
    LinuxCgroupReserved {
        limit_millicores: u64,
        admitted_heap_mib: u64,
        admitted_consent_revision: String,
    },
    LinuxCgroupActive {
        limit_millicores: u64,
        admitted_heap_mib: u64,
        admitted_consent_revision: String,
        usage_usec: u64,
    },
    Unavailable {
        detail: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CpuLimit {
    millicores: u64,
    admitted_heap_mib: u64,
    consent_revision: u64,
}

pub fn validate_cpu_request(millicores: u64) -> Result<()> {
    anyhow::ensure!(
        (MIN_CPU_MILLICORES..=MAX_CPU_MILLICORES).contains(&millicores),
        "CPU limit must be between {MIN_CPU_MILLICORES} and {MAX_CPU_MILLICORES} millicores"
    );
    Ok(())
}

/// Logical CPU-equivalent capacity, not free CPU time or a reservation.
pub(crate) fn cpu_capacity_millicores() -> Result<u64> {
    #[cfg(target_os = "linux")]
    {
        linux::cpu_capacity_millicores()
    }
    #[cfg(not(target_os = "linux"))]
    {
        bail!("effective CPU controller capacity is not verified on this platform")
    }
}

pub(crate) fn require_cpu_configuration(cpu: Option<u64>) -> Result<()> {
    let Some(cpu) = cpu else { return Ok(()) };
    validate_cpu_request(cpu)?;
    let root = configured_root()?.context("explicit CPU cap requires a delegated Linux memory and CPU cgroup")?;
    #[cfg(target_os = "linux")]
    {
        linux::validate_cpu_root(&root)?;
        anyhow::ensure!(cpu <= cpu_capacity_millicores()?, "requested CPU limit exceeds measured CPU capacity");
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        bail!("explicit CPU enforcement is unavailable on this platform")
    }
}

pub(crate) fn configured_cpu_report(cpu: Option<u64>) -> Result<ProcessCpuReport> {
    match cpu {
        None => Ok(ProcessCpuReport::Unspecified {}),
        Some(limit_millicores) => {
            require_cpu_configuration(cpu)?;
            Ok(ProcessCpuReport::LinuxCgroupConfigured { limit_millicores })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessMemoryReport {
    HeapOnly {},
    LinuxCgroupConfigured { limit_mib: u64 },
    LinuxCgroupActive { limit_mib: u64, current_bytes: u64, peak_bytes: Option<u64> },
    Unavailable { detail: String },
}

/// A persisted identity, never an operator-supplied command. Only the
/// dedicated leaf created by this build can be consulted for residual
/// process ownership after a supervisor crash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryCapIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
    limit_bytes: u64,
    high_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cpu: Option<CpuLimit>,
}

pub(crate) struct HostMemoryCap {
    identity: MemoryCapIdentity,
    claimed_by_record: bool,
}

impl HostMemoryCap {
    pub(crate) fn prepare(heap_mib: u64, cpu: Option<u64>, consent_revision: u64) -> Result<Option<Self>> {
        require_cpu_configuration(cpu)?;
        let Some(root) = configured_root()? else { return Ok(None) };
        #[cfg(target_os = "linux")]
        {
            let limit = cap_bytes(heap_mib)?;
            linux::create_with_cpu(
                &root,
                limit,
                cpu.map(|millicores| CpuLimit { millicores, admitted_heap_mib: heap_mib, consent_revision }),
            )
            .map(Some)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (root, heap_mib, cpu, consent_revision);
            bail!("whole-process hosting memory enforcement is unavailable on this platform")
        }
    }

    pub(crate) fn identity(&self) -> &MemoryCapIdentity {
        &self.identity
    }

    pub(crate) fn claim_by_record(&mut self) {
        self.claimed_by_record = true;
    }

    pub(crate) fn configure_command(&self, command: &mut Command) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            linux::configure_command(&self.identity, command)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = command;
            bail!("Linux cgroup command attachment is unavailable")
        }
    }

    pub(crate) fn verify_child(&self, pid: u32) -> Result<()> {
        self.identity.verify_child(pid)
    }

    pub(crate) fn pressure(&self) -> Result<bool> {
        #[cfg(target_os = "linux")]
        {
            let sample = linux::sample(&self.identity)?;
            Ok(sample.current >= self.identity.high_bytes || sample.high_events != 0 || sample.oom_events != 0)
        }
        #[cfg(not(target_os = "linux"))]
        {
            bail!("Linux cgroup memory pressure cannot be observed")
        }
    }

    /// Remove only after both durable records were retired and the exact
    /// cgroup was proven empty. Never kill an unknown residual process.
    pub(crate) fn release_after_record_cleanup(&mut self) {
        self.claimed_by_record = false;
    }
}

impl Drop for HostMemoryCap {
    fn drop(&mut self) {
        #[cfg(not(target_os = "linux"))]
        let _ = self.claimed_by_record;
        #[cfg(target_os = "linux")]
        if !self.claimed_by_record && matches!(self.identity.populated(), Ok(false)) {
            let _ = std::fs::remove_dir(&self.identity.path);
        }
    }
}

impl MemoryCapIdentity {
    pub(crate) fn populated(&self) -> Result<bool> {
        #[cfg(target_os = "linux")]
        {
            linux::populated(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            bail!("recorded Linux cgroup cannot be inspected on this platform")
        }
    }

    pub(crate) fn verify_child(&self, pid: u32) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            linux::verify_child(self, pid)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pid;
            bail!("recorded Linux cgroup process membership cannot be inspected")
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn observe_reserved_cpu(&self) -> Result<Option<ProcessCpuReport>> {
        linux::observe_reserved_cpu(self)
    }

    pub(crate) fn observe_cpu(&self, pid: u32) -> Result<ProcessCpuReport> {
        self.verify_child(pid)?;
        #[cfg(target_os = "linux")]
        {
            linux::observe_cpu(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            bail!("recorded CPU accounting unavailable on this platform")
        }
    }

    pub(crate) fn observe(&self, pid: u32) -> Result<ProcessMemoryReport> {
        self.verify_child(pid)?;
        #[cfg(target_os = "linux")]
        {
            let sample = linux::sample(self)?;
            Ok(ProcessMemoryReport::LinuxCgroupActive {
                limit_mib: self.limit_bytes / MIB,
                current_bytes: sample.current,
                peak_bytes: sample.peak,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            bail!("Linux cgroup accounting is unavailable")
        }
    }
}

/// Configuration is advisory until a child is attached and observed.
pub(crate) fn configured_report(heap_mib: u64) -> Result<ProcessMemoryReport> {
    if heap_mib == 0 {
        return Ok(ProcessMemoryReport::HeapOnly {});
    }
    if configured_root()?.is_some() {
        Ok(ProcessMemoryReport::LinuxCgroupConfigured { limit_mib: cap_bytes(heap_mib)? / MIB })
    } else {
        Ok(ProcessMemoryReport::HeapOnly {})
    }
}

pub(crate) fn validate_configuration() -> Result<()> {
    configured_root().map(|_| ())
}

fn cap_bytes(heap_mib: u64) -> Result<u64> {
    heap_mib
        .checked_add(crate::hosting_budget::estimated_native_overhead_mib(heap_mib))
        .and_then(|mib| mib.checked_mul(MIB))
        .context("whole-process hosting memory budget overflow")
}

fn configured_root() -> Result<Option<PathBuf>> {
    let Some(root) = std::env::var_os(HOST_CGROUP_ROOT_ENV) else { return Ok(None) };
    let root = PathBuf::from(root);
    #[cfg(target_os = "linux")]
    {
        linux::validate_delegated_root(&root)?;
        Ok(Some(root))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        bail!("a Linux hosting cgroup was configured on an unsupported platform; refusing heap-only fallback")
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::ensure;
    use std::{
        ffi::CString,
        fs::{self, File, OpenOptions},
        os::{
            fd::AsRawFd,
            unix::{ffi::OsStrExt, fs::MetadataExt, process::CommandExt},
        },
        path::{Component, Path},
    };

    const CGROUP_MOUNT: &str = "/sys/fs/cgroup";
    const LEAF_PREFIX: &str = "swarmcraft-jvm-";
    const CGROUP2_SUPER_MAGIC: u64 = 0x6367_7270;

    fn read_number(path: &Path) -> Result<u64> {
        fs::read_to_string(path)?
            .trim()
            .parse::<u64>()
            .with_context(|| format!("invalid cgroup counter {}", path.display()))
    }

    fn write_number(path: &Path, value: u64) -> Result<()> {
        fs::write(path, format!("{value}\n")).with_context(|| format!("cannot configure {}", path.display()))?;
        ensure!(read_number(path)? == value, "kernel did not retain requested cgroup setting {}", path.display());
        Ok(())
    }

    fn counter(path: &Path, key: &str) -> Result<u64> {
        let value = fs::read_to_string(path)?;
        let mut matching = value.lines().filter_map(|line| {
            let (name, number) = line.split_once(' ')?;
            (name == key).then_some(number)
        });
        let result = matching.next().with_context(|| format!("missing cgroup counter {key}"))?.trim().parse()?;
        ensure!(matching.next().is_none(), "duplicate cgroup counter {key}");
        Ok(result)
    }

    fn unified_path(pid: u32) -> Result<PathBuf> {
        ensure!(pid != 0, "invalid process PID");
        let value = fs::read_to_string(format!("/proc/{pid}/cgroup"))?;
        let mut rows = value.lines().filter_map(|line| line.strip_prefix("0::"));
        let path = Path::new(rows.next().context("unified Linux cgroup membership unavailable")?);
        ensure!(rows.next().is_none() && path.is_absolute(), "ambiguous unified cgroup membership");
        ensure!(!path.components().any(|part| part == Component::ParentDir), "invalid cgroup membership path");
        Ok(Path::new(CGROUP_MOUNT).join(path.strip_prefix("/")?))
    }

    fn validate_real_cgroup(path: &Path) -> Result<fs::Metadata> {
        ensure!(
            path.is_absolute()
                && path.starts_with(CGROUP_MOUNT)
                && path != Path::new(CGROUP_MOUNT)
                && !path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)),
            "hosting cgroup must be an absolute delegated path under /sys/fs/cgroup"
        );
        ensure!(fs::canonicalize(path)? == path, "hosting cgroup path is not canonical");
        let meta = fs::symlink_metadata(path)?;
        ensure!(meta.is_dir() && !meta.file_type().is_symlink(), "hosting cgroup must be a real directory");
        let cpath = CString::new(path.as_os_str().as_bytes())?;
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
        // SAFETY: cpath is NUL-terminated and stat has writable storage.
        if unsafe { libc::statfs(cpath.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("cannot identify cgroup filesystem");
        }
        ensure!(
            unsafe { stat.assume_init() }.f_type as u64 & 0xffff_ffff == CGROUP2_SUPER_MAGIC,
            "configured path is not on the Linux cgroup-v2 filesystem"
        );
        Ok(meta)
    }

    pub(super) fn validate_delegated_root(root: &Path) -> Result<()> {
        let meta = validate_real_cgroup(root)?;
        ensure!(
            meta.uid() == crate::machine_hosting::linux::uid() && meta.mode() & 0o022 == 0,
            "hosting cgroup root must be a privately delegated directory owned by this account"
        );
        let current = unified_path(std::process::id())?;
        ensure!(
            current.starts_with(root) && current != root,
            "controller must run in a child of its delegated cgroup root"
        );
        ensure!(
            fs::read_to_string(root.join("cgroup.type"))?.trim() == "domain",
            "delegated root must be a domain cgroup"
        );
        ensure!(
            fs::read_to_string(root.join("cgroup.procs"))?.trim().is_empty(),
            "delegated cgroup root contains unrelated processes"
        );
        ensure!(
            fs::read_to_string(root.join("cgroup.subtree_control"))?.split_whitespace().any(|value| value == "memory"),
            "memory controller is not delegated/enabled for hosting leaves"
        );
        Ok(())
    }

    pub(super) fn validate_cpu_root(root: &Path) -> Result<()> {
        validate_delegated_root(root)?;
        ensure!(
            fs::read_to_string(root.join("cgroup.subtree_control"))?.split_whitespace().any(|value| value == "cpu"),
            "CPU controller is not delegated/enabled for hosting leaves"
        );
        Ok(())
    }

    fn parse_cpu_max(value: &str) -> Result<Option<u64>> {
        let fields: Vec<_> = value.split_whitespace().collect();
        ensure!(fields.len() == 2, "invalid cpu.max");
        let period: u64 = fields[1].parse()?;
        ensure!((1000..=1_000_000).contains(&period), "invalid CPU quota period");
        if fields[0] == "max" {
            return Ok(None);
        }
        let quota: u64 = fields[0].parse()?;
        ensure!(quota >= 1000, "invalid CPU quota");
        Ok(Some(((quota as u128 * 1000) / period as u128).min(u64::MAX as u128) as u64))
    }

    pub(super) fn cpu_capacity_millicores() -> Result<u64> {
        let mount = Path::new(CGROUP_MOUNT);
        crate::hosting_budget::require_visible_cgroup_v2_mount(&fs::read_to_string("/proc/self/mountinfo")?)?;
        let mut current = unified_path(std::process::id())?;
        ensure!(current.is_dir() && fs::canonicalize(&current)? == current, "unverifiable CPU hierarchy");
        // sched_getaffinity gives the actual allowed logical processors,
        // unlike host CPU count. Ancestor finite quotas further bound it.
        let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
        if unsafe { libc::sched_getaffinity(0, std::mem::size_of_val(&set), &mut set) } != 0 {
            return Err(std::io::Error::last_os_error()).context("cannot sample CPU affinity");
        }
        let mut capacity = (unsafe { libc::CPU_COUNT(&set) } as u64).saturating_mul(1000);
        loop {
            match fs::read_to_string(current.join("cpu.max")) {
                Ok(value) => {
                    if let Some(limit) = parse_cpu_max(&value)? {
                        capacity = capacity.min(limit);
                    }
                }
                // A cgroup without the controller enabled has no local
                // quota; still inspect every visible ancestor.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            if current == mount {
                break;
            }
            current = current.parent().context("CPU hierarchy escaped mount")?.to_path_buf();
            ensure!(current.starts_with(mount), "CPU hierarchy escaped mount");
        }
        ensure!(capacity > 0, "CPU capacity is unavailable");
        Ok(capacity)
    }

    fn verify_cpu_limit(identity: &MemoryCapIdentity) -> Result<()> {
        if let Some(cpu) = &identity.cpu {
            validate_cpu_request(cpu.millicores)?;
            ensure!(
                (crate::hosting_budget::MIN_HEAP_MIB..=crate::hosting_budget::MAX_HEAP_MIB)
                    .contains(&cpu.admitted_heap_mib)
                    && cap_bytes(cpu.admitted_heap_mib)? == identity.limit_bytes
                    && cpu.consent_revision != 0,
                "invalid CPU admission memory binding"
            );
            let expected = format!("{} {}", cpu.millicores * (CPU_PERIOD_US / 1000), CPU_PERIOD_US);
            ensure!(
                fs::read_to_string(identity.path.join("cpu.max"))?.trim() == expected,
                "active hosting cgroup CPU policy changed; refusing runtime proof"
            );
            match fs::read_to_string(identity.path.join("cpu.max.burst")) {
                Ok(value) => ensure!(value.trim() == "0", "active hosting CPU burst policy changed"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub(super) fn observe_reserved_cpu(identity: &MemoryCapIdentity) -> Result<Option<ProcessCpuReport>> {
        let Some(cpu) = &identity.cpu else { return Ok(None) };
        verify_limits(identity)?;
        ensure!(!populated(identity)?, "pending CPU reservation already has unverified processes");
        Ok(Some(ProcessCpuReport::LinuxCgroupReserved {
            limit_millicores: cpu.millicores,
            admitted_heap_mib: cpu.admitted_heap_mib,
            admitted_consent_revision: cpu.consent_revision.to_string(),
        }))
    }

    pub(super) fn observe_cpu(identity: &MemoryCapIdentity) -> Result<ProcessCpuReport> {
        verify_limits(identity)?;
        match &identity.cpu {
            None => Ok(ProcessCpuReport::Unspecified {}),
            Some(cpu) => Ok(ProcessCpuReport::LinuxCgroupActive {
                limit_millicores: cpu.millicores,
                admitted_heap_mib: cpu.admitted_heap_mib,
                admitted_consent_revision: cpu.consent_revision.to_string(),
                usage_usec: counter(&identity.path.join("cpu.stat"), "usage_usec")?,
            }),
        }
    }

    fn high_limit(limit: u64) -> Result<u64> {
        // cgroup-v2 memory values are rounded to the kernel page size.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        ensure!(page > 0, "kernel page size is unavailable");
        let page = page as u64;
        Ok((limit.checked_mul(9).context("cgroup high budget overflow")? / 10 / page) * page)
    }

    #[cfg(test)]
    pub(super) fn create(root: &Path, limit_bytes: u64) -> Result<HostMemoryCap> {
        create_with_cpu(root, limit_bytes, None)
    }

    pub(super) fn create_with_cpu(root: &Path, limit_bytes: u64, cpu: Option<CpuLimit>) -> Result<HostMemoryCap> {
        validate_delegated_root(root)?;
        if cpu.is_some() {
            validate_cpu_root(root)?;
        }
        ensure!((16 * MIB..=128 * 1024 * MIB).contains(&limit_bytes), "invalid cgroup memory limit");
        let name = format!("{LEAF_PREFIX}{}", hex::encode(swarm_core::random_nonce()));
        let path = root.join(name);
        fs::create_dir(&path).context("cannot create dedicated JVM cgroup")?;
        let result = (|| -> Result<HostMemoryCap> {
            let meta = validate_real_cgroup(&path)?;
            let high_bytes = high_limit(limit_bytes)?;
            let identity = MemoryCapIdentity {
                path: path.clone(),
                device: meta.dev(),
                inode: meta.ino(),
                limit_bytes,
                high_bytes,
                cpu,
            };
            ensure!(!populated(&identity)?, "new hosting cgroup unexpectedly contains processes");
            // Hard kernel charge limit covers anonymous/native memory,
            // charged file cache and descendants; prohibit extra swap.
            write_number(&path.join("memory.max"), limit_bytes)?;
            write_number(&path.join("memory.high"), high_bytes)?;
            write_number(&path.join("memory.swap.max"), 0)?;
            write_number(&path.join("memory.oom.group"), 1)?;
            if let Some(cpu) = &identity.cpu {
                validate_cpu_request(cpu.millicores)?;
                fs::write(
                    path.join("cpu.max"),
                    format!(
                        "{} {}
",
                        cpu.millicores * (CPU_PERIOD_US / 1000),
                        CPU_PERIOD_US
                    ),
                )?;
            }
            // No Java may execute before these settings and attachment are
            // verified. Never mutate the parent's resource configuration.
            verify_limits(&identity)?;
            Ok(HostMemoryCap { identity, claimed_by_record: false })
        })();
        if result.is_err() {
            // rmdir on cgroupfs refuses populated groups; never kill tasks.
            let _ = fs::remove_dir(&path);
        }
        result
    }

    fn validate_identity(identity: &MemoryCapIdentity) -> Result<()> {
        let meta = validate_real_cgroup(&identity.path)?;
        let name = identity.path.file_name().and_then(|value| value.to_str()).context("invalid cgroup leaf name")?;
        let suffix = name.strip_prefix(LEAF_PREFIX).context("unexpected cgroup leaf name")?;
        ensure!(
            suffix.len() == 64 && suffix.bytes().all(|value| value.is_ascii_hexdigit()),
            "invalid cgroup leaf identity"
        );
        ensure!(
            meta.dev() == identity.device
                && meta.ino() == identity.inode
                && meta.uid() == crate::machine_hosting::linux::uid(),
            "recorded hosting cgroup was substituted or moved"
        );
        ensure!(
            (16 * MIB..=128 * 1024 * MIB).contains(&identity.limit_bytes)
                && identity.high_bytes == high_limit(identity.limit_bytes)?,
            "invalid recorded cgroup memory budget"
        );
        Ok(())
    }

    fn verify_limits(identity: &MemoryCapIdentity) -> Result<()> {
        validate_identity(identity)?;
        let path = &identity.path;
        ensure!(
            read_number(&path.join("memory.max"))? == identity.limit_bytes
                && read_number(&path.join("memory.high"))? == identity.high_bytes
                && read_number(&path.join("memory.swap.max"))? == 0
                && read_number(&path.join("memory.oom.group"))? == 1,
            "active hosting cgroup memory policy changed; refusing runtime proof"
        );
        verify_cpu_limit(identity)?;
        Ok(())
    }

    pub(super) fn populated(identity: &MemoryCapIdentity) -> Result<bool> {
        validate_identity(identity)?;
        match counter(&identity.path.join("cgroup.events"), "populated")? {
            0 => Ok(false),
            1 => Ok(true),
            _ => bail!("invalid cgroup population state"),
        }
    }

    pub(super) fn configure_command(identity: &MemoryCapIdentity, command: &mut Command) -> Result<()> {
        verify_limits(identity)?;
        ensure!(!populated(identity)?, "hosting cgroup is already populated");
        let procs: File = OpenOptions::new().write(true).open(identity.path.join("cgroup.procs"))?;
        // SAFETY: runs after fork and before Java exec. The captured fd was
        // opened in the parent; write(2) is async-signal-safe, and cgroup
        // procs value 0 means the calling child. No allocation, lock, path
        // resolution or unrelated PID manipulation occurs in the child.
        unsafe {
            command.pre_exec(move || {
                let bytes = b"0\n";
                let result = libc::write(procs.as_raw_fd(), bytes.as_ptr().cast(), bytes.len());
                if result == bytes.len() as isize {
                    Ok(())
                } else if result < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Err(std::io::Error::from_raw_os_error(libc::EIO))
                }
            });
        }
        Ok(())
    }

    pub(super) fn verify_child(identity: &MemoryCapIdentity, pid: u32) -> Result<()> {
        verify_limits(identity)?;
        ensure!(unified_path(pid)? == identity.path, "managed Java is not in its recorded memory-limited cgroup");
        ensure!(populated(identity)?, "managed Java cgroup is not populated");
        Ok(())
    }

    pub(super) struct Sample {
        pub(super) current: u64,
        pub(super) peak: Option<u64>,
        pub(super) high_events: u64,
        pub(super) oom_events: u64,
    }

    pub(super) fn sample(identity: &MemoryCapIdentity) -> Result<Sample> {
        verify_limits(identity)?;
        let path = &identity.path;
        let peak = match fs::read_to_string(path.join("memory.peak")) {
            Ok(value) => Some(value.trim().parse()?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("cannot read cgroup memory peak"),
        };
        Ok(Sample {
            current: read_number(&path.join("memory.current"))?,
            peak,
            high_events: counter(&path.join("memory.events"), "high")?,
            oom_events: counter(&path.join("memory.events"), "oom")?,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            os::unix::fs::PermissionsExt,
            process::{Child, Stdio},
            time::{Duration, Instant},
        };

        struct TestChild(Child);
        impl Drop for TestChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        #[test]
        fn ordinary_directory_and_root_cgroup_are_not_delegated_hosting_roots() {
            let temp = tempfile::tempdir().unwrap();
            assert!(validate_delegated_root(temp.path()).is_err());
            assert!(validate_delegated_root(Path::new(CGROUP_MOUNT)).is_err());
        }

        #[test]
        fn cpu_quota_parser_is_fractional_and_rejects_malformed_values() {
            assert_eq!(parse_cpu_max("max 100000").unwrap(), None);
            assert_eq!(parse_cpu_max("50000 100000").unwrap(), Some(500));
            assert_eq!(parse_cpu_max("150000 100000").unwrap(), Some(1500));
            for bad in ["max", "max 0", "0 100000", "100000 100000 extra", "bad 100000"] {
                assert!(parse_cpu_max(bad).is_err());
            }
            assert!(validate_cpu_request(0).is_err());
            assert!(validate_cpu_request(249).is_err());
            assert!(validate_cpu_request(64001).is_err());
            assert!(validate_cpu_request(250).is_ok());
        }

        #[test]
        #[ignore = "requires disposable delegated cgroup-v2 CPU and memory scope"]
        fn real_cpu_cap_applies_to_child_and_descendant_and_detects_policy_change() {
            use crate::{hosting_budget, runtime_process_guard::RuntimeProcessGuard};
            let temp = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
            let paths = swarm_core::DataPaths::from_root(temp.path().join("profile"));
            let world = swarm_protocol::WorldId([57; 32]);
            let consent = hosting_budget::set_contribution_with_cpu(&paths, world, 512, Some(500)).unwrap();
            let cap = HostMemoryCap::prepare(512, Some(500), consent.revision)
                .unwrap()
                .expect("explicit CPU must have a cap");
            // This is a scoped process-ownership fixture, not Minecraft host
            // or canonical authority admission; no machine-wide reservation
            // is taken and no real world/runtime is touched.
            let mut guard = RuntimeProcessGuard::begin_coordinated(&paths, world, None, Some(cap)).unwrap();
            let reserved = hosting_budget::contribution_readiness(&paths, world).unwrap();
            assert!(!reserved.may_host_now);
            assert!(matches!(reserved.process_cpu, ProcessCpuReport::LinuxCgroupReserved {
                limit_millicores:500, admitted_heap_mib:512, ref admitted_consent_revision
            } if admitted_consent_revision == &consent.revision.to_string()));
            assert!(hosting_budget::verify_live_cpu_consent(&paths, world, &consent).is_err());
            let marker = temp.path().join("ready");
            let stop = temp.path().join("stop");
            let mut command = Command::new("python3");
            command.args(["-c", r#"
import pathlib, subprocess, sys, time
ready, stop = map(pathlib.Path, sys.argv[1:])
child = subprocess.Popen([sys.executable, '-c', 'import time; deadline=time.monotonic()+10; exec("while time.monotonic()<deadline: pass")'])
try:
    ready.write_text(str(child.pid))
    deadline = time.monotonic()+10
    while not stop.exists() and time.monotonic() < deadline: pass
finally:
    child.terminate()
    child.wait(timeout=3)
"#]).arg(&marker).arg(&stop);
            guard.memory_cap().unwrap().configure_command(&mut command).unwrap();
            let mut child = TestChild(command.spawn().unwrap());
            guard.record_java(child.0.id()).unwrap();
            let cap = guard.memory_cap().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !marker.exists() {
                assert!(Instant::now() < deadline && child.0.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(20));
            }
            let pid = fs::read_to_string(&marker).unwrap().parse().unwrap();
            cap.verify_child(child.0.id()).unwrap();
            cap.verify_child(pid).unwrap();
            let start = Instant::now();
            let initial = counter(&cap.identity().path.join("cpu.stat"), "usage_usec").unwrap();
            let throttled_before = counter(&cap.identity().path.join("cpu.stat"), "nr_throttled").unwrap();
            std::thread::sleep(Duration::from_secs(3));
            let elapsed = start.elapsed().as_micros() as u64;
            let used = counter(&cap.identity().path.join("cpu.stat"), "usage_usec").unwrap() - initial;
            let throttled = counter(&cap.identity().path.join("cpu.stat"), "nr_throttled").unwrap() - throttled_before;
            assert!(
                used > 100_000 && used < elapsed * 7 / 10 && throttled > 0,
                "kernel CPU accounting: {used} / {elapsed} us, throttled={throttled}"
            );
            let observation = cap.identity().observe_cpu(child.0.id()).unwrap();
            assert!(matches!(observation, ProcessCpuReport::LinuxCgroupActive {
                limit_millicores: 500, admitted_heap_mib:512, ref admitted_consent_revision, ..
            } if admitted_consent_revision == &consent.revision.to_string()));
            let active = hosting_budget::contribution_readiness(&paths, world).unwrap();
            assert!(matches!(active.process_cpu, ProcessCpuReport::LinuxCgroupActive { limit_millicores: 500, .. }));
            assert!(!active.may_host_now);
            hosting_budget::verify_live_cpu_consent(&paths, world, &consent).unwrap();
            let revised = hosting_budget::set_contribution_with_cpu(&paths, world, 512, Some(750)).unwrap();
            assert!(hosting_budget::runtime_withdrawal(&paths, world, &consent).unwrap().is_some());
            assert!(hosting_budget::verify_live_cpu_consent(&paths, world, &revised).is_err());
            let revoked = hosting_budget::set_contribution_with_cpu(&paths, world, 0, Some(0)).unwrap();
            assert!(!revoked.enabled);
            let stopping = hosting_budget::contribution_readiness(&paths, world).unwrap();
            assert!(!stopping.consented && !stopping.may_host_now);
            assert!(
                matches!(stopping.process_cpu, ProcessCpuReport::LinuxCgroupActive { limit_millicores: 500, .. }),
                "withdrawal must not falsely report that the earlier runtime has already exited"
            );
            fs::write(cap.identity().path.join("cpu.max"), "max 100000\n").unwrap();
            assert!(cap.verify_child(child.0.id()).is_err());
            assert!(cap.identity().observe_cpu(child.0.id()).is_err());
            // Restore the test-only tampered leaf, then cleanly stop both
            // disposable processes. No change to parent scope or host.
            fs::write(cap.identity().path.join("cpu.max"), "50000 100000\n").unwrap();
            fs::write(&stop, b"stop").unwrap();
            assert!(child.0.wait().unwrap().success());
            assert!(!cap.identity().populated().unwrap());
            drop(guard);
            let ended = hosting_budget::contribution_readiness(&paths, world).unwrap();
            assert!(matches!(ended.process_cpu, ProcessCpuReport::Unspecified {}));
            assert!(!ended.consented && !ended.may_host_now);
            println!(
                "cpu_receipt={}",
                serde_json::json!({
                    "cpu_limit_millicores":500, "period_us":100000, "quota_us":50000,
                    "heap_mib":512,"kernel_memory_limit_mib":1536,
                    "elapsed_usec":elapsed, "combined_process_cpu_usec":used, "throttled_periods":throttled,
                    "descendant_membership_verified":true,"tampered_cpu_limit_refused":true,
                    "empty_leaf_verified":true, "prepared_reservation_observed":true,
                    "active_admission_bound_to_revision":true, "changed_consent_refused_for_live_proof":true,
                    "withdrawal_preserves_active_observation_until_exit":true,
                    "scope":"actual kernel cap on two bounded Python fixture processes, not Minecraft"
                })
            );
        }

        #[test]
        #[ignore = "requires a disposable delegated cgroup-v2 test scope"]
        fn real_cgroup_enforces_native_and_descendant_memory_before_exec() {
            let root = configured_root().unwrap().expect("delegated test scope must be configured");
            let cap = create(&root, 64 * MIB).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let ready = temp.path().join("ready");
            let mut command = Command::new("python3");
            command
                .args([
                    "-c",
                    r#"
import os, pathlib, subprocess, sys, time
group = pathlib.Path('/proc/self/cgroup').read_text()
assert group.strip().startswith('0::') and 'swarmcraft-jvm-' in group
own = bytearray(12*1024*1024)
for i in range(0, len(own), 4096): own[i] = 1
child = subprocess.Popen([sys.executable, '-c', 'import time; x=bytearray(12*1024*1024); time.sleep(120)'])
pathlib.Path(sys.argv[1]).write_text(str(child.pid))
time.sleep(120)
"#,
                ])
                .arg(&ready)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());
            cap.configure_command(&mut command).unwrap();
            let mut java = TestChild(command.spawn().unwrap());
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.exists() {
                assert!(java.0.try_wait().unwrap().is_none());
                assert!(Instant::now() < deadline, "native test child failed to start");
                std::thread::sleep(Duration::from_millis(20));
            }
            cap.verify_child(java.0.id()).unwrap();
            let descendant: u32 = fs::read_to_string(&ready).unwrap().parse().unwrap();
            cap.verify_child(descendant).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let measured = sample(cap.identity()).unwrap();
                if measured.current >= 24 * MIB {
                    println!("kernel accounted native JVM fixture and descendant: {} bytes", measured.current);
                    break;
                }
                assert!(Instant::now() < deadline, "native/child memory charges not observed");
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(cap.identity().populated().unwrap());
            // First verify the production soft-pressure setting. For the
            // separate hard-cap experiment only, disable high throttling so
            // an over-limit allocation reaches memory.max promptly rather
            // than stalling in reclaim. Production never changes this value.
            assert_eq!(read_number(&cap.identity().path.join("memory.high")).unwrap(), cap.identity().high_bytes);
            fs::write(cap.identity().path.join("memory.high"), "max\n").unwrap();
            // A second disposable allocation is deliberately run within
            // this exact test cgroup, not on the host outside its cap.
            let mut oom = Command::new("python3");
            oom.args(["-c", "x=bytearray(128*1024*1024); print(len(x))"]).stdout(Stdio::null()).stderr(Stdio::null());
            // Production configure_command refuses already populated
            // groups. Here use a test-only pre-exec attachment to create
            // deliberate pressure in the existing test group.
            let file = OpenOptions::new().write(true).open(cap.identity().path.join("cgroup.procs")).unwrap();
            unsafe {
                oom.pre_exec(move || {
                    let bytes = b"0\n";
                    if libc::write(file.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) == bytes.len() as isize {
                        Ok(())
                    } else {
                        Err(std::io::Error::last_os_error())
                    }
                });
            }
            let mut pressure = TestChild(oom.spawn().unwrap());
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if pressure.0.try_wait().unwrap().is_some() && java.0.try_wait().unwrap().is_some() {
                    break;
                }
                assert!(Instant::now() < deadline, "kernel did not enforce the isolated cgroup cap");
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(counter(&cap.identity().path.join("memory.events"), "oom_kill").unwrap() > 0);
            let deadline = Instant::now() + Duration::from_secs(10);
            while cap.identity().populated().unwrap() {
                assert!(Instant::now() < deadline, "residual processes remain after group OOM");
                std::thread::sleep(Duration::from_millis(20));
            }
            println!("kernel memory.max enforced the 64 MiB fixture limit; group OOM retired the test processes");
        }
        #[test]
        #[ignore = "requires a disposable delegated cgroup-v2 test scope"]
        fn residual_descendant_fences_runtime_reset_after_java_exit() {
            use crate::{
                machine_hosting::MachineHostingReservation,
                runtime_process_guard::{self, RuntimeProcessGuard},
            };
            use swarm_core::DataPaths;
            use swarm_protocol::WorldId;

            let root = configured_root().unwrap().expect("delegated test scope must be configured");
            let cap = create(&root, 64 * MIB).unwrap();
            let group = cap.identity().path.clone();
            let temp = tempfile::Builder::new().permissions(std::fs::Permissions::from_mode(0o700)).tempdir().unwrap();
            let paths = DataPaths::from_root(temp.path().join("data"));
            let world = WorldId([91; 32]);
            let account_dir = temp.path().join("account");
            let reservation = MachineHostingReservation::acquire_at(&account_dir).unwrap();
            let mut guard =
                RuntimeProcessGuard::begin_coordinated(&paths, world, Some(&reservation), Some(cap)).unwrap();
            let ready = temp.path().join("ready");
            let exit_parent = temp.path().join("exit-parent");
            let exit_descendant = temp.path().join("exit-descendant");
            let mut command = Command::new("python3");
            command.args(["-c", r#"
import pathlib, subprocess, sys, time
ready, exit_parent, exit_child = map(pathlib.Path, sys.argv[1:])
child = subprocess.Popen([sys.executable, '-c', 'import pathlib,sys,time; p=pathlib.Path(sys.argv[1]); t=time.monotonic()+30; exec("while not p.exists() and time.monotonic()<t: time.sleep(0.02)")', str(exit_child)])
ready.write_text(str(child.pid))
deadline = time.monotonic()+30
while not exit_parent.exists() and time.monotonic()<deadline: time.sleep(0.02)
"#]).arg(&ready).arg(&exit_parent).arg(&exit_descendant).stdout(Stdio::null()).stderr(Stdio::inherit());
            guard.memory_cap().unwrap().configure_command(&mut command).unwrap();
            let mut java = TestChild(command.spawn().unwrap());
            guard.record_java(java.0.id()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !ready.exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(20));
            }
            let observed = runtime_process_guard::observed_memory(&paths, world).unwrap().unwrap();
            assert!(matches!(observed, ProcessMemoryReport::LinuxCgroupActive { limit_mib: 64, .. }));
            fs::write(&exit_parent, "").unwrap();
            assert!(java.0.wait().unwrap().success());
            assert!(guard.verify_exit_quiescent().is_err());
            assert!(runtime_process_guard::refuse_unaccounted_runtimes(&paths)
                .unwrap_err()
                .to_string()
                .contains("descendants"));
            // Retiring the controller and its lock cannot erase live ownership.
            drop(guard);
            drop(reservation);
            assert!(MachineHostingReservation::acquire_at(&account_dir).is_err());
            fs::write(&exit_descendant, "").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if counter(&group.join("cgroup.events"), "populated").unwrap() == 0 {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(20));
            }
            let reservation = MachineHostingReservation::acquire_at(&account_dir).unwrap();
            assert!(runtime_process_guard::refuse_unaccounted_runtimes(&paths).is_ok());
            runtime_process_guard::clear_quiescent_record(
                &paths.root.join("control").join(world.to_hex()).join("runtime-process.json"),
            )
            .unwrap();
            drop(reservation);
            fs::remove_dir(group).unwrap();
            println!(
                "recorded residual descendant blocked same-root reset and account-wide readmission after JVM exit"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_process_limit_preserves_java_native_reserve() {
        assert_eq!(cap_bytes(512).unwrap(), 1536 * MIB);
        assert_eq!(cap_bytes(4096).unwrap(), 6144 * MIB);
        assert!(cap_bytes(u64::MAX).is_err());
    }
}
