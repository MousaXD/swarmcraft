use crate::{
    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, wire::WireRequest, wire::WireResponse,
    ConnectivityDiagnosticsV1, ConnectivityIssueKindV1, ConnectivityIssueV1, NatStatusV1,
};
use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;
use futures::StreamExt;
use libp2p::multiaddr::Protocol;
use libp2p::{
    autonat, dcutr, identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    mdns, noise, ping, relay,
    request_response::{self, cbor, ProtocolSupport},
    swarm::{dial_opts::DialOpts, ConnectionId, NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId as TransportPeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use rand_core::{OsRng, RngCore};
use std::{
    collections::{HashMap, HashSet},
    env,
    time::Duration,
};
use swarm_protocol::{PeerHelloV1, PeerId, PROTOCOL_VERSION};
use tracing::{debug, info, warn};

pub const WIRE_PROTOCOL: &str = "/swarmcraft/1";
pub const BOOTSTRAP_ENV: &str = "SWARMCRAFT_BOOTSTRAP";
pub const RELAY_ENV: &str = "SWARMCRAFT_RELAY";

#[derive(NetworkBehaviour)]
struct Behaviour {
    request_response: cbor::Behaviour<WireRequest, WireResponse>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    kad: kad::Behaviour<MemoryStore>,
    relay_client: relay::client::Behaviour,
    dcutr: dcutr::Behaviour,
    auto_nat: autonat::Behaviour,
}

#[derive(Debug)]
pub enum NetworkEvent {
    Listening {
        address: Multiaddr,
    },
    Discovered {
        transport_peer: TransportPeerId,
        address: Multiaddr,
    },
    Connected {
        transport_peer: TransportPeerId,
    },
    Disconnected {
        transport_peer: TransportPeerId,
    },
    Authenticated {
        transport_peer: TransportPeerId,
        application_peer: PeerId,
    },
    InboundRequest {
        transport_peer: TransportPeerId,
        request: WireRequest,
        channel: request_response::ResponseChannel<WireResponse>,
    },
    Response {
        transport_peer: TransportPeerId,
        request_id: request_response::OutboundRequestId,
        response: WireResponse,
    },
    OutboundFailure {
        transport_peer: TransportPeerId,
        request_id: request_response::OutboundRequestId,
        error: String,
    },
}

#[derive(Debug, Clone)]
struct RelayFallbackPlan {
    relay_address: Multiaddr,
    attempted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionPathKind {
    DirectApplication,
    RelayedApplication,
    BootstrapInfrastructure,
    RelayInfrastructure,
}

pub struct SwarmNode {
    swarm: Swarm<Behaviour>,
    local_hello: PeerHelloV1,
    application_signing_key: SigningKey,
    authenticated: HashMap<TransportPeerId, (PeerId, ConnectionId)>,
    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32])>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    connection_counts: HashMap<TransportPeerId, usize>,
    connection_paths: HashMap<ConnectionId, ConnectionPathKind>,
    bootstrap_peers: HashSet<TransportPeerId>,
    relay_peers: HashSet<TransportPeerId>,
    relay_fallbacks: HashMap<TransportPeerId, RelayFallbackPlan>,
    diagnostics: ConnectivityDiagnosticsV1,
}

impl SwarmNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1, application_signing_key: SigningKey) -> Result<Self> {
        verify_peer_hello(&local_hello).context("local peer hello must be valid before networking starts")?;

