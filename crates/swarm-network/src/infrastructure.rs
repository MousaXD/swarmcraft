//! Locator-only discovery and bounded private relay transport. No world
//! storage, membership authority, runtime, or Minecraft listener is present.
use crate::{
    game_durable_path::{validate_durable_ancestors, validate_private_state_file},
    load_or_create_transport_key, DiscoveryNetworkEvent, DiscoveryNode, WireRequest, WireResponse,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use fs2::FileExt;
use futures::StreamExt;
use libp2p::{
    allow_block_list, connection_limits, identify,
    identity::Keypair,
    multiaddr::Protocol,
    noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use swarm_protocol::{peer_id_from_public_key, PeerHelloV1, PROTOCOL_VERSION};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_ALLOWED_PEERS: usize = 32;
const MAX_CIRCUIT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_CIRCUIT_SECONDS: u64 = 3600;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateRelayConfig {
    pub listen: String,
    pub advertise: Option<String>,
    pub allowed_transport_peers: Vec<String>,
    pub max_circuit_bytes: u64,
    pub max_circuit_seconds: u64,
    pub max_circuits: usize,
}

impl PrivateRelayConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= MAX_CONFIG_BYTES, "relay config exceeds size limit");
        serde_json::from_slice(&bytes).context("invalid private relay configuration")
    }

    fn validate(&self) -> Result<(Endpoint, HashSet<PeerId>, relay::Config)> {
        let endpoint = Endpoint::parse(&self.listen, self.advertise.as_deref(), true)?;
        ensure!(
            (1..=MAX_ALLOWED_PEERS).contains(&self.allowed_transport_peers.len()),
            "relay requires 1-32 explicitly allowed transport identities"
        );
        let allowed = self
            .allowed_transport_peers
            .iter()
            .map(|peer| peer.parse::<PeerId>().context("invalid allowed transport identity"))
            .collect::<Result<HashSet<_>>>()?;
        ensure!(allowed.len() == self.allowed_transport_peers.len(), "duplicate allowed transport identity");
        ensure!((4096..=MAX_CIRCUIT_BYTES).contains(&self.max_circuit_bytes), "circuit byte limit out of bounds");
        ensure!((1..=MAX_CIRCUIT_SECONDS).contains(&self.max_circuit_seconds), "circuit duration out of bounds");
        ensure!((1..=32).contains(&self.max_circuits), "circuit count out of bounds");
        // Retain libp2p's per-IP and per-peer rate limiters.
        let limits = relay::Config {
            max_reservations: MAX_ALLOWED_PEERS,
            max_reservations_per_peer: 1,
            reservation_duration: Duration::from_secs(30 * 60),
            max_circuits: self.max_circuits,
            max_circuits_per_peer: 2,
            max_circuit_duration: Duration::from_secs(self.max_circuit_seconds),
            max_circuit_bytes: self.max_circuit_bytes,
            ..Default::default()
        };
        Ok((endpoint, allowed, limits))
    }
}

struct Endpoint {
    listen: Multiaddr,
    advertise: Option<Multiaddr>,
}

impl Endpoint {
    fn parse(listen: &str, advertise: Option<&str>, tcp_only: bool) -> Result<Self> {
        let listen: Multiaddr = listen.parse().context("invalid listen endpoint")?;
        let wildcard = validate_endpoint(&listen, true, tcp_only)?;
        let advertise = advertise
            .map(|value| -> Result<Multiaddr> {
                let address: Multiaddr = value.parse().context("invalid advertised endpoint")?;
                validate_endpoint(&address, false, tcp_only)?;
                ensure!(is_tcp(&listen) == is_tcp(&address), "advertised endpoint protocol differs from listener");
                Ok(address)
            })
            .transpose()?;
        ensure!(!wildcard || advertise.is_some(), "wildcard listener requires explicit advertised address");
        Ok(Self { listen, advertise })
    }

    fn advertised(&self, actual: Multiaddr, peer: PeerId) -> Multiaddr {
        self.advertise.clone().unwrap_or(actual).with(Protocol::P2p(peer))
    }
}

