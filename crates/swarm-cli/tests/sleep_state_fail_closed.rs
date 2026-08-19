use std::{fs, time::Duration};

use swarm_cli::migration::{self, MigrationPhase, RuntimeLaunchConfig};
use swarm_core::{create_world_genesis_with_fingerprint, DataPaths, PeerIdentity};
use swarm_protocol::{RuntimeCompatibilityManifestV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION};
use swarm_storage::{Storage, WorldMetadataV1};
use tokio::time::{sleep, timeout};

#[tokio::test]
async fn migration_supervisor_blocks_corrupt_sleep_state_before_runtime_launch() {
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path().join("data"));
    let storage = Storage::open(paths.root.clone()).unwrap();
    let identity = PeerIdentity::load_or_create(&paths).unwrap();
    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "26.1.2".into(),
        loader_id: "fabric".into(),
        loader_version: "0.19.3".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
        required_server_mods: Vec::new(),
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let fingerprint = compatibility.fingerprint().unwrap();
    let (world, genesis) = create_world_genesis_with_fingerprint(
        &identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )
    .unwrap();
    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "corrupt-sleep-supervisor".into(),
            world_id: world,
            genesis,
        })
        .unwrap();
    migration::save_runtime_config(
        &paths,
        world,
        &RuntimeLaunchConfig {
            java: temp.path().join("must-not-launch-java"),
            server_jar: temp.path().join("must-not-launch-server.jar"),
            mod_jar: temp.path().join("must-not-launch-mod.jar"),
            accept_eula: true,
            game_endpoint: Some("127.0.0.1:25565".into()),
        },
    )
    .unwrap();

    let metadata = storage.world_dir(world).join("metadata");
    fs::create_dir_all(&metadata).unwrap();
    fs::write(metadata.join("sleep.postcard"), b"corrupt-sleep-state").unwrap();

    let supervisor = tokio::spawn(migration::supervise(paths.clone()));
    let status = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(status) = migration::load_migration_status(&paths, world) {
                if status.phase == MigrationPhase::Blocked {
                    break status;
                }
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("migration supervisor did not publish a blocked corrupt-sleep status");

    assert!(!status.runtime_ready);
    let reason = status.failure_reason.unwrap_or_default();
    assert!(reason.contains("sleep state"), "unexpected block reason: {reason}");
    assert!(reason.contains("blocked"), "unexpected block reason: {reason}");
    assert!(!paths.root.join("runtime").join(world.to_hex()).exists());

    supervisor.abort();
}
