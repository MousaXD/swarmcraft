//! A fixed, isolated Java qualification process, not a Minecraft launch.
//! The caller must durably fence its reference intent before `start` and
//! serialize spawn with its signed local consent. Shares the canonical
//! account/profile exclusion and mandatory kernel CPU/memory limits.
use crate::{
    exclusive_turn_runtime_package::inspect_runtime_package,
    host_memory_cap::{self, HostMemoryCap, ProcessCpuReport, ProcessMemoryReport},
    hosting_budget,
    machine_hosting::{
        linux::{uid, validate_ancestors},
        MachineHostingReservation,
    },
    runtime_process_guard::RuntimeProcessGuard,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use swarm_core::DataPaths;
use swarm_protocol::Hash32;

const OUTPUT_LIMIT: u64 = 64 * 1024;
const PROBE_SOURCE: &[u8] = br#"class SwarmCraftProbe {
    public static void main(String[] args) throws Exception {
        if (System.in.read() != 83) System.exit(11);
        System.out.println("SWARMCRAFT_JAVA_VERSION=" + System.getProperty("java.version"));
    }
}
"#;

pub struct ProbeAdmission {
    _profile: File,
    _memory: File,
    machine: MachineHostingReservation,
    cap: Option<HostMemoryCap>,
    package: PathBuf,
    expected_pin: Hash32,
    java_version: String,
    heap_mib: u64,
    cpu_millicores: u64,
    consent_revision: u64,
}

#[derive(Debug, Serialize)]
pub struct ProbeReport {
    pub java_version: String,
    pub process_cpu: ProcessCpuReport,
    pub process_memory: ProcessMemoryReport,
    pub elapsed_ms: u64,
    pub exact_package_reverified: bool,
    pub process_and_descendants_quiescent: bool,
}

impl ProbeAdmission {
    pub fn prepare(
        paths: &DataPaths,
        package: &Path,
        expected_pin: Hash32,
        heap_mib: u64,
        cpu_millicores: u64,
        consent_revision: u64,
    ) -> Result<Self> {
        ensure!(consent_revision > 0, "invalid probe consent revision");
        // Never launch an operator's system Java or a package with a
        // different fingerprint. This is a cooperative local trust model,
        // not isolation from arbitrary code written by this same account.
        let reviewed = inspect_runtime_package(package)?;
        ensure!(reviewed.fingerprint == expected_pin, "probe runtime package differs from signed compatibility");
        let package = package.canonicalize()?;
        host_memory_cap::require_cpu_configuration(Some(cpu_millicores))?;
        hosting_budget::check_budget(heap_mib, hosting_budget::memory_snapshot()?)?;
        let (profile, memory, machine) = hosting_budget::reserve_reference_capacity(paths)?;
        hosting_budget::check_budget(heap_mib, hosting_budget::memory_snapshot()?)?;
        let cap = HostMemoryCap::prepare(heap_mib, Some(cpu_millicores), consent_revision)?
            .context("reference probe requires a delegated CPU and memory cgroup; no heap-only fallback")?;
        Ok(Self {
            _profile: profile,
            _memory: memory,
            machine,
            cap: Some(cap),
            package,
            expected_pin,
            java_version: reviewed.java_version,
            heap_mib,
            cpu_millicores,
            consent_revision,
        })
    }

