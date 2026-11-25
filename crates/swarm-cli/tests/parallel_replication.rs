//! Real local daemon processes and connection-bound authenticated replication.
//! The fixture commits a genuine signed joint membership before starting any
//! processes; no Minecraft hosting permission or external service is needed.
#![cfg(unix)]

use std::{
    fs,
    net::UdpSocket,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};
use swarm_consensus::{membership_vote_for, validate_membership_certificate_shape};
use swarm_core::{DataPaths, PeerIdentity};
use swarm_network::load_or_create_transport_key;
use swarm_protocol::{
    BlobDescriptor, BlobEncoding, DiscoveryMembershipProofV1, EpochRecordV1, MembershipCertificateV1,
    MembershipProposalV1, MembershipRecordV1, SnapshotManifestV1, WorldId, WorldMemberV1, PROTOCOL_VERSION,
};
use swarm_storage::replica_policy::{ReplicaPolicy, UtcWindow};
use swarm_storage::WorldStoragePolicy;
use swarm_storage::{SnapshotContext, Storage};

struct Device {
    temp: tempfile::TempDir,
    paths: DataPaths,
    storage: Storage,
    identity: PeerIdentity,
    port: u16,
    transport: String,
}

impl Device {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().join("data"));
        let storage = Storage::open(&paths.root).unwrap();
        let identity = PeerIdentity::load_or_create(&paths).unwrap();
        let transport = load_or_create_transport_key(&paths.transport_key()).unwrap().public().to_peer_id().to_string();
        let port = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        Self { temp, paths, storage, identity, port, transport }
    }

    fn address(&self) -> String {
        format!("/ip4/127.0.0.1/udp/{}/quic-v1/p2p/{}", self.port, self.transport)
    }

    fn member(&self, eligible: bool) -> WorldMemberV1 {
        WorldMemberV1 {
            peer_id: self.identity.peer_id(),
            public_key: self.identity.public_key(),
            authority_eligible: eligible,
            banned: false,
        }
    }

    fn start(&self, peers: &[String], log_name: &str) -> Daemon {
        let path = self.temp.path().join(log_name);
        let log = fs::File::create(&path).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
        command
            .arg("--data-dir")
            .arg(&self.paths.root)
            .args(["daemon", "--listen", &format!("/ip4/127.0.0.1/udp/{}/quic-v1", self.port)])
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .stdout(log.try_clone().unwrap())
            .stderr(log);
        for peer in peers {
            command.arg("--peer").arg(peer);
        }
        Daemon { child: command.spawn().unwrap(), log: path }
    }
}

struct Daemon {
    child: Child,
    log: PathBuf,
}

impl Daemon {
    fn text(&self) -> String {
        // Tracing may include ANSI escapes even when stdout is redirected.
        let text = fs::read_to_string(&self.log).unwrap_or_default();
        let mut clean = String::new();
        let mut escape = false;
        for ch in text.chars() {
            if escape {
                if ch.is_ascii_alphabetic() {
                    escape = false;
                }
            } else if ch == '\x1b' {
                escape = true;
            } else {
                clean.push(ch);
            }
        }
        clean
    }

    fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

fn wait(label: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(25));
    }
}

fn copy_payload(source: &Storage, destination: &Storage, manifest: &SnapshotManifestV1) {
    for descriptor in destination.missing_blobs(manifest) {
        let mut offset = destination.partial_blob_offset(manifest.world_id, &descriptor).unwrap();
        loop {
            let (bytes, finished) =
                source.read_encoded_blob_chunk(manifest.world_id, &descriptor, offset, 256 * 1024).unwrap();
            offset = destination.receive_blob_chunk(manifest.world_id, &descriptor, offset, &bytes, finished).unwrap();
            if finished {
                break;
            }
        }
    }
}

struct Fixture {
    authority: Device,
    seeder: Device,
    receiver: Device,
    world: WorldId,
    epoch: EpochRecordV1,
    membership: MembershipRecordV1,
    first: SnapshotManifestV1,
}

