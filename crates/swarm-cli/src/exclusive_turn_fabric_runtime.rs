//! Shared isolated Fabric lifecycle. No live route, canonical permit or player
//! admission. The production outer controller must durably sign the exact
//! process intent before spawning and gate the namespace worker until its
//! kernel ownership is recorded. The test-only caller uses disposable data.
use super::*;
use crate::exclusive_turn_compatibility::verify_level_version;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::os::unix::io::AsRawFd;
use swarm_core::random_nonce;
use swarm_ipc::{FabricBridgeListener, IpcTransportError};
const PORT: u16 = 45183;
const LOG_LIMIT: u64 = 8 * 1024 * 1024;

const WORK_PREFIX: &str = "fabric-work-";
pub const WORKER_ARGUMENT: &str = "--internal-exclusive-turn-fabric-worker";

/// Exact local workspace identity. This is never a transferable grant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricWorkspacePin {
    name: String,
    device: u64,
    inode: u64,
    world_device: u64,
    world_inode: u64,
}

fn private_directory(path: &Path) -> Result<fs::Metadata> {
    validate_ancestors(path)?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(meta.is_dir() && meta.uid() == uid() && meta.mode() & 0o077 == 0, "unsafe isolated Fabric workspace");
    Ok(meta)
}

fn durable_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    private_directory(path)?;
    File::open(path.parent().context("missing workspace parent")?)?.sync_all()?;
    Ok(())
}

impl FabricWorkspacePin {
    pub fn validate(&self) -> Result<()> {
        let suffix = self.name.strip_prefix(WORK_PREFIX).context("invalid Fabric workspace name")?;
        ensure!(
            suffix.len() == 64
                && suffix.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                && self.inode > 0
                && self.world_inode > 0,
            "invalid isolated Fabric workspace identity"
        );
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn capture(root: &Path) -> Result<Self> {
        let meta = private_directory(root)?;
        let world = private_directory(&root.join("sandbox/runtime/world"))?;
        let pin = Self {
            name: root.file_name().and_then(|n| n.to_str()).context("invalid workspace path")?.into(),
            device: meta.dev(),
            inode: meta.ino(),
            world_device: world.dev(),
            world_inode: world.ino(),
        };
        pin.validate()?;
        Ok(pin)
    }

    fn verify(&self, root: &Path) -> Result<()> {
        ensure!(&Self::capture(root)? == self, "Fabric workspace or restored world root was replaced");
        Ok(())
    }
}

pub struct FabricWorkspace {
    root: PathBuf,
}

impl FabricWorkspace {
    /// No reuse or automatic cleanup of uncertain runtime files. Retain
    /// the exact generation on every failure for explicit operator review.
    pub fn create(parent: &Path) -> Result<Self> {
        private_directory(parent)?;
        let parent = parent.canonicalize()?;
        let mut count = 0;
        for entry in fs::read_dir(&parent)? {
            if entry?.file_name().to_string_lossy().starts_with(WORK_PREFIX) {
                count += 1;
            }
            ensure!(count < 32, "isolated Fabric workspace retention limit reached; review retained files");
        }
        ensure!(count < 32, "isolated Fabric workspace retention limit reached; review retained files");
        let root = parent.join(format!("{WORK_PREFIX}{}", hex::encode(random_nonce())));
        durable_dir(&root)?;
        durable_dir(&root.join("sandbox"))?;
        durable_dir(&root.join("sandbox/runtime"))?;
        Ok(Self { root })
    }

    pub fn world(&self) -> PathBuf {
        self.root.join("sandbox/runtime/world")
    }
    pub fn bundle(&self) -> PathBuf {
        self.root.join("completed-bundle")
    }
    pub fn pin(&self) -> Result<FabricWorkspacePin> {
        FabricWorkspacePin::capture(&self.root)
    }
    pub fn verify(&self, pin: &FabricWorkspacePin) -> Result<()> {
        pin.verify(&self.root)
    }
    pub fn cleanup(self, pin: &FabricWorkspacePin) -> Result<()> {
        self.verify(pin)?;
        fs::remove_dir_all(&self.root)?;
        File::open(self.root.parent().context("missing workspace parent")?)?.sync_all()?;
        Ok(())
    }
}

fn read_level(world: &Path) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(world.join("level.dat"))?;
    ensure!(file.metadata()?.is_file(), "invalid isolated level.dat");
    let limit = crate::exclusive_turn_compatibility::MAX_LEVEL_COMPRESSED_BYTES as u64;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "isolated level.dat exceeds bound");
    Ok(bytes)
}