    /// The caller must have published a possibly-spawned intent and must
    /// still own its consent lock. Any failure leaves that intent fenced.
    pub fn start(&mut self, directory: &Path) -> Result<RunningProbe<'_>> {
        hosting_budget::check_budget(self.heap_mib, hosting_budget::memory_snapshot()?)?;
        self.start_checked(directory)
    }

    fn start_checked(&mut self, directory: &Path) -> Result<RunningProbe<'_>> {
        validate_ancestors(directory)?;
        let meta = fs::symlink_metadata(directory)?;
        ensure!(
            meta.is_dir() && meta.uid() == uid() && meta.permissions().mode() & 0o077 == 0,
            "probe requires owner-only reference directory"
        );
        let directory = directory.canonicalize()?;
        let local_record = directory.join("probe-process.json");
        let shared_record = self.machine.record_path.clone();
        let work = tempfile::Builder::new()
            .prefix("probe-work-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&directory)?;
        let source = work.path().join("SwarmCraftProbe.java");
        let mut input = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&source)?;
        input.write_all(PROBE_SOURCE)?;
        input.sync_all()?;
        drop(input);
        let stdout = new_output(work.path().join("stdout"))?;
        let stderr = new_output(work.path().join("stderr"))?;
        let mut command = Command::new(self.package.join("java/bin/java"));
        command
            .current_dir(work.path())
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("HOME", work.path())
            .env("TMPDIR", work.path())
            .arg("-Xms16m")
            .arg(format!("-Xmx{}m", self.heap_mib))
            .args(["-XX:+UseSerialGC", "-XX:-UsePerfData"])
            .arg(format!("-Djava.io.tmpdir={}", work.path().display()))
            .arg(format!("-Duser.home={}", work.path().display()))
            .arg(&source)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        // Restrict individual output files and core dumps before exec. The
        // child inherits its dedicated cgroup before Java begins executing.
        // SAFETY: only async-signal-safe setrlimit calls in the child.
        unsafe {
            command.pre_exec(|| {
                for (resource, value) in [(libc::RLIMIT_FSIZE, OUTPUT_LIMIT), (libc::RLIMIT_CORE, 0)] {
                    let limit = libc::rlimit { rlim_cur: value, rlim_max: value };
                    if libc::setrlimit(resource, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let cap = self.cap.take().context("probe admission already consumed")?;
        cap.configure_command(&mut command)?;
        let mut guard = RuntimeProcessGuard::begin_at(local_record.clone(), Some(&self.machine), Some(cap))?;
        let mut child = ProbeChild(command.spawn().context("cannot spawn reviewed Java qualification process")?);
        guard.record_java(child.0.id())?;
        let cap = guard.memory_cap().context("mandatory probe cgroup missing")?;
        let cpu = cap.identity().observe_cpu(child.0.id())?;
        let memory = cap.identity().observe(child.0.id())?;
        ensure!(
            matches!(&cpu, ProcessCpuReport::LinuxCgroupActive {limit_millicores, admitted_heap_mib, admitted_consent_revision, ..}
            if *limit_millicores == self.cpu_millicores && *admitted_heap_mib == self.heap_mib && admitted_consent_revision == &self.consent_revision.to_string()),
            "observed probe CPU policy differs from consent"
        );
        ensure!(matches!(memory, ProcessMemoryReport::LinuxCgroupActive { .. }), "probe memory limit is not active");
        ensure!(child.0.try_wait()?.is_none(), "Java qualification process exited before start barrier");
        Ok(RunningProbe {
            child,
            guard,
            work,
            local_record,
            shared_record,
            cpu,
            memory,
            package: self.package.clone(),
            expected_pin: self.expected_pin,
            java_version: self.java_version.clone(),
            start: Instant::now(),
        })
    }
}

fn new_output(path: PathBuf) -> Result<File> {
    Ok(OpenOptions::new().create_new(true).write(true).mode(0o600).open(path)?)
}

fn read_output(path: &Path) -> Result<String> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.nlink() == 1
            && meta.uid() == uid()
            && meta.mode() & 0o077 == 0
            && meta.len() <= OUTPUT_LIMIT,
        "unsafe or oversized Java probe output"
    );
    let mut bytes = Vec::new();
    file.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= OUTPUT_LIMIT, "Java probe output exceeds limit");
    Ok(String::from_utf8(bytes)?)
}

struct ProbeChild(Child);
impl Drop for ProbeChild {
    fn drop(&mut self) {
        // Only this directly spawned child is addressed; unknown residual
        // descendants remain fenced by the durable cgroup ownership record.
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub struct RunningProbe<'a> {
    // Drop order is important: kill/reap the owned child before releasing
    // the durable process guard and before removing the scratch directory.
    child: ProbeChild,
    guard: RuntimeProcessGuard<'a>,
    work: tempfile::TempDir,
    local_record: PathBuf,
    shared_record: PathBuf,
    cpu: ProcessCpuReport,
    memory: ProcessMemoryReport,
    package: PathBuf,
    expected_pin: Hash32,
    java_version: String,
    start: Instant,
}

fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => anyhow::bail!("durable probe ownership record remains; refusing intent retirement"),
    }
}

