use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use std::{env, time::Duration};
use swarm_network::{generate_transport_key, SwarmNode};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};
use tokio::time::{timeout_at, Instant};

const PROBE_TARGET_ENV: &str = "SWARMCRAFT_PROBE_TARGET";
const PROBE_SECONDS_ENV: &str = "SWARMCRAFT_PROBE_SECONDS";

fn signed_probe_hello() -> (PeerHelloV1, SigningKey) {
    let key = SigningKey::generate(&mut OsRng);
    let public_key = key.verifying_key().to_bytes();
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["connectivity-probe-v1".into()],
        nonce: [0xC7; 32],
        signature: Vec::new(),
    };
    hello.signature = key.sign(&hello.signing_bytes().expect("probe hello should encode")).to_bytes().to_vec();
    (hello, key)
}

#[tokio::main]
async fn main() -> Result<()> {
    let (hello, signing_key) = signed_probe_hello();
    let mut node = SwarmNode::new(generate_transport_key(), hello, signing_key)?;
    node.listen("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;

    if let Err(error) = node.listen("/ip6/::/udp/0/quic-v1".parse()?) {
        eprintln!("IPv6 listen unavailable: {error}");
    }

    if let Ok(target) = env::var(PROBE_TARGET_ENV) {
        let address = target.parse().with_context(|| format!("invalid {PROBE_TARGET_ENV} multiaddress: {target}"))?;
        node.dial(address)?;
    }

    let probe_seconds =
        env::var(PROBE_SECONDS_ENV).ok().and_then(|value| value.parse::<u64>().ok()).unwrap_or(30).clamp(5, 300);
    let deadline = Instant::now() + Duration::from_secs(probe_seconds);

    println!("{:#?}", node.connectivity_diagnostics());
    loop {
        let event = match timeout_at(deadline, node.next_event()).await {
            Ok(event) => event?,
            Err(_) => break,
        };
        eprintln!("{event:?}");
        println!("{:#?}", node.connectivity_diagnostics());
    }

    Ok(())
}