        let local_peer = transport_key.public().to_peer_id();
        let self_test =
            build_peer_hello_proof(&local_hello, &application_signing_key, [0; 32], &local_peer, &local_peer)?;
        verify_peer_hello_proof(&self_test, [0; 32], &local_peer, &local_peer)
            .context("application signing key must match the local PeerHello")?;
        let request_response = cbor::Behaviour::new(
            [(StreamProtocol::new(WIRE_PROTOCOL), ProtocolSupport::Full)],
            request_response::Config::default()
                .with_request_timeout(Duration::from_secs(30))
                .with_max_concurrent_streams(128),
        );
        let mdns =
            mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer).context("failed to initialize mDNS")?;
        let identify =
            identify::Behaviour::new(identify::Config::new(WIRE_PROTOCOL.to_owned(), transport_key.public()));
        let mut kad_config = kad::Config::default();
        kad_config.set_query_timeout(Duration::from_secs(30));
        let kad = kad::Behaviour::with_config(local_peer, MemoryStore::new(local_peer), kad_config);
        let dcutr = dcutr::Behaviour::new(local_peer);
        let auto_nat = autonat::Behaviour::new(
            local_peer,
            autonat::Config {
                retry_interval: Duration::from_secs(15),
                refresh_interval: Duration::from_secs(60),
                boot_delay: Duration::from_secs(5),
                ..Default::default()
            },
        );

        let swarm = SwarmBuilder::with_existing_identity(transport_key)
            .with_tokio()
            .with_tcp(tcp::Config::default().nodelay(true), noise::Config::new, yamux::Config::default)?
            .with_quic()
            .with_dns()?
            .with_relay_client(noise::Config::new, yamux::Config::default)?
            .with_behaviour(move |_, relay_client| Behaviour {
                request_response,
                mdns,
                identify,
                ping: ping::Behaviour::default(),
                kad,
                relay_client,
                dcutr,
                auto_nat,
            })?
            .build();

        let mut node = Self {
            swarm,
            local_hello,
            application_signing_key,
            authenticated: HashMap::new(),
            pending_challenges: HashMap::new(),
            active_connections: HashMap::new(),
            connection_counts: HashMap::new(),
            connection_paths: HashMap::new(),
            bootstrap_peers: HashSet::new(),
            relay_peers: HashSet::new(),
            relay_fallbacks: HashMap::new(),
            diagnostics: ConnectivityDiagnosticsV1::default(),
        };
        node.configure_from_environment()?;
        Ok(node)
    }

    pub fn local_transport_peer_id(&self) -> TransportPeerId {
        *self.swarm.local_peer_id()
    }

    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).map(|(peer, _)| *peer)
    }

    pub fn connectivity_diagnostics(&self) -> ConnectivityDiagnosticsV1 {
        self.diagnostics.clone()
    }

    pub fn listen(&mut self, address: Multiaddr) -> Result<()> {
        self.swarm.listen_on(address).map(|_| ()).context("failed to listen")
    }

    pub fn dial(&mut self, address: Multiaddr) -> Result<()> {
        if let Some(peer) = transport_peer_from_address(&address) {
            self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
        }
        if let Err(error) = self.swarm.dial(address.clone()) {
            self.record_issue(
                ConnectivityIssueKindV1::DirectDialFailed,
                transport_peer_from_address(&address),
                Some(&address),
                error.to_string(),
            );
            return Err(error).context("failed to dial peer");
        }
        Ok(())
    }

    pub fn dial_known_peer(&mut self, peer: TransportPeerId, address: Multiaddr) -> Result<()> {
        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
        let options = DialOpts::peer_id(peer).addresses(vec![address.clone()]).build();
        if let Err(error) = self.swarm.dial(options) {
            self.record_issue(ConnectivityIssueKindV1::DirectDialFailed, Some(peer), Some(&address), error.to_string());
            return Err(error).context("failed to dial known peer");
        }
        Ok(())
    }

    /// Prefer direct addresses and make at most one relay fallback attempt if the
    /// direct dial fails. Calling this method again replaces the prior dial plan.
    pub fn dial_with_relay_fallback(
        &mut self,
        remote_peer: TransportPeerId,
        direct_addresses: Vec<Multiaddr>,
        relay_peer: TransportPeerId,
        relay_address: Multiaddr,
    ) -> Result<()> {
        let relay_base = ensure_peer_suffix(relay_address, relay_peer);
        let relay_circuit = relay_base.clone().with(Protocol::P2pCircuit).with(Protocol::P2p(remote_peer));
        self.relay_peers.insert(relay_peer);
        self.diagnostics.record_relay_configured(relay_base.to_string(), self.relay_peers.len());
        self.swarm.behaviour_mut().kad.add_address(&relay_peer, relay_base.clone());
        self.swarm.behaviour_mut().auto_nat.add_server(relay_peer, Some(relay_base));
        self.relay_fallbacks.insert(remote_peer, RelayFallbackPlan { relay_address: relay_circuit, attempted: false });

        let direct_addresses: Vec<_> = direct_addresses
            .into_iter()
            .filter(|address| !address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)))
            .collect();

        if direct_addresses.is_empty() {
            self.record_issue(
                ConnectivityIssueKindV1::DirectDialFailed,
                Some(remote_peer),
                None,
                "no direct address is available; relay fallback is required",
            );
            self.try_relay_fallback(remote_peer)?;
            return Ok(());
        }

        for address in &direct_addresses {
            self.swarm.behaviour_mut().kad.add_address(&remote_peer, address.clone());
        }
        let options = DialOpts::peer_id(remote_peer).addresses(direct_addresses).build();
        if let Err(error) = self.swarm.dial(options) {
            self.record_issue(ConnectivityIssueKindV1::DirectDialFailed, Some(remote_peer), None, error.to_string());
            self.try_relay_fallback(remote_peer)?;
        }
        Ok(())
    }

    pub fn add_bootstrap_peer(&mut self, peer: TransportPeerId, address: Multiaddr) {
        self.bootstrap_peers.insert(peer);
        self.diagnostics.record_bootstrap_configured(self.bootstrap_peers.len());
        self.swarm.behaviour_mut().kad.add_address(&peer, address);
    }

    pub fn add_bootstrap_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let Some(peer) = transport_peer_from_address(&address) else {
            self.record_issue(
                ConnectivityIssueKindV1::InvalidAddress,
                None,
                Some(&address),
                "bootstrap address must contain /p2p/<peer-id>",
            );
            return Err(anyhow!("bootstrap address must contain /p2p/<peer-id>"));
        };
        self.add_bootstrap_peer(peer, address.clone());
        if !self.swarm.is_connected(&peer) {
            if let Err(error) = self.swarm.dial(address.clone()) {
                self.record_issue(
                    ConnectivityIssueKindV1::BootstrapUnavailable,
                    Some(peer),
                    Some(&address),
                    error.to_string(),
                );
                return Err(error).context("failed to dial bootstrap peer");
            }
        }
        Ok(peer)
    }

    pub fn bootstrap(&mut self) -> Result<()> {
        match self.swarm.behaviour_mut().kad.bootstrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                self.record_issue(ConnectivityIssueKindV1::BootstrapUnavailable, None, None, error.to_string());
                Err(error).context("Kademlia bootstrap failed")
            }
        }
    }

    pub fn configure_relay(&mut self, relay_peer: TransportPeerId, relay_address: Multiaddr) -> Result<()> {
        let relay_address = ensure_peer_suffix(relay_address, relay_peer);
        self.relay_peers.insert(relay_peer);
        self.diagnostics.record_relay_configured(relay_address.to_string(), self.relay_peers.len());
        self.swarm.behaviour_mut().kad.add_address(&relay_peer, relay_address.clone());
        self.swarm.behaviour_mut().auto_nat.add_server(relay_peer, Some(relay_address.clone()));
        if !self.swarm.is_connected(&relay_peer) {
            if let Err(error) = self.swarm.dial(relay_address.clone()) {
                self.record_issue(
                    ConnectivityIssueKindV1::RelayUnavailable,
                    Some(relay_peer),
                    Some(&relay_address),
                    error.to_string(),
                );
                return Err(error).context("failed to dial relay");
            }
        }
        self.swarm
            .listen_on(relay_address.with(Protocol::P2pCircuit))
            .map(|_| ())
            .context("failed to request relay reservation")
    }

    pub fn configure_relay_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let Some(peer) = transport_peer_from_address(&address) else {
            self.record_issue(
                ConnectivityIssueKindV1::InvalidAddress,
                None,
                Some(&address),
                "relay address must contain /p2p/<peer-id>",
            );
            return Err(anyhow!("relay address must contain /p2p/<peer-id>"));
        };
        self.configure_relay(peer, address)?;
        Ok(peer)
    }

    pub fn dial_via_relay(
        &mut self,
        relay_peer: TransportPeerId,
        relay_address: Multiaddr,
        remote_peer: TransportPeerId,
    ) -> Result<()> {
        let relay_base = ensure_peer_suffix(relay_address, relay_peer);
        self.relay_peers.insert(relay_peer);
        self.diagnostics.record_relay_configured(relay_base.to_string(), self.relay_peers.len());
        let address = relay_base.with(Protocol::P2pCircuit).with(Protocol::P2p(remote_peer));
        if let Err(error) = self.swarm.dial(address.clone()) {
            self.record_issue(
                ConnectivityIssueKindV1::RelayUnavailable,
                Some(remote_peer),
                Some(&address),
                error.to_string(),
            );
            return Err(error).context("failed to dial peer through relay");
        }
        Ok(())
    }

    pub fn add_autonat_server(&mut self, peer: TransportPeerId, address: Multiaddr) {
        self.swarm.behaviour_mut().auto_nat.add_server(peer, Some(address));
    }

    pub fn configure_from_environment(&mut self) -> Result<()> {
        let bootstraps = configured_multiaddrs(BOOTSTRAP_ENV)?;
        for address in &bootstraps {
            self.add_bootstrap_address(address.clone())?;
        }
        if !bootstraps.is_empty() {
            self.bootstrap()?;
        }

        for address in configured_multiaddrs(RELAY_ENV)? {
            self.configure_relay_address(address)?;
        }
        Ok(())
    }

    pub fn send_request(
        &mut self,
        peer: &TransportPeerId,
        request: WireRequest,
    ) -> Result<request_response::OutboundRequestId> {
        request.validate_limits()?;
        Ok(self.swarm.behaviour_mut().request_response.send_request(peer, request))
    }

    pub fn respond(
        &mut self,
        channel: request_response::ResponseChannel<WireResponse>,
        response: WireResponse,
    ) -> Result<()> {
        self.swarm
            .behaviour_mut()
            .request_response
            .send_response(channel, response)
            .map_err(|_| anyhow!("response channel closed"))
    }

    pub async fn next_event(&mut self) -> Result<NetworkEvent> {
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => {
                    self.diagnostics.record_local_address(address.to_string());
                    info!(%address, "network listening");
                    return Ok(NetworkEvent::Listening { address });
                }
                SwarmEvent::ExpiredListenAddr { address, .. } => {
                    self.diagnostics.remove_local_address(&address.to_string());
                    debug!(%address, "network listen address expired");
                }
                SwarmEvent::ConnectionEstablished { peer_id, connection_id, endpoint, num_established, .. } => {
                    let path_kind = classify_connection_path(
                        self.bootstrap_peers.contains(&peer_id),
                        self.relay_peers.contains(&peer_id),
                        endpoint.is_relayed(),
                    );
                    self.connection_paths.insert(connection_id, path_kind);
                    self.refresh_connectivity_paths();
                    if matches!(
                        path_kind,
                        ConnectionPathKind::DirectApplication | ConnectionPathKind::RelayedApplication
                    ) {
                        self.relay_fallbacks.remove(&peer_id);
                    }
                    self.connection_counts.insert(peer_id, num_established.get() as usize);
                    self.authenticated.remove(&peer_id);
                    self.pending_challenges.remove(&peer_id);
                    debug!(transport_peer = %peer_id, %connection_id, %num_established, relayed = endpoint.is_relayed(), ?path_kind, "peer connected");

                    // request-response selects by peer ID. During replacement races,
                    // close the superseded route before issuing a fresh receiver
                    // challenge so proof traffic can only use the canonical connection.
                    let previous = self.active_connections.insert(peer_id, connection_id);
                    let defer_challenge = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_challenge {
                        self.issue_auth_challenge(peer_id, connection_id)?;
                    }
                    return Ok(NetworkEvent::Connected { transport_peer: peer_id });
                }
                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    self.connection_paths.remove(&connection_id);
                    self.refresh_connectivity_paths();
                    self.connection_counts.insert(peer_id, num_established as usize);
                    if self
                        .authenticated
                        .get(&peer_id)
                        .is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id)
                    {
                        self.authenticated.remove(&peer_id);
                    }
                    if self
                        .pending_challenges
                        .get(&peer_id)
                        .is_some_and(|(challenge_connection, _)| *challenge_connection == connection_id)
                    {
                        self.pending_challenges.remove(&peer_id);
                    }
                    if num_established == 0 {
                        self.active_connections.remove(&peer_id);
                        self.connection_counts.remove(&peer_id);
                        self.authenticated.remove(&peer_id);
                        self.pending_challenges.remove(&peer_id);
                        return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });
                    }

                    if let Some(active) =
                        self.active_connections.get(&peer_id).copied().filter(|active| *active != connection_id)
                    {
                        self.issue_auth_challenge(peer_id, active)?;
                    }
                    debug!(transport_peer = %peer_id, %connection_id, remaining_connections = num_established, "peer connection closed; replacement requires fresh application proof");
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    let error_text = error.to_string();
                    if let Some(peer) = peer_id {
                        if self.relay_fallbacks.get(&peer).is_some_and(|plan| !plan.attempted) {
                            self.record_issue(ConnectivityIssueKindV1::DirectDialFailed, Some(peer), None, error_text);
                            if let Err(fallback_error) = self.try_relay_fallback(peer) {
                                warn!(transport_peer = %peer, error = %fallback_error, "relay fallback could not be started");
                            }
                            continue;
                        }
                        if self.relay_fallbacks.get(&peer).is_some_and(|plan| plan.attempted) {
                            self.record_issue(ConnectivityIssueKindV1::RelayUnavailable, Some(peer), None, error_text);
                            self.diagnostics.record_no_viable_path(format!(
                                "direct and relay paths failed for transport peer {peer}"
                            ));
                            self.relay_fallbacks.remove(&peer);
                            continue;
                        }
                        if self.bootstrap_peers.contains(&peer) {
                            self.record_issue(
                                ConnectivityIssueKindV1::BootstrapUnavailable,
                                Some(peer),
                                None,
                                error_text,
                            );
                        } else if self.relay_peers.contains(&peer) {
                            self.record_issue(ConnectivityIssueKindV1::RelayUnavailable, Some(peer), None, error_text);
                        } else {
                            self.record_issue(ConnectivityIssueKindV1::DirectDialFailed, Some(peer), None, error_text);
                        }
                    } else {
                        self.record_issue(ConnectivityIssueKindV1::DirectDialFailed, None, None, error_text);
                    }
                    warn!(transport_peer = ?peer_id, error = %error, "outgoing connection failed");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                    if let Some((peer, address)) = peers.into_iter().next() {
                        debug!(transport_peer = %peer, %address, "mDNS discovered peer");
                        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
                        if !self.swarm.is_connected(&peer) {
                            if let Err(error) = self.swarm.dial(address.clone()) {
                                self.record_issue(
                                    ConnectivityIssueKindV1::DirectDialFailed,
                                    Some(peer),
                                    Some(&address),
                                    error.to_string(),
                                );
                                warn!(transport_peer = %peer, %address, %error, "mDNS peer dial failed");
                            }
                        }
                        return Ok(NetworkEvent::Discovered { transport_peer: peer, address });
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(peers))) => {
                    for (peer, address) in peers {
                        self.swarm.behaviour_mut().kad.remove_address(&peer, &address);
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                    peer_id, info, ..
                })) => {
                    self.diagnostics.record_observed_address(info.observed_addr.to_string());
                    for address in info.listen_addrs {
                        self.swarm.behaviour_mut().kad.add_address(&peer_id, address);
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(event)) => {
                    match event {
                        request_response::Event::Message { peer, connection_id, message } => {
                            match message {
                                request_response::Message::Request { request, channel, .. } => {
                                    if let Err(error) = request.validate_limits() {
                                        let response = WireResponse::Error {
                                            code: "REQUEST_LIMIT_EXCEEDED".into(),
                                            message: error.to_string(),
                                        };
                                        if let Err(response_error) = self.respond(channel, response) {
                                            warn!(
                                                transport_peer = %peer,
                                                error = %response_error,
                                                "failed to send request limit error; continuing network loop"
                                            );
                                        }
                                        continue;
                                    }
                                    match request {
                                WireRequest::Hello(_) => {
                                    let _ = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: "CONNECTION_PROOF_REQUIRED".into(),
                                            message: "reusable PeerHello is not an authentication proof; wait for a receiver challenge".into(),
                                        },
                                    );
                                }
                                WireRequest::HelloChallenge { challenge } => {
                                    let canonical = self.active_connections.get(&peer).is_some_and(|active| *active == connection_id)
                                        && self.connection_counts.get(&peer).copied() == Some(1);
                                    if !canonical {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "AUTH_CONNECTION_RETRY".into(),
                                                message: "authentication challenge arrived on a superseded connection".into(),
                                            },
                                        );
                                        continue;
                                    }
                                    let local_transport = *self.swarm.local_peer_id();
                                    let proof = build_peer_hello_proof(
                                        &self.local_hello,
                                        &self.application_signing_key,
                                        challenge,
                                        &local_transport,
                                        &peer,
                                    )?;
                                    self.respond(channel, WireResponse::HelloChallengeAccepted)?;
                                    self.swarm
                                        .behaviour_mut()
                                        .request_response
                                        .send_request(&peer, WireRequest::HelloProof(Box::new(proof)));
                                }
                                WireRequest::HelloProof(proof) => {
                                    let expected = self.pending_challenges.remove(&peer);
                                    let Some((challenge_connection, expected_challenge)) = expected else {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "PEER_AUTHENTICATION_FAILED".into(),
                                                message: "no live receiver challenge exists for this proof".into(),
                                            },
                                        );
                                        continue;
                                    };
                                    if challenge_connection != connection_id
                                        || self.active_connections.get(&peer).is_none_or(|active| *active != connection_id)
                                        || self.connection_counts.get(&peer).copied() != Some(1)
                                    {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "PEER_AUTHENTICATION_FAILED".into(),
                                                message: "connection was replaced before proof verification".into(),
                                            },
                                        );
                                        continue;
                                    }
                                    match verify_peer_hello_proof(
                                        &proof,
                                        expected_challenge,
                                        &peer,
                                        self.swarm.local_peer_id(),
                                    ) {
                                        Ok(()) => {
                                            self.authenticated.insert(peer, (proof.hello.peer_id, connection_id));
                                            if let Err(response_error) = self.respond(
                                                channel,
                                                WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                            ) {
                                                warn!(
                                                    transport_peer = %peer,
                                                    error = %response_error,
                                                    "peer proof response channel closed; continuing network loop"
                                                );
                                            }
                                            return Ok(NetworkEvent::Authenticated {
                                                transport_peer: peer,
                                                application_peer: proof.hello.peer_id,
                                            });
                                        }
                                        Err(error) => {
                                            let _ = self.respond(
                                                channel,
                                                WireResponse::Error {
                                                    code: "PEER_AUTHENTICATION_FAILED".into(),
                                                    message: error.to_string(),
                                                },
                                            );
                                        }
                                    }
                                }
                                request if self.authenticated.get(&peer).is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id) => {
                                    return Ok(NetworkEvent::InboundRequest { transport_peer: peer, request, channel });
                                }
                                _ => {
                                    if let Err(response_error) = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: "HANDSHAKE_REQUIRED".into(),
                                            message: "complete the connection-bound application proof before other requests".into(),
                                        },
                                    ) {
                                        warn!(
                                            transport_peer = %peer,
                                            error = %response_error,
                                            "handshake-required response channel closed; continuing network loop"
                                        );
                                    }
                                }
                            }
                                }
                                request_response::Message::Response { request_id, response } => {
                                    return Ok(NetworkEvent::Response { transport_peer: peer, request_id, response });
                                }
                            }
                        }
                        request_response::Event::OutboundFailure { peer, request_id, error, .. } => {
                            let error = error.to_string();
                            self.record_issue(ConnectivityIssueKindV1::RequestFailed, Some(peer), None, error.clone());
                            return Ok(NetworkEvent::OutboundFailure { transport_peer: peer, request_id, error });
                        }
                        request_response::Event::InboundFailure { peer, error, .. } => {
                            warn!(transport_peer = %peer, %error, "inbound request failed");
                        }
                        request_response::Event::ResponseSent { .. } => {}
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::RelayClient(event)) => {
                    // Relay reservation/circuit transport events are infrastructure signals.
                    // RelayConnected is derived only from an established relayed application connection.
                    debug!(?event, "relay client event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Dcutr(event)) => {
                    self.diagnostics.start_hole_punch();
                    match &event.result {
                        Ok(_) => self.diagnostics.finish_hole_punch(Ok::<(), String>(())),
                        Err(error) => self.diagnostics.finish_hole_punch(Err(error.to_string())),
                    }
                    info!(remote_peer = %event.remote_peer_id, result = ?event.result, "DCUtR hole-punch event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::AutoNat(event)) => {
                    if let autonat::Event::StatusChanged { new, .. } = &event {
                        match new {
                            autonat::NatStatus::Public(address) => {
                                self.diagnostics.record_observed_address(address.to_string());
                                self.diagnostics.record_nat_status(NatStatusV1::Public);
                            }
                            autonat::NatStatus::Private => self.diagnostics.record_nat_status(NatStatusV1::Private),
                            autonat::NatStatus::Unknown => self.diagnostics.record_nat_status(NatStatusV1::Unknown),
                        }
                    }
                    debug!(?event, "AutoNAT event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Kad(_))
                | SwarmEvent::Behaviour(BehaviourEvent::Ping(_))
                | SwarmEvent::Behaviour(BehaviourEvent::Identify(_)) => {}
                other => debug!(event = ?other, "network event"),
            }
        }
    }

    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        let mut challenge = [0_u8; 32];
        OsRng.fill_bytes(&mut challenge);
        self.authenticated.remove(&peer);
        self.pending_challenges.insert(peer, (connection_id, challenge));
        self.swarm.behaviour_mut().request_response.send_request(&peer, WireRequest::HelloChallenge { challenge });
        Ok(())
    }

    fn refresh_connectivity_paths(&mut self) {
        let (direct_paths, relay_paths, bootstrap_paths) =
            summarize_connection_paths(self.connection_paths.values().copied());
        self.diagnostics.record_active_paths(direct_paths, relay_paths, bootstrap_paths);
    }

    fn try_relay_fallback(&mut self, peer: TransportPeerId) -> Result<bool> {
        let relay_address = {
            let Some(plan) = self.relay_fallbacks.get_mut(&peer) else {
                return Ok(false);
            };
            if plan.attempted {
                return Ok(false);
            }
            plan.attempted = true;
            plan.relay_address.clone()
        };

        if let Err(error) = self.swarm.dial(relay_address.clone()) {
            self.record_issue(
                ConnectivityIssueKindV1::RelayUnavailable,
                Some(peer),
                Some(&relay_address),
                error.to_string(),
            );
            self.diagnostics.record_no_viable_path(format!(
                "direct path failed and relay fallback could not be started for transport peer {peer}"
            ));
            self.relay_fallbacks.remove(&peer);
            return Err(error).context("failed to start relay fallback");
        }
        Ok(true)
    }

    fn record_issue(
        &mut self,
        kind: ConnectivityIssueKindV1,
        peer: Option<TransportPeerId>,
        address: Option<&Multiaddr>,
        detail: impl Into<String>,
    ) {
        self.diagnostics.record_issue(ConnectivityIssueV1 {
            kind,
            peer: peer.map(|peer| peer.to_string()),
            address: address.map(ToString::to_string),
            detail: detail.into(),
        });
    }
}