impl Fixture {
    fn new() -> Self {
        let authority = Device::new();
        let seeder = Device::new();
        let receiver = Device::new();
        assert_ne!(authority.port, seeder.port);
        assert_ne!(authority.port, receiver.port);
        assert_ne!(seeder.port, receiver.port);
        let result = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
            .arg("--data-dir")
            .arg(&authority.paths.root)
            .args([
                "world",
                "create",
                "--name",
                "Parallel replication fixture",
                "--minecraft",
                "26.1.2",
                "--fabric-loader",
                "0.19.3",
            ])
            .env("RUST_LOG", "warn")
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let world: WorldId = String::from_utf8(result.stdout)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("World ID: "))
            .unwrap()
            .parse()
            .unwrap();
        let epoch = authority.storage.load_epoch_record(world).unwrap();
        let first = authority.storage.latest_snapshot(world).unwrap().unwrap();
        let metadata = authority.storage.load_world(world).unwrap();
        let config = authority.storage.load_world_config(world).unwrap();
        let initial = authority.storage.load_membership_record(world).unwrap();
        let mut membership = initial.clone();
        membership.sequence += 1;
        membership.previous_membership_hash = Some(initial.record_hash().unwrap());
        membership.members = vec![authority.member(true), seeder.member(false), receiver.member(false)];
        membership.members.sort_by_key(|member| member.peer_id);
        authority.identity.sign_membership(&mut membership).unwrap();
        let proposal = MembershipProposalV1 { previous: initial.clone(), proposed: membership.clone() };
        let votes = [&authority, &seeder, &receiver]
            .into_iter()
            .map(|device| {
                let mut vote =
                    membership_vote_for(&proposal, device.identity.peer_id(), device.identity.public_key()).unwrap();
                vote.signature = device.identity.sign(&vote.signing_bytes().unwrap());
                vote
            })
            .collect();
        let certificate = MembershipCertificateV1 { proposal, votes };
        validate_membership_certificate_shape(&certificate).unwrap();
        let proof = DiscoveryMembershipProofV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            genesis: metadata.genesis.clone(),
            initial_membership: initial.clone(),
            current_membership: membership.clone(),
            membership_certificates: vec![certificate.clone()],
            pending_membership: None,
        };
        swarm_cli::live_authority::verify_genesis_anchored_membership(&proof).unwrap();
        let mut descriptor = authority.storage.load_world_descriptor(world).unwrap();
        descriptor.members = membership.members.clone();
        authority.storage.save_membership_certificate(&certificate).unwrap();
        authority.storage.save_membership_record(&membership).unwrap();
        authority.storage.save_world_descriptor(&descriptor).unwrap();
        for peer in [&seeder, &receiver] {
            peer.storage.create_world(&metadata).unwrap();
            peer.storage.save_epoch_record(&epoch).unwrap();
            peer.storage.save_membership_record(&initial).unwrap();
            peer.storage.save_membership_certificate(&certificate).unwrap();
            peer.storage.save_membership_record(&membership).unwrap();
            peer.storage.save_world_descriptor(&descriptor).unwrap();
            peer.storage.save_world_config(&config).unwrap();
            copy_payload(&authority.storage, &peer.storage, &first);
            peer.storage.finalize_replica(&first).unwrap();
        }
        seeder.storage.set_background_seeding(world, true).unwrap();
        Self { authority, seeder, receiver, world, epoch, membership, first }
    }

    fn prepare(
        &self,
        previous: &SnapshotManifestV1,
        file_count: usize,
        bytes_per_file: usize,
    ) -> (tempfile::TempDir, swarm_storage::SnapshotPublication) {
        let source = tempfile::tempdir().unwrap();
        for index in 0..file_count {
            let mut state = (previous.snapshot_number * 10_000 + index as u64 * 37 + 1) ^ 0xAC45_8D23;
            let bytes: Vec<u8> = (0..bytes_per_file)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state >> 23) as u8
                })
                .collect();
            fs::write(source.path().join(format!("region-{index}.dat")), bytes).unwrap();
        }
        fs::write(source.path().join("empty.dat"), []).unwrap();
        let mut publication = self
            .authority
            .storage
            .snapshot_directory(
                source.path(),
                SnapshotContext {
                    world: self.world,
                    snapshot_number: previous.snapshot_number + 1,
                    epoch: self.epoch.epoch_number,
                    sequence: previous.sequence + 1,
                    previous_snapshot_hash: Some(previous.manifest_hash().unwrap()),
                    authority_peer_id: self.authority.identity.peer_id(),
                    authority_public_key: self.authority.identity.public_key(),
                },
            )
            .unwrap();
        self.authority.identity.sign_snapshot(&mut publication).unwrap();
        copy_payload(&self.authority.storage, &self.seeder.storage, publication.manifest());
        self.seeder.storage.finalize_replica(publication.manifest()).unwrap();
        (source, publication)
    }

    fn restore(&self, manifest: &SnapshotManifestV1, source: &Path, label: &str) {
        self.receiver.storage.verify_snapshot(manifest).unwrap();
        let restored = self.receiver.temp.path().join(label);
        self.receiver.storage.restore_snapshot(manifest, &restored).unwrap();
        for entry in &manifest.entries {
            assert_eq!(fs::read(source.join(&entry.path)).unwrap(), fs::read(restored.join(&entry.path)).unwrap());
        }
        assert_eq!(self.receiver.storage.load_epoch_record(self.world).unwrap(), self.epoch);
        assert_eq!(self.receiver.storage.load_membership_record(self.world).unwrap(), self.membership);
        assert!(swarm_cli::hosting_budget::load_consent(&self.receiver.paths, self.world).unwrap().is_none());
    }

    fn wait_head(&self, daemon: &mut Daemon, manifest: &SnapshotManifestV1, label: &str) -> Duration {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(90);
        loop {
            assert!(daemon.alive(), "receiver daemon exited: {}", daemon.text());
            if self.receiver.storage.latest_snapshot(self.world).ok().flatten().as_ref() == Some(manifest) {
                break;
            }
            assert!(Instant::now() < deadline, "{label}: receiver log:\n{}", daemon.text());
            thread::sleep(Duration::from_millis(25));
        }
        start.elapsed()
    }
}

fn complete_line(log: &str, number: u64) -> Option<&str> {
    log.lines().find(|line| {
        line.contains("receiver-driven replica pull completed")
            && line.split_whitespace().any(|field| field == format!("snapshot={number}"))
    })
}

fn metric(line: &str, name: &str) -> usize {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("missing {name} in {line}"))
        .parse()
        .unwrap()
}