fn is_tcp(address: &Multiaddr) -> bool {
    address.iter().any(|part| matches!(part, Protocol::Tcp(_)))
}

pub(crate) fn validate_endpoint(address: &Multiaddr, binding: bool, tcp_only: bool) -> Result<bool> {
    let mut parts = address.iter();
    let wildcard = match parts.next() {
        Some(Protocol::Ip4(ip)) if !ip.is_multicast() && !ip.is_broadcast() => ip.is_unspecified(),
        Some(Protocol::Ip6(ip)) if !ip.is_multicast() => ip.is_unspecified(),
        Some(Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_)) if !binding => false,
        _ => bail!("endpoint requires a concrete IP or advertised DNS name"),
    };
    ensure!(binding || !wildcard, "advertised endpoint must not be a wildcard");
    let port = match parts.next() {
        Some(Protocol::Tcp(port)) => port,
        Some(Protocol::Udp(port)) if !tcp_only => {
            ensure!(matches!(parts.next(), Some(Protocol::QuicV1)), "UDP locator requires QUIC v1");
            port
        }
        _ => bail!("unsupported infrastructure transport"),
    };
    ensure!(parts.next().is_none(), "unexpected endpoint suffix");
    ensure!(binding || port != 0, "advertised endpoint requires a nonzero port");
    Ok(wildcard)
}

/// Dedicated private root, exclusive ownership, and stable service identity.
/// Never adopt an existing player data directory or silently rotate bad keys.
pub struct InfrastructureState {
    root: PathBuf,
    _lock: File,
}

impl InfrastructureState {
    pub fn open(root: &Path, role: &str) -> Result<Self> {
        ensure!(matches!(role, "locator" | "relay"), "unknown infrastructure role");
        validate_durable_ancestors(root)?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(root)?;
        validate_durable_ancestors(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                fs::metadata(root)?.permissions().mode() & 0o077 == 0,
                "infrastructure state directory must be private (0700)"
            );
        }
        if !root.join("role-v1").exists() {
            for entry in fs::read_dir(root)? {
                ensure!(
                    entry?.file_name() == "service.lock",
                    "refusing to adopt a nonempty infrastructure state directory"
                );
            }
        }
        let lock_path = root.join("service.lock");
        validate_private_state_file(&lock_path)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path)?;
        FileExt::try_lock_exclusive(&lock).context("infrastructure state already owned by another process")?;
        let marker = root.join("role-v1");
        if validate_private_state_file(&marker)? {
            let expected = format!("{role}\n");
            ensure!(
                fs::metadata(&marker)?.len() == expected.len() as u64 && fs::read_to_string(&marker)? == expected,
                "infrastructure state role mismatch"
            );
        } else {
            for entry in fs::read_dir(root)? {
                ensure!(
                    entry?.file_name() == "service.lock",
                    "refusing to adopt a nonempty infrastructure state directory"
                );
            }
            let mut marker = options.create_new(true).open(&marker)?;
            writeln!(marker, "{role}")?;
            marker.sync_all()?;
            #[cfg(unix)]
            File::open(root)?.sync_all()?;
        }
        Ok(Self { root: root.to_owned(), _lock: lock })
    }

    fn key(&self, filename: &str) -> Result<Keypair> {
        let path = self.root.join(filename);
        for candidate in [&path, &path.with_extension("key.tmp")] {
            if validate_private_state_file(candidate)? {
                ensure!(fs::metadata(candidate)?.len() <= 1024, "infrastructure key exceeds size limit");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    ensure!(
                        fs::metadata(candidate)?.permissions().mode() & 0o077 == 0,
                        "infrastructure key is not private"
                    );
                }
            }
        }
        load_or_create_transport_key(&path)
    }

    pub fn transport_key(&self) -> Result<Keypair> {
        self.key("transport.key")
    }
}

// The allowlist and limits precede stateful relay handlers. No application
// protocol is present, so this service cannot become a world member.
#[allow(unreachable_code)]
mod generated {
    use super::*;
    #[derive(NetworkBehaviour)]
    pub(super) struct RelayBehaviour {
        pub(super) allowed: allow_block_list::Behaviour<allow_block_list::AllowedPeers>,
        pub(super) limits: connection_limits::Behaviour,
        pub(super) relay: relay::Behaviour,
        pub(super) ping: ping::Behaviour,
        pub(super) identify: identify::Behaviour,
    }
}
use generated::RelayBehaviour;

