//! Ignored, disposable real-Fabric lifecycle test. There is deliberately no
//! production CLI path to this launcher. The outer test owns the account
//! reservation, exact staged package, and cgroup; the inner controller and
//! Minecraft run in a fresh mapped-user, PID and loopback-only network
//! namespace. Neither controller creates a canonical permit or game route.
use crate::exclusive_turn_process_probe::fabric::{log_tail, write_new};
use crate::{
    exclusive_turn_runtime_package::StagedRuntimePackage,
    host_memory_cap::{HostMemoryCap, ProcessCpuReport, ProcessMemoryReport},
    hosting_budget,
    runtime_process_guard::{ProbeRecoveryPin, RuntimeProcessGuard},
};
use anyhow::{ensure, Context, Result};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use swarm_core::DataPaths;
use swarm_protocol::Hash32;

const HEAP: u64 = 1024;
const CPU: u64 = 2000;
const WORKER: &str = "exclusive_turn_fabric_lab::namespace_worker";

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn env_path(name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var_os(name).with_context(|| format!("missing lab input {name}"))?))
}

#[test]
fn namespace_worker() {
    // Normal library suite never launches a JVM. Only the ignored outer
    // gate provides this flag, mapped identity and disposable paths.
    if std::env::var("SC8_FABRIC_LAB_WORKER").as_deref() != Ok("1") {
        return;
    }
    crate::exclusive_turn_process_probe::fabric::run_worker(true).unwrap();
}

#[test]
#[ignore = "requires disposable reviewed Fabric package, restored test save and delegated scope"]
fn real_fabric_lifecycle_isolated_under_exact_package_and_cgroup() {
    run_outer().unwrap();
}

fn run_outer() -> Result<()> {
    ensure!(unsafe { libc::geteuid() } != 0, "lab must run under original unprivileged account");
    let root = env_path("SC8_LAB_ROOT")?.canonicalize()?;
    let package = env_path("SC8_LAB_PACKAGE")?;
    let pin: Hash32 = std::env::var("SC8_LAB_COMPATIBILITY")?.parse()?;
    let paths = DataPaths::from_root(root.join("process-profile"));
    let (_profile, _memory, machine) = hosting_budget::reserve_reference_capacity(&paths)?;
    let mut stage = StagedRuntimePackage::stage(&package, &root, pin)?;
    let mut cap = HostMemoryCap::prepare(HEAP, Some(CPU), 1)?.context("lab requires delegated cgroup")?;
    let recovery = ProbeRecoveryPin::capture(cap.identity())?;
    cap.retain_for_probe_recovery();
    stage.retain_for_possible_spawn();
    let host_net = fs::read_link("/proc/self/ns/net")?;
    let mut command = Command::new("unshare");
    command
        .args([
            "--user",
            "--map-root-user",
            "--net",
            "--pid",
            "--fork",
            "--kill-child",
            "--mount-proc",
            "--",
            "/bin/sh",
            "-ec",
            "ip link set lo up; exec \"$@\"",
            "sh",
        ])
        .arg(std::env::current_exe()?)
        .args(["--exact", WORKER, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("SC8_FABRIC_LAB_WORKER", "1")
        .env("SC8_LAB_ROOT", &root)
        .env("SC8_LAB_STAGE", stage.root())
        .env("SC8_LAB_HOST_NET", host_net)
        .env("SC8_LAB_HOST_UID", unsafe { libc::geteuid() }.to_string())
        .env("SC8_LAB_COMPATIBILITY", pin.to_string())
        .env("SC8_LAB_HEAP", HEAP.to_string())
        .env("SC8_LAB_CPU", CPU.to_string())
        .env("SC8_LAB_CLASSPATH", serde_json::to_string(&stage.probe_classpath())?)
        .stdin(Stdio::null());
    cap.configure_command(&mut command)?;
    let local = root.join("fabric-lab-process.json");
    let shared = machine.record_path.clone();
    let mut guard = RuntimeProcessGuard::begin_at(local.clone(), Some(&machine), Some(cap))?;
    stage.verify_unchanged()?;
    let mut child = Reap(command.spawn()?);
    // The directly supervised PID is the namespace wrapper, not Java.
    // Kernel limits and the final empty-cgroup proof cover all descendants.
    guard.record_java(child.0.id())?;
    let cap = guard.memory_cap().context("lab cgroup missing")?;
    let cpu = cap.identity().observe_cpu(child.0.id())?;
    let memory = cap.identity().observe(child.0.id())?;
    ensure!(
        matches!(cpu, ProcessCpuReport::LinuxCgroupActive { limit_millicores: CPU, .. })
            && matches!(memory, ProcessMemoryReport::LinuxCgroupActive { limit_mib: 2048, .. }),
        "lab kernel resource limits differ"
    );
    let deadline = Instant::now() + Duration::from_secs(170);
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        ensure!(Instant::now() < deadline, "isolated Fabric lifecycle deadline exceeded");
        if let Err(error) = cap.verify_child(child.0.id()) {
            if let Some(status) = child.0.try_wait()? {
                break status;
            }
            return Err(error);
        }
        ensure!(!cap.pressure()?, "isolated Fabric lifecycle encountered cgroup memory pressure");
        std::thread::sleep(Duration::from_millis(100));
    };
    ensure!(status.success(), "isolated Fabric worker failed: {status}\n{}", log_tail(&root.join("fabric.log")));
    ensure!(!cap.pressure()?, "isolated Fabric lifecycle recorded memory pressure");
    guard.verify_exit_quiescent()?;
    stage.reverify()?;
    drop(guard);
    ensure!(!local.exists() && !shared.exists(), "lab process ownership records remain");
    recovery.retire_completed_leaf()?;
    stage.retire_completed()?;
    write_new(
        &root.join("fabric-supervision.json"),
        &serde_json::to_vec(&json!({
            "classification":"disposable_fabric_process_boundary", "process_cpu":cpu,"process_memory":memory,
            "exact_staged_package_reverified":true,"process_and_descendants_quiescent":true,"ownership_records_retired":true,
            "canonical_authority":false,"runtime_activation":false,"player_admission":false,
        }))?,
    )?;
    Ok(())
}