fn blob_path(storage: &Storage, world: WorldId, descriptor: &BlobDescriptor) -> PathBuf {
    let suffix = match descriptor.encoding {
        BlobEncoding::Raw => "raw",
        BlobEncoding::Zstd => "zst",
    };
    storage.world_dir(world).join("blobs").join(format!("{}.{suffix}", descriptor.hash))
}

#[test]
fn controlled_replica_policy_pauses_resumes_and_survives_source_churn() {
    let fixture = Fixture::new();
    let world = fixture.world;
    let paused = ReplicaPolicy { follow: false, ..Default::default() };
    fixture.receiver.storage.set_world_replica_policy(world, paused).unwrap();
    let mut authority = fixture.authority.start(&[], "policy-authority.log");
    wait("authority listener", Duration::from_secs(15), || authority.text().contains("daemon listening"));
    let mut receiver = fixture.receiver.start(&[fixture.authority.address()], "policy-receiver.log");
    wait("policy receiver authentication", Duration::from_secs(20), || receiver.text().contains("peer authenticated"));
    let (source, next) = fixture.prepare(&fixture.first, 1, 1024 * 1024);
    fixture.authority.storage.commit_snapshot(&next).unwrap();
    let blob = next.entries.iter().find(|entry| entry.blob.encoded_size > 512 * 1024).unwrap().blob.clone();
    let offset = || fixture.receiver.storage.partial_blob_offset(world, &blob).unwrap();
    let old_head = || {
        assert_eq!(fixture.receiver.storage.latest_snapshot(world).unwrap().as_ref(), Some(&fixture.first));
        fixture.receiver.storage.verify_snapshot(&fixture.first).unwrap();
    };
    wait("explicit receiver pause", Duration::from_secs(30), || authority.text().contains("CACHE_PAUSED"));
    old_head();
    assert_eq!(offset(), 0);
    let minute = swarm_cli::replica_cache::utc_minute().unwrap();
    let closed = ReplicaPolicy {
        follow: true,
        hot: true,
        utc_window: Some(UtcWindow { start_minute: (minute + 60) % 1440, end_minute: (minute + 61) % 1440 }),
        max_kib_per_second: Some(256),
    };
    let prior_denials = authority.text().matches("CACHE_PAUSED").count();
    fixture.receiver.storage.set_world_replica_policy(world, closed).unwrap();
    wait("closed UTC window refusal", Duration::from_secs(30), || {
        authority.text().matches("CACHE_PAUSED").count() > prior_denials
    });
    old_head();
    assert_eq!(offset(), 0);

    let controlled = ReplicaPolicy { utc_window: None, ..closed };
    fixture
        .receiver
        .storage
        .set_world_storage_policy(world, WorldStoragePolicy { max_world_bytes: Some(0), ..Default::default() })
        .unwrap();
    fixture.receiver.storage.set_world_replica_policy(world, controlled).unwrap();
    wait("single-source controlled pull", Duration::from_secs(30), || {
        receiver.text().contains("receiver-driven replica pull negotiated")
    });
    thread::sleep(Duration::from_secs(1));
    old_head();
    assert_eq!(offset(), 0, "quota must prevent scheduling payload reads");
    fixture.receiver.storage.set_world_storage_policy(world, WorldStoragePolicy::default()).unwrap();
    wait("durable partial", Duration::from_secs(30), || {
        let n = offset();
        n > 0 && n < blob.encoded_size
    });
    fixture.receiver.storage.set_world_replica_policy(world, paused).unwrap();
    wait("in-flight policy withdrawal", Duration::from_secs(10), || {
        receiver.text().contains("receiver-driven replica pull paused by local cache policy")
    });
    let partial = offset();
    assert!(partial > 0 && partial < blob.encoded_size);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(offset(), partial);
    old_head();
    receiver.stop();
    authority.stop();
    // One surviving opted-in seeder already has the exact signed checkpoint.
    // Restart the receiving daemon: policy and partial must survive.
    let reopened = Storage::open(&fixture.receiver.paths.root).unwrap();
    assert_eq!(reopened.world_replica_policy(world).unwrap(), paused);
    assert_eq!(reopened.partial_blob_offset(world, &blob).unwrap(), partial);
    reopened.set_world_replica_policy(world, controlled).unwrap();
    let mut seeder = fixture.seeder.start(&[], "policy-seeder.log");
    wait("seeder listener", Duration::from_secs(15), || seeder.text().contains("daemon listening"));
    let start = Instant::now();
    let mut receiver = fixture.receiver.start(&[fixture.seeder.address()], "policy-restarted.log");
    fixture.wait_head(&mut receiver, next.manifest(), "controlled resume from surviving seeder");
    fixture.restore(next.manifest(), source.path(), "policy-restored");
    fixture.seeder.storage.verify_snapshot(next.manifest()).unwrap();
    let remaining = blob.encoded_size - partial;
    let elapsed = start.elapsed();
    // With a quarter-second initial burst, the total cannot arrive sooner
    // than the requested payload budget permits (wall time includes startup).
    assert!(elapsed.as_secs_f64() + 0.3 >= remaining as f64 / (256.0 * 1024.0));
    wait("verified pull completion diagnostic", Duration::from_secs(5), || {
        complete_line(&receiver.text(), next.snapshot_number).is_some()
    });
    let log = receiver.text();
    let line = complete_line(&log, next.snapshot_number).expect("verified pull completion");
    assert_eq!(metric(line, "sources"), 1);
    assert_eq!(metric(line, "max_parallel"), 1);
    assert!(metric(line, "resumed") > 0);
    assert!(receiver.alive() && seeder.alive());
    eprintln!("controlled replica resumed {remaining} bytes in {elapsed:?}; two surviving verified copies; {line}");
}