impl RunningProbe<'_> {
    pub fn complete(mut self, mut check_consent: impl FnMut() -> Result<()>) -> Result<ProbeReport> {
        check_consent()?;
        let mut gate = self.child.0.stdin.take().context("Java start barrier missing")?;
        gate.write_all(b"S")?;
        drop(gate);
        let deadline = self.start + Duration::from_secs(60);
        let status = loop {
            check_consent()?;
            if let Some(status) = self.child.0.try_wait()? {
                break status;
            }
            ensure!(Instant::now() < deadline, "Java qualification deadline exceeded");
            let cap = self.guard.memory_cap().context("probe cgroup missing")?;
            if let Err(error) = cap.verify_child(self.child.0.id()) {
                // A normal exit can race the procfs membership sample.
                // Still demand the exact cgroup policy and empty subtree
                // before accepting the observed exit.
                if let Some(status) = self.child.0.try_wait()? {
                    ensure!(!cap.pressure()?, "resource pressure during Java qualification");
                    self.guard.verify_exit_quiescent()?;
                    break status;
                }
                return Err(error);
            }
            ensure!(
                !cap.pressure()? && !hosting_budget::reference_runtime_memory_pressure()?,
                "resource pressure requires stopping Java qualification"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        ensure!(
            !self.guard.memory_cap().context("probe cgroup missing")?.pressure()?,
            "resource pressure during Java qualification"
        );
        self.guard.verify_exit_quiescent()?;
        let out = read_output(&self.work.path().join("stdout"))?;
        let err = read_output(&self.work.path().join("stderr"))?;
        ensure!(
            status.success() && err.is_empty() && out == format!("SWARMCRAFT_JAVA_VERSION={}\n", self.java_version),
            "Java qualification exited unsuccessfully or reported an unexpected version"
        );
        let reviewed = inspect_runtime_package(&self.package)?;
        ensure!(
            reviewed.fingerprint == self.expected_pin && reviewed.java_version == self.java_version,
            "runtime package changed during Java qualification"
        );
        check_consent()?;
        // Keep the account/profile reservation borrowed until both ownership
        // records have actually retired. A failed deletion is a hard fence.
        drop(self.guard);
        absent(&self.local_record)?;
        absent(&self.shared_record)?;
        Ok(ProbeReport {
            java_version: self.java_version,
            process_cpu: self.cpu,
            process_memory: self.memory,
            elapsed_ms: self.start.elapsed().as_millis().min(u64::MAX as u128) as u64,
            exact_package_reverified: true,
            process_and_descendants_quiescent: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // This exercises the actual process boundary in a disposable delegated
    // scope without pretending the machine has full game/client headroom.
    // Production prepare/start still require the full hosting budget.
    #[test]
    #[ignore = "requires reviewed test package and disposable delegated CPU/memory scope"]
    fn real_java_probe_owns_kernel_resources_and_retires_exact_records() {
        let package =
            PathBuf::from(std::env::var_os("SWARMCRAFT_TEST_PROBE_PACKAGE").expect("reviewed test package required"));
        let reviewed = inspect_runtime_package(&package).unwrap();
        let temp = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("profile"));
        let other = DataPaths::from_root(temp.path().join("other-profile"));
        let directory = temp.path().join("reference");
        swarm_core::create_private_dir_all(&directory).unwrap();
        let make = || {
            let (profile, memory, machine) = hosting_budget::reserve_reference_capacity(&paths).unwrap();
            let cap = HostMemoryCap::prepare(512, Some(500), 1).unwrap().expect("delegated cap");
            ProbeAdmission {
                _profile: profile,
                _memory: memory,
                machine,
                cap: Some(cap),
                package: package.canonicalize().unwrap(),
                expected_pin: reviewed.fingerprint,
                java_version: reviewed.java_version.clone(),
                heap_mib: 512,
                cpu_millicores: 500,
                consent_revision: 1,
            }
        };
        let mut admission = make();
        let shared = admission.machine.record_path.clone();
        let running = admission.start_checked(&directory).unwrap();
        assert!(directory.join("probe-process.json").exists() && shared.exists());
        assert!(
            hosting_budget::reserve_reference_capacity(&other).is_err(),
            "second profile escaped account reservation"
        );
        let report = running.complete(|| Ok(())).unwrap();
        assert!(report.exact_package_reverified && report.process_and_descendants_quiescent);
        assert_eq!(report.java_version, reviewed.java_version);
        assert!(matches!(report.process_cpu, ProcessCpuReport::LinuxCgroupActive { limit_millicores: 500, .. }));
        assert!(matches!(report.process_memory, ProcessMemoryReport::LinuxCgroupActive { limit_mib: 1536, .. }));
        assert!(!directory.join("probe-process.json").exists() && !shared.exists());
        println!("actual isolated Java process: {}", serde_json::to_string(&report).unwrap());
        drop(admission);
        // Withdrawal before releasing the child's stdin gate: kill/reap
        // the known child, retire process records only after quiescence.
        let mut second = make();
        let running = second.start_checked(&directory).unwrap();
        let pid = running.child.0.id();
        assert!(running.complete(|| anyhow::bail!("test consent withdrawn")).is_err());
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        assert!(!directory.join("probe-process.json").exists() && !shared.exists());
        assert!(!paths.worlds_dir().exists());
        println!("withdrawal before Java gate: child reaped, both records retired; no canonical world");
    }
}
