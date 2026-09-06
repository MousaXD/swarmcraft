#![cfg(target_os = "linux")]

use std::{
    fs,
    process::{Command, Stdio},
    str::FromStr,
    thread,
    time::Duration,
};
use swarm_cli::world_import::{import_world, ImportWorldRequest};
use swarm_core::DataPaths;
use swarm_protocol::{WorldId, WorldVisibilityV1};
use swarm_storage::Storage;

const JAVA_LOCK_OWNER: &str = r#"
import java.nio.channels.FileChannel;
import java.nio.channels.FileLock;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

public class MinecraftSessionLockOwner {
    public static void main(String[] args) throws Exception {
        Path lockPath = Path.of(args[0]);
        Path worldFile = Path.of(args[1]);
        Path ready = Path.of(args[2]);
        Path release = Path.of(args[3]);
        try (FileChannel channel = FileChannel.open(lockPath, StandardOpenOption.READ, StandardOpenOption.WRITE);
             FileLock ignored = channel.lock()) {
            int generation = 0;
            Files.writeString(ready, "locked\n", StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING);
            while (!Files.exists(release)) {
                Files.writeString(
                    worldFile,
                    "live-generation-" + generation++ + "\n",
                    StandardOpenOption.CREATE,
                    StandardOpenOption.TRUNCATE_EXISTING
                );
                Thread.sleep(5);
            }
        }
    }
}
"#;

fn request(source: std::path::PathBuf) -> ImportWorldRequest {
    ImportWorldRequest {
        source,
        name: "Real Lock Import".into(),
        minecraft_version: "26.1.2".into(),
        fabric_loader_version: "0.19.3".into(),
        visibility: WorldVisibilityV1::Private,
        server_mod_jars: Vec::new(),
        confirm_no_server_mods: true,
    }
}

#[test]
#[ignore = "requires a real Java process; Agent 6 exact-head acceptance runs this under Java 25"]
fn java_nio_minecraft_lock_rejects_live_import_then_clean_stop_imports() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source-world");
    fs::create_dir_all(source.join("region")).unwrap();
    fs::write(source.join("level.dat"), b"real-lock-level\n").unwrap();
    fs::write(source.join("session.lock"), b"minecraft-compatible-lock\n").unwrap();
    let region = source.join("region/r.0.0.mca");
    fs::write(&region, b"pre-live\n").unwrap();

    let helper = temp.path().join("MinecraftSessionLockOwner.java");
    let ready = temp.path().join("java-lock-ready");
    let release = temp.path().join("java-lock-release");
    fs::write(&helper, JAVA_LOCK_OWNER).unwrap();

    let mut child = Command::new("java")
        .arg(&helper)
        .arg(source.join("session.lock"))
        .arg(&region)
        .arg(&ready)
        .arg(&release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Java 25 must be installed for Agent 6 real import acceptance");

    for _ in 0..1000 {
        if ready.exists() {
            break;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("Java Minecraft-compatible lock owner exited before lock acquisition: {status}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists(), "Java lock owner never acquired Minecraft session.lock");

    thread::sleep(Duration::from_millis(30));
    let live_bytes = fs::read(&region).unwrap();
    assert!(String::from_utf8_lossy(&live_bytes).starts_with("live-generation-"));

    let paths = DataPaths::from_root(temp.path().join("swarmcraft-data"));
    let error = import_world(&paths, &request(source.clone())).expect_err("live Minecraft-compatible lock must reject import");
    let message = error.to_string();
    assert!(message.contains("currently open") || message.contains("session lock"), "unexpected rejection: {message}");

    let storage = Storage::open(paths.root.clone()).unwrap();
    assert!(storage.list_worlds().unwrap().is_empty(), "rejected live import published a canonical world");
    let staging = paths.root.join(".import-staging");
    assert!(
        !staging.exists() || fs::read_dir(&staging).unwrap().next().is_none(),
        "rejected live import left a partial staging world"
    );

    fs::write(&release, b"release\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Java lock owner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stopped_bytes = fs::read(&region).unwrap();
    let result = import_world(&paths, &request(source)).expect("stopped source must import successfully");
    let world = WorldId::from_str(&result.world_id).unwrap();
    let latest = storage.latest_snapshot(world).unwrap().expect("import must publish snapshot");
    storage.verify_snapshot(&latest).unwrap();
    let restored = temp.path().join("restored");
    storage.restore_snapshot(&latest, &restored).unwrap();
    assert_eq!(fs::read(restored.join("region/r.0.0.mca")).unwrap(), stopped_bytes);
}