#[test]
fn parallel_replication_is_bounded_and_falls_back_after_source_corruption_and_disconnect() {
    let fixture = Fixture::new();
    let mut authority = fixture.authority.start(&[], "authority.log");
    let mut seeder = fixture.seeder.start(&[], "seeder.log");
    wait("both source listeners", Duration::from_secs(15), || {
        authority.text().contains("daemon listening") && seeder.text().contains("daemon listening")
    });
    let mut receiver = fixture.receiver.start(&[fixture.authority.address(), fixture.seeder.address()], "receiver.log");
    let authority_peer = fixture.authority.identity.peer_id().to_string();
    let seeder_peer = fixture.seeder.identity.peer_id().to_string();
    wait("two independent connection-bound peers", Duration::from_secs(20), || {
        let text = receiver.text();
        text.lines().any(|line| line.contains("peer authenticated") && line.contains(&authority_peer))
            && text.lines().any(|line| line.contains("peer authenticated") && line.contains(&seeder_peer))
    });

    let (source, next) = fixture.prepare(&fixture.first, 8, 768 * 1024);
    fixture.authority.storage.commit_snapshot(&next).unwrap();
    let start = Instant::now();
    let parallel_download = fixture.wait_head(&mut receiver, next.manifest(), "parallel verified checkpoint");
    fixture.restore(next.manifest(), source.path(), "parallel-restored");
    let parallel_restore = start.elapsed();
    wait("parallel completion diagnostic", Duration::from_secs(5), || {
        complete_line(&receiver.text(), next.snapshot_number).is_some()
    });
    let line = complete_line(&receiver.text(), next.snapshot_number).unwrap().to_owned();
    eprintln!("parallel: {line}; download={parallel_download:?}; including verified restore={parallel_restore:?}");
    assert_eq!(metric(&line, "sources"), 2, "{line}");
    assert_eq!(metric(&line, "max_parallel"), 2, "{line}");
    assert_eq!(metric(&line, "failures"), 0, "{line}");

    // Same file count/size, but a connected member has disabled voluntary
    // background seeding. It is NOT a data source, even if it advertises
    // protocol support; only the authority actually provides bytes.
    fixture.seeder.storage.set_background_seeding(fixture.world, false).unwrap();
    let (fallback_source, fallback) = fixture.prepare(next.manifest(), 8, 768 * 1024);
    fixture.authority.storage.commit_snapshot(&fallback).unwrap();
    let start = Instant::now();
    let fallback_download = fixture.wait_head(&mut receiver, fallback.manifest(), "single-source fallback");
    fixture.restore(fallback.manifest(), fallback_source.path(), "fallback-restored");
    let fallback_restore = start.elapsed();
    wait("fallback completion diagnostic", Duration::from_secs(5), || {
        complete_line(&receiver.text(), fallback.snapshot_number).is_some()
    });
    let line = complete_line(&receiver.text(), fallback.snapshot_number).unwrap().to_owned();
    eprintln!("single-source fallback: {line}; download={fallback_download:?}; including verified restore={fallback_restore:?}");
    assert_eq!(metric(&line, "sources"), 1, "{line}");
    assert!(metric(&line, "failures") > 0, "{line}");
    // Timings are measured and reported, not a flaky universal throughput
    // assertion. Parallel slots and distinct actual sources are strict.
    eprintln!("same-sized controlled LAN fixture: parallel={parallel_restore:?}, fallback={fallback_restore:?}");

    // A new generation of data includes corrupted same-length encoded
    // payloads on the optional seeder. It is still authenticated but its
    // bytes are not trusted. Stop it after a prefix reaches the receiver.
    fixture.seeder.storage.set_background_seeding(fixture.world, true).unwrap();
    let (corrupt_source, corrupt) = fixture.prepare(fallback.manifest(), 4, 3 * 1024 * 1024);
    let good = corrupt.entries.iter().find(|entry| entry.blob.encoded_size > 1024 * 1024).unwrap().blob.clone();
    // An unrelated completed blob must remain available unchanged even
    // after an invalid mixed-source partial is rejected.
    let (mut offset, world) = (0, fixture.world);
    loop {
        let (bytes, finished) =
            fixture.authority.storage.read_encoded_blob_chunk(world, &good, offset, 256 * 1024).unwrap();
        offset = fixture.receiver.storage.receive_blob_chunk(world, &good, offset, &bytes, finished).unwrap();
        if finished {
            break;
        }
    }
    let good_path = blob_path(&fixture.receiver.storage, world, &good);
    use std::os::unix::fs::MetadataExt;
    let good_inode = fs::metadata(&good_path).unwrap().ino();
    for entry in &corrupt.entries {
        if entry.blob.encoded_size == 0 {
            continue;
        }
        let path = blob_path(&fixture.seeder.storage, world, &entry.blob);
        let mut bytes = fs::read(&path).unwrap();
        if !bytes.is_empty() {
            bytes[0] ^= 0x5a;
            fs::write(path, bytes).unwrap();
        }
    }
    fixture.authority.storage.commit_snapshot(&corrupt).unwrap();
    wait("corrupt optional source contributes a prefix", Duration::from_secs(35), || {
        receiver.text().lines().any(|line| {
            line.contains("receiver-driven replica source contributing")
                && line.contains(&seeder_peer)
                && line.split_whitespace().any(|field| field == format!("snapshot={}", corrupt.snapshot_number))
        })
    });
    seeder.stop();
    assert!(fixture.receiver.storage.has_complete_blob(world, &good));
    assert_eq!(fs::metadata(&good_path).unwrap().ino(), good_inode);
    fixture.wait_head(&mut receiver, corrupt.manifest(), "fallback after corrupt source disconnect");
    fixture.restore(corrupt.manifest(), corrupt_source.path(), "corrupt-source-restored");
    wait("corrupt-source completion diagnostic", Duration::from_secs(5), || {
        complete_line(&receiver.text(), corrupt.snapshot_number).is_some()
    });
    let text = receiver.text();
    let line = complete_line(&text, corrupt.snapshot_number).unwrap();
    eprintln!("corrupt source fallback: {line}");
    assert!(metric(line, "resumed") > 0, "{line}");
    assert!(metric(line, "failures") > 0, "{line}");
    assert!(text.contains("resumed replica pull failed verification; clean retry"), "{text}");
    assert!(fixture.receiver.storage.has_complete_blob(world, &good));
    assert_eq!(fs::metadata(&good_path).unwrap().ino(), good_inode);
    assert!(authority.alive());
    assert!(receiver.alive());
    assert_eq!(
        fixture.receiver.storage.list_snapshots(world).unwrap(),
        vec![fixture.first.clone(), next.manifest().clone(), fallback.manifest().clone(), corrupt.manifest().clone(),]
    );
}