fn classify_connection_path(bootstrap_peer: bool, relay_peer: bool, relayed_endpoint: bool) -> ConnectionPathKind {
    if bootstrap_peer {
        ConnectionPathKind::BootstrapInfrastructure
    } else if relay_peer {
        ConnectionPathKind::RelayInfrastructure
    } else if relayed_endpoint {
        ConnectionPathKind::RelayedApplication
    } else {
        ConnectionPathKind::DirectApplication
    }
}

fn summarize_connection_paths(paths: impl IntoIterator<Item = ConnectionPathKind>) -> (usize, usize, usize) {
    let mut direct_paths = 0;
    let mut relay_paths = 0;
    let mut bootstrap_paths = 0;
    for path in paths {
        match path {
            ConnectionPathKind::DirectApplication => direct_paths += 1,
            ConnectionPathKind::RelayedApplication => relay_paths += 1,
            ConnectionPathKind::BootstrapInfrastructure => bootstrap_paths += 1,
            ConnectionPathKind::RelayInfrastructure => {}
        }
    }
    (direct_paths, relay_paths, bootstrap_paths)
}

fn configured_multiaddrs(name: &str) -> Result<Vec<Multiaddr>> {
    let Some(value) = env::var_os(name) else {
        return Ok(Vec::new());
    };
    let value = value.to_string_lossy();
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<Multiaddr>().with_context(|| format!("invalid {name} multiaddress: {value}")))
        .collect()
}