/// This entrypoint only performs the namespace-local lifecycle. It cannot
/// load a journal, sign a checkpoint, issue an authority permit or create
/// a live route. The parent is the sole durable mutation controller.
pub fn worker_entry() -> Result<()> {
    run_worker(false)
}

#[derive(Debug, Serialize)]
pub struct CompletedFabric {
    pub lifecycle: Value,
    pub process_cpu: ProcessCpuReport,
    pub process_memory: ProcessMemoryReport,
    pub evidence_digest: Hash32,
}

pub struct RunningFabric<'a> {
    child: ProbeChild,
    guard: RuntimeProcessGuard<'a>,
    workspace: &'a FabricWorkspace,
    workspace_pin: FabricWorkspacePin,
    stage: &'a StagedRuntimePackage,
    compatibility: Hash32,
    local_record: PathBuf,
    shared_record: PathBuf,
    cpu: ProcessCpuReport,
    memory: ProcessMemoryReport,
    started: Instant,
}

impl ProbeAdmission {
    pub fn verify_fabric_launch_package(&self) -> Result<()> {
        self.stage.verify_launch_classpath()
    }
    /// Only after the caller signs/fsyncs the exact possibly-spawned Fabric
    /// intent and while it holds the consent lock. No production fallback.
    pub fn start_fabric<'a>(
        &'a mut self,
        directory: &Path,
        workspace: &'a FabricWorkspace,
        pin: &FabricWorkspacePin,
        compatibility: Hash32,
    ) -> Result<RunningFabric<'a>> {
        hosting_budget::check_budget(self.heap_mib, hosting_budget::memory_snapshot()?)?;
        self.start_fabric_checked(directory, workspace, pin, compatibility, &std::env::current_exe()?)
    }

    fn start_fabric_checked<'a>(
        &'a mut self,
        directory: &Path,
        workspace: &'a FabricWorkspace,
        pin: &FabricWorkspacePin,
        compatibility: Hash32,
        worker_executable: &Path,
    ) -> Result<RunningFabric<'a>> {
        ensure!(self.recovery_pin_prepared, "signed Fabric recovery pin must precede spawn");
        let directory = directory.canonicalize()?;
        ensure!(
            self.stage.root().parent() == Some(directory.as_path())
                && workspace.root.parent() == Some(directory.as_path()),
            "isolated runtime belongs to another reference directory"
        );
        self.stage.pin().verify_compatibility(compatibility)?;
        self.stage.verify_unchanged()?;
        workspace.verify(pin)?;
        ensure!(!session_locked(&workspace.world())?, "restored world is already open");
        let gate = random_nonce();
        let host_net = fs::read_link("/proc/self/ns/net")?;
        let host_uid = uid();
        ensure!(host_uid != 0, "isolated Fabric refuses real root");
        let mut command = Command::new("/usr/bin/unshare");
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
            .arg(worker_executable)
            .arg(WORKER_ARGUMENT)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("SC8_LAB_ROOT", &workspace.root)
            .env("SC8_LAB_STAGE", self.stage.root())
            .env("SC8_LAB_HOST_NET", host_net)
            .env("SC8_LAB_HOST_UID", host_uid.to_string())
            .env("SC8_LAB_COMPATIBILITY", compatibility.to_string())
            .env("SC8_LAB_HEAP", self.heap_mib.to_string())
            .env("SC8_LAB_CPU", self.cpu_millicores.to_string())
            .env("SC8_LAB_GATE", hex::encode(gate))
            .env("SC8_LAB_CLASSPATH", serde_json::to_string(&self.stage.probe_classpath())?)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(output_file(&workspace.root.join("worker-errors"))?));
        let cap = self.cap.take().context("Fabric admission already consumed")?;
        cap.configure_command(&mut command)?;
        let local_record = directory.join("fabric-process.json");
        let shared_record = self.machine.record_path.clone();
        let mut guard = RuntimeProcessGuard::begin_at(local_record.clone(), Some(&self.machine), Some(cap))?;
        let mut child = ProbeChild(command.spawn().context("cannot create isolated Fabric namespace")?);
        guard.record_java(child.0.id())?;
        let cap = guard.memory_cap().context("mandatory Fabric cgroup missing")?;
        let cpu = cap.identity().observe_cpu(child.0.id())?;
        let memory = cap.identity().observe(child.0.id())?;
        ensure!(
            matches!(&cpu, ProcessCpuReport::LinuxCgroupActive {limit_millicores, admitted_heap_mib, admitted_consent_revision, ..}
            if *limit_millicores == self.cpu_millicores && *admitted_heap_mib == self.heap_mib && admitted_consent_revision == &self.consent_revision.to_string())
                && matches!(memory, ProcessMemoryReport::LinuxCgroupActive { .. }),
            "Fabric process kernel policy differs from consent"
        );
        // The caller still owns the signed consent lock. Do not release it
        // until the worker has actually spawned Java, not merely a wrapper.
        let mut input = child.0.stdin.take().context("missing Fabric start gate")?;
        input.write_all(&gate)?;
        drop(input);
        let deadline = Instant::now() + Duration::from_secs(12);
        let marker = workspace.root.join("fabric-child-started");
        loop {
            match fs::symlink_metadata(&marker) {
                Ok(_) => {
                    ensure!(read_private_evidence(&marker, 32)? == gate, "Fabric child start acknowledgment differs");
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            ensure!(
                Instant::now() < deadline && child.0.try_wait()?.is_none(),
                "isolated Fabric child did not start: {}",
                log_tail(&workspace.root.join("worker-errors"))
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(RunningFabric {
            child,
            guard,
            workspace,
            workspace_pin: pin.clone(),
            stage: &self.stage,
            compatibility,
            local_record,
            shared_record,
            cpu,
            memory,
            started: Instant::now(),
        })
    }
}

impl RunningFabric<'_> {
    /// A revoked consent, timeout or ambiguous result leaves durable intent
    /// fenced; the child guard reaps only its own namespace wrapper.
    pub fn complete(mut self, mut check_consent: impl FnMut() -> Result<()>) -> Result<CompletedFabric> {
        check_consent()?;
        self.workspace.verify(&self.workspace_pin)?;
        self.stage.verify_unchanged()?;
        let deadline = self.started + Duration::from_secs(170);
        let status = loop {
            check_consent()?;
            if let Some(status) = self.child.0.try_wait()? {
                break status;
            }
            ensure!(Instant::now() < deadline, "isolated Fabric lifecycle deadline exceeded");
            let cap = self.guard.memory_cap().context("Fabric cgroup missing")?;
            if let Err(error) = cap.verify_child(self.child.0.id()) {
                if let Some(status) = self.child.0.try_wait()? {
                    break status;
                }
                return Err(error);
            }
            ensure!(
                !cap.pressure()? && !hosting_budget::reference_runtime_memory_pressure()?,
                "resource pressure requires stopping isolated Fabric"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        ensure!(
            status.success(),
            "isolated Fabric failed: {status}; {}\n{}",
            log_tail(&self.workspace.root.join("worker-errors")),
            log_tail(&self.workspace.root.join("fabric.log"))
        );
        ensure!(
            !self.guard.memory_cap().context("Fabric cgroup missing")?.pressure()?,
            "isolated Fabric recorded memory pressure"
        );
        self.guard.verify_exit_quiescent()?;
        self.workspace.verify(&self.workspace_pin)?;
        ensure!(!session_locked(&self.workspace.world())?, "isolated Fabric session remains open");
        self.stage.reverify()?;
        check_consent()?;
        let bytes = read_private_evidence(&self.workspace.root.join("fabric-lifecycle.json"), 16384)?;
        let lifecycle: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            lifecycle["classification"] == "isolated_fabric_lifecycle_not_live_authority"
                && lifecycle["test_marker"] == false
                && lifecycle["compatibility"] == self.compatibility.to_string()
                && lifecycle["minecraft"] == "26.1.2"
                && lifecycle["fabric_loader"] == "0.19.3"
                && lifecycle["java_major"] == 25
                && [
                    "private_network_namespace",
                    "authenticated_world_info",
                    "missing_permit_readiness_refused",
                    "active_session_lock_verified",
                    "save_barrier_acknowledged",
                    "shutdown_barrier_acknowledged",
                    "clean_java_exit",
                    "offline_session_lock_verified"
                ]
                .iter()
                .all(|key| lifecycle[*key] == true)
                && ["canonical_authority", "player_admission", "minecraft_gameplay_verified"]
                    .iter()
                    .all(|key| lifecycle[*key] == false),
            "incomplete isolated Fabric lifecycle evidence"
        );
        drop(self.guard);
        absent(&self.local_record)?;
        absent(&self.shared_record)?;
        Ok(CompletedFabric {
            lifecycle,
            process_cpu: self.cpu,
            process_memory: self.memory,
            evidence_digest: Hash32::from_domain_bytes(
                b"swarmcraft/exclusive-turn/isolated-fabric-lifecycle/v1\0",
                &bytes,
            ),
        })
    }
}

fn read_private_evidence(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == uid() && meta.nlink() == 1 && meta.mode() & 0o077 == 0 && meta.len() <= limit,
        "invalid private Fabric evidence"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "Fabric evidence exceeds bound");
    Ok(bytes)
}

fn env_path(name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var_os(name).with_context(|| format!("missing isolated worker input {name}"))?))
}