/// Run in a dedicated local impairment namespace. Unlike the transport-only
/// soak, this records the exact signed checkpoints on three actual daemon
/// processes and restores the receiver's durable payload before and after
/// hard process loss. The admission proof is the same signed fixture as the
/// production replica-policy tests; no host capability or Java is configured.
#[test]
#[ignore = "explicit local kernel impairment and process-loss stress campaign"]
fn impaired_replica_crash_stress_preserves_signed_head() {
    let fixture = Fixture::new();
    let world = fixture.world;
    for device in [&fixture.authority, &fixture.seeder, &fixture.receiver] {
        assert!(swarm_cli::hosting_budget::load_consent(&device.paths, world).unwrap().is_none());
        assert!(swarm_cli::migration::load_runtime_config(&device.paths, world).is_err());
    }
    let mut authority = fixture.authority.start(&[], "stress-authority.log");
    wait("authority listener", Duration::from_secs(15), || {
        assert!(authority.alive(), "stress authority exited: {}", authority.text());
        authority.text().contains("daemon listening")
    });
    let mut receiver = fixture.receiver.start(&[fixture.authority.address()], "stress-receiver.log");
    wait("receiver authentication", Duration::from_secs(30), || receiver.text().contains("peer authenticated"));
    let (old_source, old) = fixture.prepare(&fixture.first, 2, 256 * 1024);
    fixture.authority.storage.commit_snapshot(&old).unwrap();
    fixture.wait_head(&mut receiver, old.manifest(), "initial nonempty signed checkpoint");
    swarm_core::verify_snapshot_signature(old.manifest()).unwrap();
    fixture.restore(old.manifest(), old_source.path(), "stress-old-restored");
    let old_hash = old.manifest_hash().unwrap();
    fixture
        .receiver
        .storage
        .set_world_replica_policy(
            world,
            ReplicaPolicy { follow: true, hot: true, utc_window: None, max_kib_per_second: Some(512) },
        )
        .unwrap();

    let (new_source, next) = fixture.prepare(old.manifest(), 1, 4 * 1024 * 1024);
    fixture.authority.storage.commit_snapshot(&next).unwrap();
    let descriptor = next.entries.iter().find(|entry| entry.blob.encoded_size > 3 * 1024 * 1024).unwrap().blob.clone();
    let offset = || fixture.receiver.storage.partial_blob_offset(world, &descriptor).unwrap();
    wait("durable partial payload", Duration::from_secs(45), || {
        let bytes = offset();
        bytes >= 64 * 1024 && bytes < descriptor.encoded_size
    });
    assert_eq!(fixture.receiver.storage.latest_snapshot(world).unwrap().as_ref(), Some(old.manifest()));
    receiver.stop();
    authority.stop();
    assert!(!receiver.alive() && !authority.alive());
    let reopened = Storage::open(&fixture.receiver.paths.root).unwrap();
    let partial = reopened.partial_blob_offset(world, &descriptor).unwrap();
    assert!(partial >= 64 * 1024 && partial < descriptor.encoded_size);
    assert_eq!(reopened.latest_snapshot(world).unwrap().as_ref(), Some(old.manifest()));
    reopened.verify_snapshot(old.manifest()).unwrap();
    fixture.restore(old.manifest(), old_source.path(), "stress-after-kill-restored");
    let mut seeder = fixture.seeder.start(&[], "stress-seeder.log");
    wait("surviving replica listener", Duration::from_secs(15), || seeder.text().contains("daemon listening"));
    let mut receiver = fixture.receiver.start(&[fixture.seeder.address()], "stress-receiver-restart.log");
    let elapsed =
        fixture.wait_head(&mut receiver, next.manifest(), "verified resume after receiver and original source SIGKILL");
    swarm_core::verify_snapshot_signature(next.manifest()).unwrap();
    fixture.restore(next.manifest(), new_source.path(), "stress-new-restored");
    // The complete original signed history, not just the last payload,
    // must still be verifiable after reconnection and restart.
    let history = reopened.list_snapshots(world).unwrap();
    assert_eq!(history, vec![fixture.first.clone(), old.manifest().clone(), next.manifest().clone()]);
    for snapshot in &history {
        swarm_core::verify_snapshot_signature(snapshot).unwrap();
        reopened.verify_snapshot(snapshot).unwrap();
    }
    wait("verified resumed pull receipt", Duration::from_secs(10), || {
        complete_line(&receiver.text(), next.snapshot_number).is_some()
    });
    let line = complete_line(&receiver.text(), next.snapshot_number).unwrap().to_owned();
    let resumed = metric(&line, "resumed");
    assert!(resumed > 0, "expected actual durable resume: {line}");
    assert_eq!(metric(&line, "sources"), 1);
    assert_eq!(metric(&line, "max_parallel"), 1);
    assert!(receiver.alive() && seeder.alive());
    receiver.stop();
    seeder.stop();
    assert!(!receiver.alive() && !seeder.alive());
    let final_store = Storage::open(&fixture.receiver.paths.root).unwrap();
    assert_eq!(final_store.latest_snapshot(world).unwrap().as_ref(), Some(next.manifest()));
    println!(
        "stress_receipt={}",
        serde_json::json!({
            "previous_verified_checkpoint": old_hash.to_string(),
            "last_accepted_checkpoint": next.manifest_hash().unwrap().to_string(),
            "previous_snapshot_number": old.snapshot_number,
            "final_snapshot_number": next.snapshot_number,
            "retained_signed_snapshots": history.len(),
            "fault": "SIGKILL receiving daemon and original source after durable partial",
            "receiver_partial_bytes": partial,
            "encoded_payload_bytes": descriptor.encoded_size,
            "payload_limit_kib_per_second": 512,
            "resumed_blobs": resumed,
            "resume_elapsed_ms": elapsed.as_millis(),
            "old_and_new_restores_byte_exact": true,
            "consent_absent": true,
            "all_daemons_exited": true
        })
    );
}