pub struct PrivateRelay {
    swarm: Swarm<RelayBehaviour>,
    advertised: Multiaddr,
}

impl PrivateRelay {
    pub async fn bind(key: Keypair, config: &PrivateRelayConfig) -> Result<Self> {
        let (endpoint, allowed, limits) = config.validate()?;
        let peer = key.public().to_peer_id();
        ensure!(!allowed.contains(&peer), "relay cannot allow its own transport identity");
        let mut admission = allow_block_list::Behaviour::default();
        for allowed_peer in allowed {
            admission.allow_peer(allowed_peer);
        }
        let mut swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_tcp(tcp::Config::default().nodelay(true), noise::Config::new, yamux::Config::default)?
            .with_behaviour(move |key| RelayBehaviour {
                allowed: admission,
                limits: connection_limits::Behaviour::new(
                    connection_limits::ConnectionLimits::default()
                        .with_max_pending_incoming(Some(32))
                        .with_max_pending_outgoing(Some(8))
                        .with_max_established(Some(64))
                        .with_max_established_per_peer(Some(2)),
                ),
                relay: relay::Behaviour::new(peer, limits),
                ping: ping::Behaviour::default(),
                identify: identify::Behaviour::new(identify::Config::new(
                    concat!("/", "swarmcraft", "/private-relay/1").into(),
                    key.public(),
                )),
            })?
            .with_swarm_config(|config| {
                config.with_idle_connection_timeout(Duration::from_secs(60)).with_max_negotiating_inbound_streams(16)
            })
            .build();
        let listener = swarm.listen_on(endpoint.listen.clone())?;
        let actual = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match swarm.select_next_some().await {
                    SwarmEvent::NewListenAddr { listener_id, address } if listener_id == listener => {
                        return Ok(address)
                    }
                    SwarmEvent::ListenerError { listener_id, .. } | SwarmEvent::ListenerClosed { listener_id, .. }
                        if listener_id == listener =>
                    {
                        bail!("private relay listener failed")
                    }
                    _ => {}
                }
            }
        })
        .await
        .context("private relay listener did not start")??;
        let advertised = endpoint.advertised(actual, peer);
        swarm.add_external_address(advertised.clone());
        Ok(Self { swarm, advertised })
    }

    pub fn advertised_address(&self) -> &Multiaddr {
        &self.advertised
    }

    pub async fn run(mut self) -> Result<()> {
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::ListenerError { .. } | SwarmEvent::ListenerClosed { .. } => {
                    bail!("private relay listener stopped");
                }
                // No remote identities, addresses, circuit events or packets
                // are written to service logs.
                _ => {}
            }
        }
    }
}

