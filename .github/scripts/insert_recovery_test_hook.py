from pathlib import Path

path = Path("crates/swarm-cli/tests/recovery_successor_dies.rs")
text = path.read_text()
text = text.replace(
    '    process::{Child, Command, Stdio},\n',
    '    process::{Child, Command, Stdio},\n',
    1,
)
old_struct = '''struct ManagedChild(Child);

impl ManagedChild {
    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
'''
new_struct = '''struct ManagedChild {
    child: Child,
    log_path: std::path::PathBuf,
}

impl ManagedChild {
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn status(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().unwrap()
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_else(|error| format!("<failed to read daemon log: {error}>"))
    }
}
'''
if old_struct in text:
    text = text.replace(old_struct, new_struct, 1)
elif new_struct not in text:
    raise SystemExit("ManagedChild block not found")
old_spawn = '''fn spawn_daemon(peer: &PeerFixture, bootstraps: &[String], pause_after_certificate: bool) -> ManagedChild {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/127.0.0.1/udp/{}/quic-v1", peer.port))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if !bootstraps.is_empty() {
        command.env("SWARMCRAFT_BOOTSTRAP", bootstraps.join(","));
    }
    if pause_after_certificate {
        command.env("SWARMCRAFT_TEST_PAUSE_AFTER_RECOVERY_CERTIFICATE_MS", RECOVERY_PAUSE_MS.to_string());
    }
    ManagedChild(command.spawn().unwrap())
}
'''
new_spawn = '''fn spawn_daemon(peer: &PeerFixture, bootstraps: &[String], pause_after_certificate: bool) -> ManagedChild {
    let log_path = peer.paths.root.join("recovery-acceptance-daemon.log");
    let log = fs::File::create(&log_path).unwrap();
    let log_err = log.try_clone().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/127.0.0.1/udp/{}/quic-v1", peer.port))
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    if !bootstraps.is_empty() {
        command.env("SWARMCRAFT_BOOTSTRAP", bootstraps.join(","));
    }
    if pause_after_certificate {
        command.env("SWARMCRAFT_TEST_PAUSE_AFTER_RECOVERY_CERTIFICATE_MS", RECOVERY_PAUSE_MS.to_string());
    }
    ManagedChild { child: command.spawn().unwrap(), log_path }
}
'''
if old_spawn in text:
    text = text.replace(old_spawn, new_spawn, 1)
elif new_spawn not in text:
    raise SystemExit("spawn_daemon block not found")
insert_after = '''fn wait_until(label: &str, timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(WAIT_STEP);
    }
    panic!("timed out waiting for {label}");
}
'''
extra = '''
fn wait_until_with_daemon(
    label: &str,
    timeout: Duration,
    daemon: &mut ManagedChild,
    mut predicate: impl FnMut() -> bool,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        if let Some(status) = daemon.status() {
            panic!("daemon exited while waiting for {label}: {status}\\n{}", daemon.log());
        }
        thread::sleep(WAIT_STEP);
    }
    panic!("timed out waiting for {label}\\n{}", daemon.log());
}
'''
if extra not in text:
    if insert_after not in text:
        raise SystemExit("wait_until block not found")
    text = text.replace(insert_after, insert_after + extra, 1)
old_wait = '''    wait_until("stale first successor adopting newer certified recovery", Duration::from_secs(40), || {
        first_successor.storage.load_epoch_record(world).is_ok_and(|record| {
            record.epoch_number == 2 && record.fencing_token == 2 && record.authority_peer_id == second_successor_id
        })
    });
'''
new_wait = '''    wait_until_with_daemon(
        "stale first successor adopting newer certified recovery",
        Duration::from_secs(40),
        &mut restarted_first,
        || {
            first_successor.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.authority_peer_id == second_successor_id
            })
        },
    );
'''
if old_wait in text:
    text = text.replace(old_wait, new_wait, 1)
elif new_wait not in text:
    raise SystemExit("stale successor wait block not found")
old_wait_a = '''    wait_until("original stale authority adopting accepted recovery", Duration::from_secs(40), || {
        a.storage.load_epoch_record(world).is_ok_and(|record| {
            record.epoch_number == 2 && record.fencing_token == 2 && record.authority_peer_id == second_successor_id
        })
    });
'''
new_wait_a = '''    wait_until_with_daemon(
        "original stale authority adopting accepted recovery",
        Duration::from_secs(40),
        &mut restarted_a,
        || {
            a.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.authority_peer_id == second_successor_id
            })
        },
    );
'''
if old_wait_a in text:
    text = text.replace(old_wait_a, new_wait_a, 1)
elif new_wait_a not in text:
    raise SystemExit("stale authority wait block not found")
path.write_text(text)
