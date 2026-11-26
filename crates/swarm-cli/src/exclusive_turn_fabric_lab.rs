//! Ignored, disposable real-Fabric lifecycle test. There is deliberately no
//! production CLI path to this launcher. The outer test owns the account
//! reservation, exact staged package, and cgroup; the inner controller and
//! Minecraft run in a fresh mapped-user, PID and loopback-only network
//! namespace. Neither controller creates a canonical permit or game route.
use crate::{
    exclusive_turn_compatibility::verify_level_version,
    exclusive_turn_runtime_package::StagedRuntimePackage,
    host_memory_cap::{HostMemoryCap, ProcessCpuReport, ProcessMemoryReport},
    hosting_budget,
    runtime_process_guard::{ProbeRecoveryPin, RuntimeProcessGuard},
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{fs::OpenOptionsExt, io::AsRawFd, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use swarm_core::{random_nonce, DataPaths};
use swarm_ipc::{FabricBridgeListener, IpcTransportError};
use swarm_protocol::Hash32;

const HEAP: u64 = 1024;
const CPU: u64 = 2000;
const PORT: u16 = 45183;
const LOG_LIMIT: u64 = 8 * 1024 * 1024;
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

fn output_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().create_new(true).write(true).mode(0o600).open(path)?)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = output_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn log_text(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)?.take(LOG_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LOG_LIMIT, "lab log exceeded byte bound");
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn log_tail(path: &Path) -> String {
    log_text(path)
        .map(|s| s.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

fn session_locked(world: &Path) -> Result<bool> {
    let file =
        OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOFOLLOW).open(world.join("session.lock"))?;
    ensure!(file.metadata()?.is_file(), "unsafe lab session lock");
    // Java FileChannel uses POSIX record locks, not flock.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    // SAFETY: valid descriptor and initialized lock structure.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) } == 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EACCES | libc::EAGAIN) => Ok(true),
        _ => Err(error.into()),
    }
}

fn check_namespace() -> Result<()> {
    ensure!(
        std::process::id() == 1 && unsafe { libc::geteuid() } == 0,
        "lab requires isolated mapped-root PID namespace"
    );
    let host_uid = std::env::var("SC8_LAB_HOST_UID")?.parse::<u32>()?;
    ensure!(host_uid != 0, "lab refuses real root");
    let map = fs::read_to_string("/proc/self/uid_map")?;
    let fields: Vec<_> = map.split_whitespace().collect();
    ensure!(fields == ["0", &host_uid.to_string(), "1"], "lab user mapping differs");
    let current = fs::read_link("/proc/self/ns/net")?;
    ensure!(current != env_path("SC8_LAB_HOST_NET")?, "lab reused host network namespace");
    let ip = |args: &[&str]| -> Result<Value> {
        let out = Command::new("ip").args(args).output()?;
        ensure!(out.status.success(), "cannot inspect isolated network");
        Ok(serde_json::from_slice(&out.stdout)?)
    };
    let links = ip(&["-j", "link", "show"])?;
    let links = links.as_array().context("invalid link report")?;
    ensure!(links.len() == 1 && links[0]["ifname"] == "lo", "lab has a non-loopback interface");
    for args in [["-j", "-4", "route", "show"], ["-j", "-6", "route", "show"]] {
        ensure!(ip(&args)?.as_array().is_some_and(|v| v.is_empty()), "lab has a route outside loopback");
    }
    Ok(())
}

#[test]
fn namespace_worker() {
    // Normal library suite never launches a JVM. Only the ignored outer
    // gate provides this flag, mapped identity and disposable paths.
    if std::env::var("SC8_FABRIC_LAB_WORKER").as_deref() != Ok("1") {
        return;
    }
    run_worker().unwrap();
}

fn run_worker() -> Result<()> {
    check_namespace()?;
    let root = env_path("SC8_LAB_ROOT")?;
    let stage = env_path("SC8_LAB_STAGE")?;
    let pin: Hash32 = std::env::var("SC8_LAB_COMPATIBILITY")?.parse()?;
    let runtime = root.join("runtime");
    let world = runtime.join("world");
    ensure!(world.is_dir() && !session_locked(&world)?, "initial restored fixture must be stopped");
    verify_level_version(&fs::read(world.join("level.dat"))?, 4790)?;
    let classpath: Vec<String> = serde_json::from_str(&std::env::var("SC8_LAB_CLASSPATH")?)?;
    ensure!(classpath.len() == 48, "unexpected reviewed runtime JAR count");
    let classpath = std::env::join_paths(
        classpath.iter().filter(|p| p.as_str() != "mods/swarmcraft-fabric.jar").map(|p| stage.join(p)),
    )?;
    write_new(&runtime.join("eula.txt"), b"eula=true\n")?;
    write_new(&runtime.join("server.properties"), format!("server-ip=127.0.0.1\nserver-port={PORT}\nlevel-name=world\nonline-mode=true\nenforce-secure-profile=true\nwhite-list=true\nenforce-whitelist=true\nenable-rcon=false\nenable-query=false\nmanagement-server-enabled=false\nenable-jmx-monitoring=false\nview-distance=2\nsimulation-distance=2\nmax-players=1\n").as_bytes())?;
    for name in ["ops.json", "whitelist.json"] {
        write_new(&runtime.join(name), b"[]\n")?;
    }
    let log = root.join("fabric.log");
    let stdout = output_file(&log)?;
    let stderr = stdout.try_clone()?;
    let executor = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    executor.block_on(async {
        let listener = FabricBridgeListener::bind().await?;
        let launch = listener.launch_config()?;
        let mut command = Command::new(stage.join("java/bin/java"));
        command.current_dir(&runtime).env_clear().env("LANG", "C").env("LC_ALL", "C")
            .env("HOME", &runtime).env("TMPDIR", &runtime)
            .envs(launch.environment()).env("SWARMCRAFT_WORLD_DIR", &world)
            .env("SWARMCRAFT_COMPAT_FINGERPRINT", pin.to_string())
            // Do not create any authority permit. The real bridge must
            // refuse readiness. Its normal watchdog stays enabled.
            .env("SWARMCRAFT_REQUIRE_AUTHORITY_PERMIT", "true")
            .args(["-Xms128m", "-Xmx1024m", "-XX:ActiveProcessorCount=2", "-XX:+UseSerialGC", "-XX:-UsePerfData"])
            .arg(format!("-Djava.io.tmpdir={}", runtime.display()))
            .arg(format!("-Duser.home={}", runtime.display()))
            .arg(format!("-Dfabric.gameJarPath={}", stage.join("versions/26.1.2/server-26.1.2.jar").display()))
            .arg(format!("-Dfabric.addMods={}", stage.join("mods/swarmcraft-fabric.jar").display()))
            .arg("-cp").arg(classpath).args(["net.fabricmc.loader.impl.launch.knot.KnotServer", "nogui"])
            .stdin(Stdio::piped()).stdout(Stdio::from(stdout)).stderr(Stdio::from(stderr));
        // SAFETY: async-signal-safe setrlimit calls only before exec.
        unsafe { command.pre_exec(|| {
            for (resource, value) in [(libc::RLIMIT_FSIZE, LOG_LIMIT), (libc::RLIMIT_CORE, 0)] {
                let limit = libc::rlimit { rlim_cur: value, rlim_max: value };
                if libc::setrlimit(resource, &limit) != 0 { return Err(std::io::Error::last_os_error()); }
            }
            Ok(())
        }); }
        let started = Instant::now();
        let mut java = Reap(command.spawn()?);
        let mut session = listener.accept(Duration::from_secs(100)).await.with_context(|| format!("Fabric handshake failed:\n{}", log_tail(&log)))?;
        let info = session.world_info();
        ensure!(info.minecraft_version == "26.1.2" && info.fabric_loader_version == "0.19.3" && info.java_major == 25
            && Path::new(&info.world_directory) == world && info.compatibility_fingerprint == pin, "authenticated Fabric world info differs");
        ensure!(session_locked(&world)?, "running Minecraft did not hold its Java session lock");
        let readiness = session.runtime_challenge(1, random_nonce(), Duration::from_millis(1500)).await;
        ensure!(matches!(readiness, Err(IpcTransportError::RemoteError {request_id:1, ref code}) if code == "RUNTIME_UNAVAILABLE"), "missing-permit readiness did not refuse: {readiness:?}");
        // Actual deterministic world data mutation, not a synthetic byte edit.
        let mut input = java.0.stdin.take().context("Minecraft console missing")?;
        input.write_all(b"scoreboard objectives add sc8marker dummy\nscoreboard players set sc8_checkpoint sc8marker 2601\n")?;
        input.flush()?;
        let deadline = Instant::now()+Duration::from_secs(7);
        loop {
            let current = log_text(&log)?;
            if current.lines().any(|line| line.contains("sc8_checkpoint") && line.contains("2601")) { break; }
            ensure!(Instant::now()<deadline && java.0.try_wait()?.is_none(), "real console mutation did not complete:\n{}", log_tail(&log));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        session.save_barrier(2, Duration::from_secs(8)).await?;
        session.prepare_shutdown(3, Duration::from_secs(8)).await?;
        let deadline = Instant::now()+Duration::from_secs(45);
        let status = loop {
            if let Some(status) = java.0.try_wait()? { break status; }
            ensure!(Instant::now()<deadline,"Minecraft shutdown deadline:\n{}",log_tail(&log));
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        ensure!(status.success(),"Minecraft shutdown failed: {status}\n{}",log_tail(&log));
        ensure!(!session_locked(&world)?, "stopped Minecraft still holds session lock");
        verify_level_version(&fs::read(world.join("level.dat"))?,4790)?;
        ensure!(log_text(&log)?.contains("SwarmCraft authority permit guard enabled"),"actual canonical guard was not enabled");
        ensure!(!runtime.join("authority.permit").exists(),"unexpected lab permit");
        write_new(&root.join("fabric-lifecycle.json"), &serde_json::to_vec(&json!({
            "classification":"isolated_fabric_lifecycle_not_live_authority", "minecraft":"26.1.2", "fabric_loader":"0.19.3",
            "java_major":25, "compatibility":pin.to_string(), "private_network_namespace":true,
            "authenticated_world_info":true,"missing_permit_readiness_refused":true,"active_session_lock_verified":true,
            "save_barrier_acknowledged":true,"shutdown_barrier_acknowledged":true,"clean_java_exit":true,
            "offline_session_lock_verified":true,"elapsed_ms":started.elapsed().as_millis(),
            "canonical_authority":false,"player_admission":false,"minecraft_gameplay_verified":false,
        }))?)?;
        Ok::<_,anyhow::Error>(())
    })
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