fn support_cli(device: &Device, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .arg("--data-dir")
        .arg(&device.paths.root)
        .args(args)
        .env("RUST_LOG", "warn")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn support_value(device: &Device, args: &[&str]) -> serde_json::Value {
    let result = support_cli(device, args);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    serde_json::from_slice(&result.stdout).unwrap()
}

fn has_observation(report: &serde_json::Value, metric: &str, outcome: &str) -> bool {
    report["events"].as_array().unwrap().iter().any(|event| event["metric"] == metric && event["outcome"] == outcome)
}

fn support_report(device: &Device) -> serde_json::Value {
    support_value(device, &["support", "report", "--confirm"])
}

/// Use real authenticated daemon processes and signed membership, with no
/// runtime configuration or hosting consent. This verifies operational
/// evidence, not a real Minecraft login or field authority transfer.
#[test]
fn operational_support_reports_actual_replication_lease_denial_restore_and_publication() {
    let fixture = Fixture::new();
    let world = fixture.world;
    let world_id = world.to_string();
    let authority = &fixture.authority;
    let receiver = &fixture.receiver;
    assert!(!support_report(authority)["collection_enabled"].as_bool().unwrap());
    assert!(!authority.paths.root.join("support-metrics-v1.json").exists());
    for device in [authority, receiver] {
        assert!(support_value(device, &["support", "configure", "--collection", "on"])["collection_enabled"]
            .as_bool()
            .unwrap());
    }
    // Force the receiver-driven path, rather than allowing legacy push.
    receiver
        .storage
        .set_world_replica_policy(
            world,
            ReplicaPolicy { follow: true, hot: true, utc_window: None, max_kib_per_second: Some(512) },
        )
        .unwrap();
    let mut source_daemon = authority.start(&[], "support-authority.log");
    wait("actual listener and missing lease quorum", Duration::from_secs(20), || {
        assert!(source_daemon.alive(), "authority daemon exited: {}", source_daemon.text());
        let report = support_report(authority);
        has_observation(&report, "storage_open", "completed")
            && has_observation(&report, "daemon_listener", "completed")
            && has_observation(&report, "lease_permit", "quorum_missing")
    });
    assert!(!swarm_cli::authority_permit::permit_path(&authority.paths, world).exists());
    let mut receiver_daemon = receiver.start(&[authority.address()], "support-receiver.log");
    wait("actual authenticated lease refresh", Duration::from_secs(30), || {
        assert!(receiver_daemon.alive() && source_daemon.alive());
        swarm_cli::authority_permit::permit_path(&authority.paths, world).is_file()
            && has_observation(&support_report(authority), "lease_permit", "permit_refreshed")
    });
    // A heartbeat is not permission to start Minecraft.
    assert!(swarm_cli::hosting_budget::load_consent(&authority.paths, world).unwrap().is_none());
    assert!(swarm_cli::migration::load_runtime_config(&authority.paths, world).is_err());

    let (source, next) = fixture.prepare(&fixture.first, 1, 1024 * 1024);
    authority.storage.commit_snapshot(&next).unwrap();
    fixture.wait_head(&mut receiver_daemon, next.manifest(), "support signed pull");
    fixture.restore(next.manifest(), source.path(), "support-fixture-restore");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let report = support_report(receiver);
        let log = receiver_daemon.text();
        if has_observation(&report, "replica_pull", "completed") && complete_line(&log, next.snapshot_number).is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "receiver pull observation missing: {report}; daemon log: {log}");
        thread::sleep(Duration::from_millis(50));
    }
    let report = support_report(receiver);
    let transfer = report["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["metric"] == "replica_pull" && event["outcome"] == "completed")
        .unwrap()
        .clone();
    let log = receiver_daemon.text();
    let line = complete_line(&log, next.snapshot_number).unwrap();
    assert_eq!(transfer["transfer"]["payload_bytes"], metric(line, "bytes"));
    assert_eq!(transfer["transfer"]["source_count"], metric(line, "sources"));
    assert_eq!(transfer["transfer"]["max_parallel"], metric(line, "max_parallel"));
    assert!(transfer["elapsed_ms"].is_number());

    let selected = support_value(receiver, &["support", "report", "--confirm", "--world", &world_id, "--live-route"]);
    assert_eq!(selected["selected_world_observation"]["live_route"]["outcome"], "gateway_unavailable");
    assert_eq!(selected["selected_world_observation"]["live_route"]["proof_fresh_at_observation"], false);
    let selection = support_cli(receiver, &["world", "game-route-select", &world_id]);
    assert!(selection.status.success(), "{}", String::from_utf8_lossy(&selection.stderr));
    // The real local Minecraft gateway must reject a new TCP session
    // without a fresh host proof. No Minecraft process is started here.
    let port =
        swarm_network::game_port_registry::GameWorldPortRegistry::new(receiver.paths.root.join("game-gateway-ports"))
            .lookup(world)
            .unwrap()
            .unwrap();
    let socket_addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut socket = std::net::TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2)).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(25))).unwrap();
    let mut byte = [0u8; 1];
    match std::io::Read::read(&mut socket, &mut byte) {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("unauthorized gateway session did not close: {other:?}"),
    }
    drop(socket);
    let denied = support_value(receiver, &["support", "report", "--confirm", "--world", &world_id, "--live-route"]);
    let route = &denied["selected_world_observation"]["live_route"];
    assert_eq!(route["availability"], "observed");
    assert_eq!(route["proof_fresh_at_observation"], false);
    assert!(
        matches!(
            route["recent_session"]["state"].as_str(),
            Some("route_refused" | "route_timed_out" | "stream_admission_failed")
        ),
        "{route}"
    );
    assert!(matches!(
        route["outcome"].as_str(),
        Some(
            "host_proof_unavailable"
                | "peer_link_unavailable"
                | "route_check_timed_out"
                | "authority_changed"
                | "unavailable"
        )
    ));
    wait("recorded route denial", Duration::from_secs(10), || {
        has_observation(&support_report(receiver), "game_route", route["outcome"].as_str().unwrap())
    });

    let recovered = receiver.temp.path().join("PRIVATE_RESTORE_PATH_CANARY");
    let restore = support_cli(
        receiver,
        &["world", "recover", &world_id, &next.snapshot_number.to_string(), recovered.to_str().unwrap()],
    );
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stderr));
    for entry in &next.entries {
        assert_eq!(fs::read(recovered.join(&entry.path)).unwrap(), fs::read(source.path().join(&entry.path)).unwrap());
    }
    assert!(has_observation(&support_report(receiver), "checkpoint_restore", "completed"));
    let bad_destination = receiver.temp.path().join("PRIVATE_BAD_RESTORE_CANARY");
    let failed = support_cli(receiver, &["world", "recover", &world_id, "99999", bad_destination.to_str().unwrap()]);
    assert!(!failed.status.success() && !bad_destination.exists());
    assert!(has_observation(&support_report(receiver), "checkpoint_restore", "failed"));
    let verify = support_cli(receiver, &["world", "verify", &world_id]);
    assert!(verify.status.success());
    assert!(has_observation(&support_report(receiver), "checkpoint_verify", "completed"));
    let raw = serde_json::to_string(&support_value(
        receiver,
        &["support", "report", "--confirm", "--world", &world_id, "--live-route"],
    ))
    .unwrap();
    for secret in [
        world_id.clone(),
        world.to_hex(),
        authority.identity.peer_id().to_string(),
        receiver.identity.peer_id().to_string(),
        authority.transport.clone(),
        receiver.transport.clone(),
        next.manifest_hash().unwrap().to_string(),
        next.state_root.to_string(),
        authority.address(),
        socket_addr.to_string(),
        receiver.paths.root.to_string_lossy().into_owned(),
        "Parallel replication fixture".into(),
        "PRIVATE_RESTORE_PATH_CANARY".into(),
        "PRIVATE_BAD_RESTORE_CANARY".into(),
    ] {
        assert!(!raw.contains(&secret), "redacted report leaked protected data");
    }

    // Remove the explicit rate limit with only one connected sender.
    // The next distinct checkpoint must use ordinary push, and its
    // finalization must be independently observable (no invented bytes).
    receiver.storage.set_world_replica_policy(world, ReplicaPolicy::default()).unwrap();
    let (push_source, pushed) = fixture.prepare(next.manifest(), 1, 128 * 1024);
    authority.storage.commit_snapshot(&pushed).unwrap();
    fixture.wait_head(&mut receiver_daemon, pushed.manifest(), "support ordinary push");
    fixture.restore(pushed.manifest(), push_source.path(), "support-push-restore");
    wait("measured ordinary replica finalization", Duration::from_secs(10), || {
        has_observation(&support_report(receiver), "replica_finalize", "completed")
    });

    receiver_daemon.stop();
    wait("actual lease removed after losing the peer", Duration::from_secs(15), || {
        !swarm_cli::authority_permit::permit_path(&authority.paths, world).exists()
    });
    let offline = support_value(receiver, &["support", "report", "--confirm", "--world", &world_id, "--live-route"]);
    assert_eq!(offline["selected_world_observation"]["live_route"]["availability"], "unavailable");
    assert_eq!(offline["selected_world_observation"]["live_route"]["proof_fresh_at_observation"], false);
    source_daemon.stop();
    // With all daemons stopped, explicitly publish a further signed
    // checkpoint using the actual CLI. No Minecraft process is running.
    let publish = support_cli(
        authority,
        &[
            "world",
            "snapshot",
            &world_id,
            source.path().to_str().unwrap(),
            "--epoch",
            &pushed.epoch.to_string(),
            "--sequence",
            &(pushed.sequence + 1).to_string(),
        ],
    );
    assert!(publish.status.success(), "{}", String::from_utf8_lossy(&publish.stderr));
    assert!(has_observation(&support_report(authority), "checkpoint_publication", "completed"));
    let rejected = support_cli(
        authority,
        &[
            "world",
            "snapshot",
            &world_id,
            "/nonexistent/PRIVATE_PUBLICATION_CANARY",
            "--epoch",
            &pushed.epoch.to_string(),
            "--sequence",
            &(pushed.sequence + 2).to_string(),
        ],
    );
    assert!(!rejected.status.success());
    assert!(has_observation(&support_report(authority), "checkpoint_publication", "failed"));
    let published = authority.storage.latest_snapshot(world).unwrap().unwrap();
    assert_eq!(published.snapshot_number, pushed.snapshot_number + 1);
    authority.storage.verify_snapshot(&published).unwrap();
    swarm_core::verify_snapshot_signature(&published).unwrap();
    assert_eq!(published.previous_snapshot_hash, Some(pushed.manifest_hash().unwrap()));
    let preference = support_cli(authority, &["world", "discovery-listing", &world_id, "--paused", "true"]);
    assert!(preference.status.success(), "{}", String::from_utf8_lossy(&preference.stderr));
    let log_path = authority.temp.path().join("support-discovery.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut publisher = Daemon {
        child: Command::new(env!("CARGO_BIN_EXE_swarmcraft-runtime"))
            .arg("--data-dir")
            .arg(&authority.paths.root)
            .args(["discovery-serve", "--listen", "/ip4/127.0.0.1/tcp/0"])
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
        log: log_path,
    };
    wait("actual private discovery refresh", Duration::from_secs(15), || {
        assert!(publisher.alive(), "discovery publisher exited: {}", publisher.text());
        has_observation(&support_report(authority), "discovery_refresh", "completed")
    });
    let final_report = support_value(authority, &["support", "report", "--confirm", "--world", &world_id]);
    assert_eq!(final_report["selected_world_observation"]["discovery_preference"]["locally_paused"], true);
    assert_eq!(
        final_report["selected_world_observation"]["discovery_preference"]["remote_publication_confirmed"],
        false
    );
    assert!(has_observation(&final_report, "discovery_preference", "completed"));
    // Clear and opt out while a previously enabled process is alive. Its
    // pending/periodic observations must not repopulate the cleared store.
    support_value(authority, &["support", "clear", "--confirm"]);
    support_value(authority, &["support", "configure", "--collection", "off"]);
    thread::sleep(Duration::from_secs(3));
    assert_eq!(support_report(authority)["retained_count"], 0);
    publisher.stop();
    assert!(!publisher.alive() && !source_daemon.alive() && !receiver_daemon.alive());
    assert!(swarm_cli::hosting_budget::load_consent(&authority.paths, world).unwrap().is_none());
    assert!(swarm_cli::hosting_budget::load_consent(&receiver.paths, world).unwrap().is_none());
    println!(
        "support_receipt={}",
        serde_json::json!({
            "source_summary":final_report["summary"],
            "receiver_summary":support_report(receiver)["summary"],
            "actual_replication_sample":transfer,
            "route_denial":route["outcome"],
            "actual_gateway_session":route["recent_session"],
            "offline_route_unavailable":true,
            "last_accepted_checkpoint_number":published.snapshot_number,
            "signed_head_verified":true,
            "restore_byte_exact":true,
            "ordinary_push_finalized_and_restored":true,
            "clear_and_pause_prevent_old_process_repopulation":true,
            "redaction_canaries_absent":true,
            "consent_absent_and_all_processes_exited":true
        })
    );
}