fn ensure_peer_suffix(mut address: Multiaddr, peer: TransportPeerId) -> Multiaddr {
    let has_peer = address.iter().any(|protocol| matches!(protocol, Protocol::P2p(value) if value == peer));
    if !has_peer {
        address.push(Protocol::P2p(peer));
    }
    address
}

fn transport_peer_from_address(address: &Multiaddr) -> Option<TransportPeerId> {
    address
        .iter()
        .filter_map(|protocol| match protocol {
            Protocol::P2p(peer) => Some(peer),
            _ => None,
        })
        .last()
}

#[cfg(test)]
mod connectivity_path_tests {
    use super::*;

    #[test]
    fn bootstrap_and_relay_infrastructure_are_isolated_from_application_paths() {
        assert_eq!(classify_connection_path(true, false, false), ConnectionPathKind::BootstrapInfrastructure);
        assert_eq!(classify_connection_path(false, true, false), ConnectionPathKind::RelayInfrastructure);
        assert_eq!(classify_connection_path(false, false, true), ConnectionPathKind::RelayedApplication);
        assert_eq!(classify_connection_path(false, false, false), ConnectionPathKind::DirectApplication);
    }

    #[test]
    fn path_summary_preserves_multiple_direct_connections() {
        let paths = [
            ConnectionPathKind::DirectApplication,
            ConnectionPathKind::DirectApplication,
            ConnectionPathKind::BootstrapInfrastructure,
            ConnectionPathKind::RelayInfrastructure,
        ];
        assert_eq!(summarize_connection_paths(paths), (2, 0, 1));
        assert_eq!(summarize_connection_paths(paths.into_iter().skip(1)), (1, 0, 1));
        assert_eq!(summarize_connection_paths(paths.into_iter().skip(2)), (0, 0, 1));
    }
}
