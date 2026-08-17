use std::{fs, process::Command};
use swarm_network::{ConnectivityDiagnosticsV1, NatStatusV1, CONNECTIVITY_DIAGNOSTICS_SNAPSHOT_FILE};
use tempfile::TempDir;

#[test]
fn connectivity_json_command_is_node_scoped_and_machine_readable() {
    let temp = TempDir::new().unwrap();
    let mut diagnostics = ConnectivityDiagnosticsV1::default();
    diagnostics.record_nat_status(NatStatusV1::Public);
    diagnostics.record_local_address("/ip4/127.0.0.1/udp/4000/quic-v1");
    diagnostics.record_active_paths(1, 0, 0);
    fs::write(temp.path().join(CONNECTIVITY_DIAGNOSTICS_SNAPSHOT_FILE), postcard::to_allocvec(&diagnostics).unwrap())
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_swarmcraft"))
        .args(["--data-dir", temp.path().to_str().unwrap(), "diagnostics", "connectivity", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["state"], "direct_reachable");
    assert_eq!(value["nat_status"], "public");
    assert_eq!(value["local_addresses"][0], "/ip4/127.0.0.1/udp/4000/quic-v1");
    assert_eq!(value["ipv4_available"], true);
    assert_eq!(value["ipv6_available"], false);
    assert!(value.get("world_id").is_none());
    assert!(value.get("private_key").is_none());
    assert!(value.get("ipc_token").is_none());
}