fn output_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().create_new(true).write(true).mode(0o600).open(path)?)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
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

pub(crate) fn log_tail(path: &Path) -> String {
    log_text(path)
        .map(|s| s.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

pub(super) fn session_locked(world: &Path) -> Result<bool> {
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

pub(crate) fn run_worker(test_marker: bool) -> Result<()> {
    ensure!(!test_marker || cfg!(test), "test mutation unavailable in production");
    check_namespace()?;
    let root = env_path("SC8_LAB_ROOT")?;
    let stage = env_path("SC8_LAB_STAGE")?;
    let pin: Hash32 = std::env::var("SC8_LAB_COMPATIBILITY")?.parse()?;
    let runtime = if test_marker { root.join("runtime") } else { root.join("sandbox/runtime") };
    let heap: u64 = std::env::var("SC8_LAB_HEAP")?.parse()?;
    let cpu: u64 = std::env::var("SC8_LAB_CPU")?.parse()?;
    ensure!(
        (crate::hosting_budget::MIN_HEAP_MIB..=crate::hosting_budget::MAX_HEAP_MIB).contains(&heap),
        "invalid isolated heap"
    );
    crate::host_memory_cap::validate_cpu_request(cpu)?;
    let mut gate = [0u8; 32];
    if !test_marker {
        std::io::stdin().read_exact(&mut gate)?;
        let expected = std::env::var("SC8_LAB_GATE")?;
        ensure!(hex::encode(gate) == expected, "isolated start gate differs");
    }
    let permit = runtime
        .parent()
        .and_then(Path::parent)
        .context("invalid runtime layout")?
        .join("control")
        .join(runtime.file_name().context("missing runtime name")?)
        .join("authority.permit");
    ensure!(
        matches!(fs::symlink_metadata(&permit), Err(e) if e.kind() == std::io::ErrorKind::NotFound),
        "unexpected authority permit"
    );
    let world = runtime.join("world");
    ensure!(world.is_dir() && !session_locked(&world)?, "initial restored fixture must be stopped");
    verify_level_version(&read_level(&world)?, 4790)?;
    let classpath: Vec<String> = serde_json::from_str(&std::env::var("SC8_LAB_CLASSPATH")?)?;
    ensure!(
        !classpath.is_empty()
            && classpath.len() <= 266
            && classpath.iter().all(|entry| !entry.starts_with('/')
                && !entry.contains(':')
                && Path::new(entry).components().all(|c| matches!(c, std::path::Component::Normal(_)))),
        "invalid reviewed runtime classpath"
    );
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
            .arg("-Xms128m").arg(format!("-Xmx{heap}m")).arg(format!("-XX:ActiveProcessorCount={}", cpu.div_ceil(1000))).args(["-XX:+UseSerialGC", "-XX:-UsePerfData"])
            .arg(format!("-Djava.io.tmpdir={}", runtime.display()))
            .arg(format!("-Duser.home={}", runtime.display()))
            .arg(format!("-Dfabric.gameJarPath={}", stage.join("versions/26.1.2/server-26.1.2.jar").display()))
            .arg(format!("-Dfabric.addMods={}", stage.join("mods/swarmcraft-fabric.jar").display()))
            .arg("-cp").arg(classpath).args(["net.fabricmc.loader.impl.launch.knot.KnotServer", "nogui"])
            .stdin(Stdio::piped()).stdout(Stdio::from(stdout)).stderr(Stdio::from(stderr));
        // SAFETY: async-signal-safe setrlimit calls only before exec.
        unsafe { command.pre_exec(move || {
            for (resource, value) in [(libc::RLIMIT_FSIZE, if test_marker { LOG_LIMIT } else { 256 * 1024 * 1024 }), (libc::RLIMIT_CORE, 0)] {
                let limit = libc::rlimit { rlim_cur: value, rlim_max: value };
                if libc::setrlimit(resource, &limit) != 0 { return Err(std::io::Error::last_os_error()); }
            }
            Ok(())
        }); }
        let started = Instant::now();
        let mut java = ProbeChild(command.spawn()?);
        if !test_marker {
            // Atomically publish only after actual Java spawn, so the
            // parent can serialize the full spawn with consent withdrawal.
            let mut marker=tempfile::NamedTempFile::new_in(&root)?;
            marker.write_all(&gate)?;
            marker.as_file().sync_all()?;
            marker.persist_noclobber(root.join("fabric-child-started")).map_err(|e|e.error)?;
            File::open(&root)?.sync_all()?;
        }
        let mut session = listener.accept(Duration::from_secs(100)).await.with_context(|| format!("Fabric handshake failed:\n{}", log_tail(&log)))?;
        let info = session.world_info();
        ensure!(info.minecraft_version == "26.1.2" && info.fabric_loader_version == "0.19.3" && info.java_major == 25
            && Path::new(&info.world_directory) == world && info.compatibility_fingerprint == pin, "authenticated Fabric world info differs");
        ensure!(session_locked(&world)?, "running Minecraft did not hold its Java session lock");
        let readiness = session.runtime_challenge(1, random_nonce(), Duration::from_millis(1500)).await;
        ensure!(matches!(readiness, Err(IpcTransportError::RemoteError {request_id:1, ref code}) if code == "RUNTIME_UNAVAILABLE"), "missing-permit readiness did not refuse: {readiness:?}");
        // Actual deterministic world data mutation, not a synthetic byte edit.
        let mut input = java.0.stdin.take().context("Minecraft console missing")?;
        if test_marker {
        input.write_all(b"scoreboard objectives add sc8marker dummy\nscoreboard players set sc8_checkpoint sc8marker 2601\n")?;
        input.flush()?;
        let deadline = Instant::now()+Duration::from_secs(7);
        loop {
            let current = log_text(&log)?;
            if current.lines().any(|line| line.contains("sc8_checkpoint") && line.contains("2601")) { break; }
            ensure!(Instant::now()<deadline && java.0.try_wait()?.is_none(), "real console mutation did not complete:\n{}", log_tail(&log));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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
        verify_level_version(&read_level(&world)?,4790)?;
        ensure!(log_text(&log)?.contains("SwarmCraft authority permit guard enabled"),"actual canonical guard was not enabled");
        ensure!(matches!(fs::symlink_metadata(&permit), Err(e) if e.kind() == std::io::ErrorKind::NotFound),"unexpected authority permit");
        write_new(&root.join("fabric-lifecycle.json"), &serde_json::to_vec(&json!({
            "classification":"isolated_fabric_lifecycle_not_live_authority", "test_marker":test_marker, "minecraft":"26.1.2", "fabric_loader":"0.19.3",
            "java_major":25, "compatibility":pin.to_string(), "private_network_namespace":true,
            "authenticated_world_info":true,"missing_permit_readiness_refused":true,"active_session_lock_verified":true,
            "save_barrier_acknowledged":true,"shutdown_barrier_acknowledged":true,"clean_java_exit":true,
            "offline_session_lock_verified":true,"elapsed_ms":started.elapsed().as_millis(),
            "canonical_authority":false,"player_admission":false,"minecraft_gameplay_verified":false,
        }))?)?;
        Ok::<_,anyhow::Error>(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only controller with a disposable account reservation. It runs
    /// the exact production namespace worker, but must not touch the user's
    /// real account record or pretend to have full game/client headroom.
    #[test]
    fn interrupted_fabric_controller() {
        if std::env::var_os("SC8_TEST_INTERRUPTED_CONTROLLER").is_none() {
            return;
        }
        let root = env_path("SC8_LAB_ROOT").unwrap().canonicalize().unwrap();
        let package = env_path("SC8_LAB_PACKAGE").unwrap();
        let cli = env_path("SC8_LAB_CLI").unwrap().canonicalize().unwrap();
        let compatibility: Hash32 = std::env::var("SC8_LAB_COMPATIBILITY").unwrap().parse().unwrap();
        let paths = DataPaths::from_root(root.join("crash-profile"));
        let profile = hosting_budget::hosting_profile_guard(&paths).unwrap();
        let memory = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(paths.root.join("control/hosting-active-memory.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&memory).unwrap();
        let machine = MachineHostingReservation::acquire_at(&root.join("crash-account")).unwrap();
        let stage = StagedRuntimePackage::stage(&package, &root, compatibility).unwrap();
        let workspace = FabricWorkspace::create(&root).unwrap();
        fs::rename(root.join("runtime/world"), workspace.world()).unwrap();
        let pin = workspace.pin().unwrap();
        let cap = HostMemoryCap::prepare(1024, Some(2000), 1).unwrap().unwrap();
        write_new(&root.join("crash-cap.postcard"), &postcard::to_allocvec(cap.identity()).unwrap()).unwrap();
        let mut admission = ProbeAdmission {
            _profile: profile,
            _memory: memory,
            machine,
            cap: Some(cap),
            stage,
            recovery_pin_prepared: false,
            heap_mib: 1024,
            cpu_millicores: 2000,
            consent_revision: 1,
        };
        admission.verify_fabric_launch_package().unwrap();
        let recovery = admission.prepare_recovery_pin().unwrap();
        write_new(&root.join("crash-pin.postcard"), &postcard::to_allocvec(&recovery).unwrap()).unwrap();
        write_new(&root.join("crash-workspace"), pin.name().as_bytes()).unwrap();
        let _running = admission.start_fabric_checked(&root, &workspace, &pin, compatibility, &cli).unwrap();
        write_new(&root.join("crash-ready"), b"actual Java spawn acknowledged").unwrap();
        // Parent owns this controller child handle. Do not execute normal
        // Drop or publish a checkpoint before the actual controller death.
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    #[ignore = "requires disposable reviewed Fabric package, real CLI worker and delegated scope"]
    fn actual_interrupted_fabric_controller_requires_exact_kernel_recovery() {
        use crate::host_memory_cap::MemoryCapIdentity;
        use std::os::unix::process::ExitStatusExt;
        let root = env_path("SC8_LAB_ROOT").unwrap().canonicalize().unwrap();
        let account = root.join("crash-account");
        let local = root.join("fabric-process.json");
        let shared = account.join("runtime-process.json");
        let mut controller = ProbeChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "exclusive_turn_process_probe::fabric::tests::interrupted_fabric_controller",
                    "--nocapture",
                ])
                .env("SC8_TEST_INTERRUPTED_CONTROLLER", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        while !root.join("crash-ready").is_file() {
            assert!(controller.0.try_wait().unwrap().is_none(), "fixture controller exited before Java spawn");
            assert!(Instant::now() < deadline, "fixture controller spawn timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        let pin: ProbeRecoveryPin = postcard::from_bytes(&fs::read(root.join("crash-pin.postcard")).unwrap()).unwrap();
        let cap: MemoryCapIdentity = postcard::from_bytes(&fs::read(root.join("crash-cap.postcard")).unwrap()).unwrap();
        pin.verify_budget(1024, 2000, 1).unwrap();
        cap.verify_probe_budget(1024, 2000, 1).unwrap();
        assert!(cap.populated().unwrap(), "actual runtime did not start");
        let before_local = fs::read(&local).unwrap();
        let before_shared = fs::read(&shared).unwrap();
        assert_eq!(before_local, before_shared);
        assert!(
            MachineHostingReservation::lock_test_probe_recovery(&account).is_err(),
            "active controller lock escaped"
        );
        controller.0.kill().unwrap();
        assert_eq!(controller.0.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert!(cap.populated().unwrap(), "real namespace exited before crash recovery could be tested");
        assert!(
            MachineHostingReservation::acquire_at(&account).is_err(),
            "normal admission accepted surviving runtime"
        );
        let lock = MachineHostingReservation::lock_test_probe_recovery(&account).unwrap();
        let error = pin.recover(&local, &shared).unwrap_err();
        let detail = format!("{error:#}");
        assert!(
            detail.contains("process is still alive") || detail.contains("cgroup still contains processes"),
            "unexpected refusal: {detail}"
        );
        assert_eq!(fs::read(&local).unwrap(), before_local);
        assert_eq!(fs::read(&shared).unwrap(), before_shared);
        let deadline = Instant::now() + Duration::from_secs(180);
        while cap.populated().unwrap() {
            assert!(Instant::now() < deadline, "actual interrupted namespace did not exit");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(pin.recover(&local, &shared).unwrap(), "exact_empty_cgroup_retired");
        assert!(!local.exists() && !shared.exists());
        assert!(cap.populated().is_err(), "exact leaf was not retired");
        assert!(pin.recover(&local, &shared).is_err(), "missing same-boot leaf was accepted");
        drop(lock);
        assert!(MachineHostingReservation::acquire_at(&account).is_ok());
        let name = fs::read_to_string(root.join("crash-workspace")).unwrap();
        let workspace = root.join(&name);
        assert!(workspace.join("sandbox/runtime/world").is_dir() && !workspace.join("completed-bundle").exists());
        let result = json!({"classification":"actual_interrupted_fabric_fixture_kernel_recovery",
            "real_java_spawn_observed":true,"controller_sigkill":true,"surviving_runtime_refused":true,
            "exact_empty_cgroup_retired":true,"both_records_retired":true,"missing_leaf_refused":true,
            "workspace_retained":true,"checkpoint_published":false,"canonical_authority":false,
            "player_admission":false,"full_public_admission_tested":false});
        write_new(&root.join("fabric-interruption.json"), &serde_json::to_vec(&result).unwrap()).unwrap();
        println!("{result}");
    }
    #[test]
    #[ignore = "requires disposable reviewed Fabric package, real CLI worker and delegated scope"]
    fn actual_gated_fabric_worker_has_no_player_route_and_retires_exact_process_boundary() {
        // Deliberately test-only, as with the existing delegated Java probe:
        // the daily workstation may lack full client+OS hosting reserves.
        // No production admission or start path bypasses check_budget.
        let root = env_path("SC8_LAB_ROOT").unwrap().canonicalize().unwrap();
        let package = env_path("SC8_LAB_PACKAGE").unwrap();
        let cli = env_path("SC8_LAB_CLI").unwrap().canonicalize().unwrap();
        let compatibility: Hash32 = std::env::var("SC8_LAB_COMPATIBILITY").unwrap().parse().unwrap();
        let paths = DataPaths::from_root(root.join("process-profile"));
        let (profile, memory, machine) = hosting_budget::reserve_reference_capacity(&paths).unwrap();
        let stage = StagedRuntimePackage::stage(&package, &root, compatibility).unwrap();
        let workspace = FabricWorkspace::create(&root).unwrap();
        let original = root.join("runtime/world");
        fs::rename(&original, workspace.world()).unwrap();
        let pin = workspace.pin().unwrap();
        let cap = HostMemoryCap::prepare(1024, Some(2000), 1).unwrap().unwrap();
        let mut admission = ProbeAdmission {
            _profile: profile,
            _memory: memory,
            machine,
            cap: Some(cap),
            stage,
            recovery_pin_prepared: false,
            heap_mib: 1024,
            cpu_millicores: 2000,
            consent_revision: 1,
        };
        admission.verify_fabric_launch_package().unwrap();
        assert!(
            admission.start_fabric_checked(&root, &workspace, &pin, compatibility, &cli).is_err(),
            "unprepared process gate accepted"
        );
        let recovery = admission.prepare_recovery_pin().unwrap();
        let running = admission.start_fabric_checked(&root, &workspace, &pin, compatibility, &cli).unwrap();
        let completed = running.complete(|| Ok(())).unwrap();
        admission.cleanup_completed_leaf(&recovery).unwrap();
        admission.cleanup_completed_stage().unwrap();
        workspace.verify(&pin).unwrap();
        // The test owns these paths and has reaped all descendants. Return
        // the stopped save to the external fixture for independent capture.
        fs::rename(workspace.world(), &original).unwrap();
        write_new(&root.join("fabric-lifecycle.json"), &serde_json::to_vec(&completed.lifecycle).unwrap()).unwrap();
        write_new(&root.join("fabric-supervision.json"),&serde_json::to_vec(&json!({"classification":"disposable_gated_fabric_process_boundary","process_cpu":completed.process_cpu,"process_memory":completed.process_memory,"exact_staged_package_reverified":true,"process_and_descendants_quiescent":true,"ownership_records_retired":true,"evidence_digest":completed.evidence_digest.to_string(),"canonical_authority":false,"runtime_activation":false,"player_admission":false})).unwrap()).unwrap();
        // The external fixture removes its entire disposable root after
        // independently checking the returned save. Never delete another
        // world or account record here.
        println!("actual gated worker: {}", serde_json::to_string(&completed).unwrap());
    }
}