/// Only a transport-bound discovery hello is signed. This independent
/// process has no world or presence publication and no authority keys.
pub async fn serve_locator(state: &InfrastructureState, listen: &str, advertise: Option<&str>) -> Result<()> {
    let endpoint = Endpoint::parse(listen, advertise, false)?;
    let identity =
        state.key("locator-app.key")?.try_into_ed25519().map_err(|_| anyhow!("invalid locator application key"))?;
    let signing = SigningKey::from_keypair_bytes(&identity.to_bytes())?;
    let public_key = signing.verifying_key().to_bytes();
    let mut nonce = [0; 32];
    OsRng.fill_bytes(&mut nonce);
    let mut hello = PeerHelloV1 {
        peer_id: peer_id_from_public_key(&public_key),
        public_key,
        protocol_versions: vec![PROTOCOL_VERSION],
        capabilities: vec!["discovery-v1".into(), "locator-only-v1".into()],
        nonce,
        signature: Vec::new(),
    };
    hello.signature = signing.sign(&hello.signing_bytes()?).to_bytes().to_vec();
    let mut node = DiscoveryNode::new_locator(state.transport_key()?, hello, signing)?;
    node.listen(endpoint.listen.clone())?;
    let actual = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let DiscoveryNetworkEvent::Listening { address } = node.next_event().await? {
                return Ok::<_, anyhow::Error>(address);
            }
        }
    })
    .await
    .context("locator listener did not start")??;
    let advertised = endpoint.advertised(actual, node.local_transport_peer_id());
    node.add_locator_external_address(advertised.clone())?;
    println!("Locator address: {advertised}");
    loop {
        if let DiscoveryNetworkEvent::InboundRequest { request, channel, .. } = node.next_event().await? {
            let response = match request {
                WireRequest::DiscoveryPublic { .. } => WireResponse::DiscoveryWorlds(Vec::new()),
                WireRequest::DiscoveryResolve { .. } => WireResponse::DiscoveryResolved(None),
                WireRequest::DiscoveryFreshnessContext { .. } => WireResponse::DiscoveryFreshnessContext(None),
                WireRequest::DiscoveryFreshnessVote(_) => WireResponse::DiscoveryFreshnessVote(None),
                WireRequest::FriendPresence { .. } => WireResponse::FriendPresence(None),
                _ => {
                    WireResponse::Error { code: "LOCATOR_ONLY".into(), message: "No application world service".into() }
                }
            };
            let _ = node.respond(channel, response);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_explicit_direct_and_protocol_matched() {
        assert!(Endpoint::parse("/ip4/0.0.0.0/tcp/4000", None, true).is_err());
        assert!(Endpoint::parse("/ip4/0.0.0.0/tcp/4000", Some("/dns4/relay.example/tcp/4000"), true).is_ok());
        assert!(Endpoint::parse("/ip4/127.0.0.1/tcp/0", None, true).is_ok());
        assert!(Endpoint::parse("/ip4/127.0.0.1/udp/4000/quic-v1", None, true).is_err());
        assert!(
            Endpoint::parse("/ip6/::/udp/4000/quic-v1", Some("/dns6/locator.example/udp/4000/quic-v1"), false).is_ok()
        );
        for invalid in [
            "/ip4/0.0.0.0/tcp/4000",
            "/dns4/relay.example/tcp/0",
            "/dns4/relay.example/tcp/4000/p2p-circuit",
            "/dns4/relay.example/udp/4000/quic-v1",
        ] {
            assert!(Endpoint::parse("/ip4/127.0.0.1/tcp/4000", Some(invalid), true).is_err());
        }
    }

    #[test]
    fn private_relay_has_no_anonymous_or_unbounded_configuration() {
        let mut config = PrivateRelayConfig {
            listen: "/ip4/127.0.0.1/tcp/0".into(),
            advertise: None,
            allowed_transport_peers: Vec::new(),
            max_circuit_bytes: 65536,
            max_circuit_seconds: 10,
            max_circuits: 2,
        };
        assert!(config.validate().is_err());
        config.allowed_transport_peers.push(Keypair::generate_ed25519().public().to_peer_id().to_string());
        assert!(config.validate().is_ok());
        config.allowed_transport_peers.push(config.allowed_transport_peers[0].clone());
        assert!(config.validate().is_err());
        config.allowed_transport_peers.pop();
        config.max_circuit_bytes = u64::MAX;
        assert!(config.validate().is_err());
        config.max_circuit_bytes = 65536;
        config.max_circuit_seconds = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn state_is_exclusive_role_bound_and_preserves_identity() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = root.path().join("infra");
        let first = InfrastructureState::open(&path, "relay").unwrap();
        let identity = first.transport_key().unwrap().public().to_peer_id();
        assert!(InfrastructureState::open(&path, "relay").is_err());
        drop(first);
        assert!(InfrastructureState::open(&path, "locator").is_err());
        let reopened = InfrastructureState::open(&path, "relay").unwrap();
        assert_eq!(reopened.transport_key().unwrap().public().to_peer_id(), identity);
        let other = root.path().join("player-data");
        fs::create_dir(&other).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(other.join("identity"), "not a service root").unwrap();
        assert!(InfrastructureState::open(&other, "relay").is_err());
    }
}
