//! Optional Linux delegated cgroup-v2 memory enforcement for the managed JVM
//! and its descendants. No changes to the operator's parent controllers,
//! no attachment after exec, and no fallback on a configured cap failure.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Command};

pub const HOST_CGROUP_ROOT_ENV: &str = "SWARMCRAFT_HOST_CGROUP_ROOT";
const MIB: u64 = 1024 * 1024;

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
}

pub(crate) struct HostMemoryCap {
    identity: MemoryCapIdentity,
    claimed_by_record: bool,
}

impl HostMemoryCap {
    pub(crate) fn prepare(heap_mib: u64) -> Result<Option<Self>> {
        let Some(root) = configured_root()? else { return Ok(None) };
        #[cfg(target_os = "linux")]
        {
            let limit = cap_bytes(heap_mib)?;
            linux::create(&root, limit).map(Some)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (root, heap_mib);
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

    fn high_limit(limit: u64) -> Result<u64> {
        // cgroup-v2 memory values are rounded to the kernel page size.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        ensure!(page > 0, "kernel page size is unavailable");
        let page = page as u64;
        Ok((limit.checked_mul(9).context("cgroup high budget overflow")? / 10 / page) * page)
    }

    pub(super) fn create(root: &Path, limit_bytes: u64) -> Result<HostMemoryCap> {
        validate_delegated_root(root)?;
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
            };
            ensure!(!populated(&identity)?, "new hosting cgroup unexpectedly contains processes");
            // Hard kernel charge limit covers anonymous/native memory,
            // charged file cache and descendants; prohibit extra swap.
            write_number(&path.join("memory.max"), limit_bytes)?;
            write_number(&path.join("memory.high"), high_bytes)?;
            write_number(&path.join("memory.swap.max"), 0)?;
            write_number(&path.join("memory.oom.group"), 1)?;
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
