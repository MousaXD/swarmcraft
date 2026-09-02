use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;
use futures::StreamExt;
use libp2p::{
    connection_limits, identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    mdns, noise, ping,
    request_response::{self, cbor, ProtocolSupport},
    swarm::{dial_opts::DialOpts, ConnectionId, NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId as TransportPeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use rand_core::{OsRng, RngCore};
use std::{
    collections::HashMap,
    env,
    time::{Duration, Instant},
};
use swarm_protocol::{PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tracing::{debug, warn};

use crate::{
    admission::{
        application_connection_allowed, auth_challenge_expired, discovery_connection_limits, AdmissionController,
        AUTH_CHALLENGE_TIMEOUT,
    },
    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, WireRequest, WireResponse, BOOTSTRAP_ENV,
};

pub const DISCOVERY_WIRE_PROTOCOL: &str = "/swarmcraft/discovery/1";
const PUBLIC_DIRECTORY_KEY: &[u8] = b"swarmcraft/discovery/public/v1";
const WORLD_KEY_PREFIX: &[u8] = b"swarmcraft/discovery/world/v1\0";
const FRIEND_KEY_PREFIX: &[u8] = b"swarmcraft/discovery/friend/v2\0";

#[derive(NetworkBehaviour)]
struct DiscoveryBehaviour {
    request_response: cbor::Behaviour<WireRequest, WireResponse>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    kad: kad::Behaviour<MemoryStore>,
    limits: connection_limits::Behaviour,
}

#[derive(Debug)]
pub enum DiscoveryNetworkEvent {
    Listening {
        address: Multiaddr,
    },
    Authenticated {
        transport_peer: TransportPeerId,
        application_peer: PeerId,
    },
    Disconnected {
        transport_peer: TransportPeerId,
        application_peer: Option<PeerId>,
    },
    InboundRequest {
        transport_peer: TransportPeerId,
        application_peer: PeerId,
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
    ProvidersFound {
        query_id: kad::QueryId,
        providers: Vec<TransportPeerId>,
    },
    ProvidersFinished {
        query_id: kad::QueryId,
    },
    ProvidersFailed {
        query_id: kad::QueryId,
        error: String,
    },
    ProviderPublished {
        query_id: kad::QueryId,
    },
    ProviderPublishFailed {
        query_id: kad::QueryId,
        error: String,
    },
}

pub struct DiscoveryNode {
    swarm: Swarm<DiscoveryBehaviour>,
    local_hello: PeerHelloV1,
    application_signing_key: SigningKey,
    authenticated: HashMap<TransportPeerId, (PeerId, ConnectionId)>,
    pending_challenges: HashMap<TransportPeerId, (ConnectionId, [u8; 32], Instant)>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    connection_counts: HashMap<TransportPeerId, usize>,
    admission: AdmissionController,
}

impl DiscoveryNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1, application_signing_key: SigningKey) -> Result<Self> {
        verify_peer_hello(&local_hello).context("local discovery PeerHello must be valid")?;
        let local_peer = transport_key.public().to_peer_id();
        let self_test =
            build_peer_hello_proof(&local_hello, &application_signing_key, [0; 32], &local_peer, &local_peer)?;
        verify_peer_hello_proof(&self_test, [0; 32], &local_peer, &local_peer)
            .context("application signing key must match the local discovery PeerHello")?;
        let request_response = cbor::Behaviour::new(
            [(StreamProtocol::new(DISCOVERY_WIRE_PROTOCOL), ProtocolSupport::Full)],
            request_response::Config::default()
                .with_request_timeout(Duration::from_secs(15))
                .with_max_concurrent_streams(64),
        );
        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer)
            .context("failed to initialize discovery mDNS")?;
        let identify =
            identify::Behaviour::new(identify::Config::new(DISCOVERY_WIRE_PROTOCOL.to_owned(), transport_key.public()));
        let mut kad_config = kad::Config::default();
        kad_config.set_query_timeout(Duration::from_secs(15));
        let mut kad = kad::Behaviour::with_config(local_peer, MemoryStore::new(local_peer), kad_config);
        kad.set_mode(Some(kad::Mode::Server));

        let swarm = SwarmBuilder::with_existing_identity(transport_key)
            .with_tokio()
            .with_tcp(tcp::Config::default().nodelay(true), noise::Config::new, yamux::Config::default)?
            .with_quic()
            .with_dns()?
            .with_behaviour(move |_| DiscoveryBehaviour {
                request_response,
                mdns,
                identify,
                ping: ping::Behaviour::default(),
                kad,
                limits: connection_limits::Behaviour::new(discovery_connection_limits()),
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
            admission: AdmissionController::new(),
        };
        node.configure_from_environment()?;
        Ok(node)
    }

    pub fn local_transport_peer_id(&self) -> TransportPeerId {
        *self.swarm.local_peer_id()
    }

    pub fn listen(&mut self, address: Multiaddr) -> Result<()> {
        self.swarm.listen_on(address).map(|_| ()).context("failed to listen for discovery")
    }

    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).map(|(peer, _)| *peer)
    }

    pub fn add_peer_address(&mut self, peer: TransportPeerId, address: Multiaddr) {
        self.swarm.behaviour_mut().kad.add_address(&peer, address);
    }

    pub fn add_bootstrap_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let peer = transport_peer_from_address(&address)
            .ok_or_else(|| anyhow!("bootstrap address must contain /p2p/<peer-id>"))?;
        self.add_peer_address(peer, address.clone());
        if !self.swarm.is_connected(&peer) {
            self.swarm
                .dial(DialOpts::peer_id(peer).addresses(vec![address]).build())
                .context("failed to dial discovery bootstrap peer")?;
        }
        Ok(peer)
    }

    pub fn bootstrap(&mut self) -> Result<()> {
        self.swarm.behaviour_mut().kad.bootstrap().map(|_| ()).context("discovery Kademlia bootstrap failed")
    }

    pub fn configure_from_environment(&mut self) -> Result<()> {
        let addresses = configured_multiaddrs(BOOTSTRAP_ENV)?;
        for address in &addresses {
            self.add_bootstrap_address(address.clone())?;
        }
        if !addresses.is_empty() {
            self.bootstrap()?;
        }
        Ok(())
    }

    pub fn dial_peer(&mut self, peer: TransportPeerId) -> Result<()> {
        if !self.swarm.is_connected(&peer) {
            self.swarm.dial(DialOpts::peer_id(peer).build()).context("failed to dial discovery provider")?;
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
        response.validate_limits()?;
        self.swarm
            .behaviour_mut()
            .request_response
            .send_response(channel, response)
            .map_err(|_| anyhow!("discovery response channel closed"))
    }

    pub fn start_providing_public_directory(&mut self) -> Result<kad::QueryId> {
        self.swarm
            .behaviour_mut()
            .kad
            .start_providing(public_directory_key())
            .context("failed to publish public discovery provider")
    }

    pub fn stop_providing_public_directory(&mut self) {
        self.swarm.behaviour_mut().kad.stop_providing(&public_directory_key());
    }

    pub fn start_providing_world(&mut self, world: WorldId) -> Result<kad::QueryId> {
        self.swarm
            .behaviour_mut()
            .kad
            .start_providing(world_discovery_key(world))
            .context("failed to publish world discovery provider")
    }

    pub fn stop_providing_world(&mut self, world: WorldId) {
        self.swarm.behaviour_mut().kad.stop_providing(&world_discovery_key(world));
    }

    pub fn start_providing_friend_presence(&mut self, peer: PeerId, requester: PeerId) -> Result<kad::QueryId> {
        self.swarm
            .behaviour_mut()
            .kad
            .start_providing(friend_presence_key(peer, requester))
            .context("failed to publish friend presence provider")
    }

    pub fn stop_providing_friend_presence(&mut self, peer: PeerId, requester: PeerId) {
        self.swarm.behaviour_mut().kad.stop_providing(&friend_presence_key(peer, requester));
    }

    pub fn find_public_providers(&mut self) -> kad::QueryId {
        self.swarm.behaviour_mut().kad.get_providers(public_directory_key())
    }

    pub fn find_world_providers(&mut self, world: WorldId) -> kad::QueryId {
        self.swarm.behaviour_mut().kad.get_providers(world_discovery_key(world))
    }

    pub fn find_friend_providers(&mut self, peer: PeerId, requester: PeerId) -> kad::QueryId {
        self.swarm.behaviour_mut().kad.get_providers(friend_presence_key(peer, requester))
    }

    pub async fn next_event(&mut self) -> Result<DiscoveryNetworkEvent> {
        loop {
            self.expire_stale_auth_challenges();
            let event = match tokio::time::timeout(AUTH_CHALLENGE_TIMEOUT, self.swarm.select_next_some()).await {
                Ok(event) => event,
                Err(_) => continue,
            };
            self.expire_stale_auth_challenges();
            match event {
                SwarmEvent::NewListenAddr { address, .. } => return Ok(DiscoveryNetworkEvent::Listening { address }),
                SwarmEvent::ConnectionEstablished { peer_id, connection_id, num_established, .. } => {
                    if !application_connection_allowed(
                        self.active_connections.len(),
                        self.active_connections.contains_key(&peer_id),
                    ) {
                        warn!(transport_peer = %peer_id, %connection_id, "discovery connection admission limit reached");
                        let _ = self.swarm.close_connection(connection_id);
                        continue;
                    }
                    self.connection_counts.insert(peer_id, num_established.get() as usize);
                    self.authenticated.remove(&peer_id);
                    self.pending_challenges.remove(&peer_id);
                    let previous = self.active_connections.insert(peer_id, connection_id);
                    let defer_challenge = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_challenge {
                        self.issue_auth_challenge(peer_id, connection_id)?;
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
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
                        .is_some_and(|(challenge_connection, _, _)| *challenge_connection == connection_id)
                    {
                        self.pending_challenges.remove(&peer_id);
                    }
                    if num_established == 0 {
                        self.active_connections.remove(&peer_id);
                        self.connection_counts.remove(&peer_id);
                        let application_peer = self.authenticated.remove(&peer_id).map(|(peer, _)| peer);
                        self.pending_challenges.remove(&peer_id);
                        self.admission.forget_peer(peer_id);
                        return Ok(DiscoveryNetworkEvent::Disconnected { transport_peer: peer_id, application_peer });
                    }
                    if let Some(active) =
                        self.active_connections.get(&peer_id).copied().filter(|active| *active != connection_id)
                    {
                        self.issue_auth_challenge(peer_id, active)?;
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                    for (peer, address) in peers {
                        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
                        if !self.swarm.is_connected(&peer) {
                            let _ = self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build());
                        }
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Mdns(mdns::Event::Expired(peers))) => {
                    for (peer, address) in peers {
                        self.swarm.behaviour_mut().kad.remove_address(&peer, &address);
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Identify(identify::Event::Received {
                    peer_id,
                    info,
                    ..
                })) => {
                    for address in info.listen_addrs {
                        self.swarm.behaviour_mut().kad.add_address(&peer_id, address);
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::RequestResponse(event)) => match event {
                    request_response::Event::Message { peer, connection_id, message } => match message {
                        request_response::Message::Request { request, channel, .. } => {
                            let authenticated_request =
                                self.authenticated.get(&peer).is_some_and(|(_, authenticated_connection)| {
                                    *authenticated_connection == connection_id
                                });
                            if !self.admission.admit_request(peer, authenticated_request, Instant::now()) {
                                let _ = self.respond(
                                    channel,
                                    WireResponse::Error {
                                        code: "RATE_LIMITED".into(),
                                        message: if authenticated_request {
                                            "authenticated discovery request budget exceeded".into()
                                        } else {
                                            "pre-authentication discovery request budget exceeded".into()
                                        },
                                    },
                                );
                                continue;
                            }
                            if let Err(error) = request.validate_limits() {
                                let _ = self.respond(
                                    channel,
                                    WireResponse::Error {
                                        code: "REQUEST_LIMIT_EXCEEDED".into(),
                                        message: error.to_string(),
                                    },
                                );
                                continue;
                            }
                            match request {
                                WireRequest::Hello(_) => {
                                    let _ = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: "CONNECTION_PROOF_REQUIRED".into(),
                                            message: "reusable PeerHello is not an authentication proof".into(),
                                        },
                                    );
                                }
                                WireRequest::HelloChallenge { challenge } => {
                                    let canonical = self
                                        .active_connections
                                        .get(&peer)
                                        .is_some_and(|active| *active == connection_id)
                                        && self.connection_counts.get(&peer).copied() == Some(1);
                                    if !canonical {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "AUTH_CONNECTION_RETRY".into(),
                                                message: "authentication challenge arrived on a superseded connection"
                                                    .into(),
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
                                    let Some((challenge_connection, expected_challenge, _issued_at)) =
                                        self.pending_challenges.remove(&peer)
                                    else {
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
                                        || self
                                            .active_connections
                                            .get(&peer)
                                            .is_none_or(|active| *active != connection_id)
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
                                            self.respond(
                                                channel,
                                                WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                            )?;
                                            return Ok(DiscoveryNetworkEvent::Authenticated {
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
                                request => {
                                    if let Some((application_peer, _)) = self.authenticated.get(&peer).copied().filter(
                                        |(_, authenticated_connection)| *authenticated_connection == connection_id,
                                    ) {
                                        return Ok(DiscoveryNetworkEvent::InboundRequest {
                                            transport_peer: peer,
                                            application_peer,
                                            request,
                                            channel,
                                        });
                                    }
                                    let _ = self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: "HANDSHAKE_REQUIRED".into(),
                                            message: "complete the connection-bound application proof before discovery requests".into(),
                                        },
                                    );
                                }
                            }
                        }
                        request_response::Message::Response { request_id, response } => {
                            if let Err(error) = response.validate_limits() {
                                return Ok(DiscoveryNetworkEvent::OutboundFailure {
                                    transport_peer: peer,
                                    request_id,
                                    error: error.to_string(),
                                });
                            }
                            return Ok(DiscoveryNetworkEvent::Response { transport_peer: peer, request_id, response });
                        }
                    },
                    request_response::Event::OutboundFailure { peer, request_id, error, .. } => {
                        return Ok(DiscoveryNetworkEvent::OutboundFailure {
                            transport_peer: peer,
                            request_id,
                            error: error.to_string(),
                        });
                    }
                    request_response::Event::InboundFailure { peer, error, .. } => {
                        warn!(transport_peer = %peer, %error, "discovery inbound request failed");
                    }
                    request_response::Event::ResponseSent { .. } => {}
                },
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Kad(kad::Event::OutboundQueryProgressed {
                    id,
                    result,
                    ..
                })) => match result {
                    kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FoundProviders { providers, .. })) => {
                        return Ok(DiscoveryNetworkEvent::ProvidersFound {
                            query_id: id,
                            providers: providers.into_iter().collect(),
                        });
                    }
                    kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FinishedWithNoAdditionalRecord {
                        ..
                    })) => {
                        return Ok(DiscoveryNetworkEvent::ProvidersFinished { query_id: id });
                    }
                    kad::QueryResult::GetProviders(Err(error)) => {
                        return Ok(DiscoveryNetworkEvent::ProvidersFailed { query_id: id, error: error.to_string() });
                    }
                    kad::QueryResult::StartProviding(Ok(_)) => {
                        return Ok(DiscoveryNetworkEvent::ProviderPublished { query_id: id });
                    }
                    kad::QueryResult::StartProviding(Err(error)) => {
                        return Ok(DiscoveryNetworkEvent::ProviderPublishFailed {
                            query_id: id,
                            error: error.to_string(),
                        });
                    }
                    _ => {}
                },
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Ping(_))
                | SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Identify(_))
                | SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Kad(_)) => {}
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    debug!(transport_peer = ?peer_id, %error, "discovery outgoing connection failed");
                }
                other => debug!(event = ?other, "discovery network event"),
            }
        }
    }
    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        let mut challenge = [0_u8; 32];
        OsRng.fill_bytes(&mut challenge);
        self.authenticated.remove(&peer);
        self.pending_challenges.insert(peer, (connection_id, challenge, Instant::now()));
        self.swarm.behaviour_mut().request_response.send_request(&peer, WireRequest::HelloChallenge { challenge });
        Ok(())
    }

    fn expire_stale_auth_challenges(&mut self) {
        let now = Instant::now();
        let stale = self
            .pending_challenges
            .iter()
            .filter_map(|(peer, (connection_id, _, issued_at))| {
                auth_challenge_expired(*issued_at, now).then_some((*peer, *connection_id))
            })
            .collect::<Vec<_>>();
        for (peer, connection_id) in stale {
            self.pending_challenges.remove(&peer);
            self.authenticated.remove(&peer);
            if self.active_connections.get(&peer).is_some_and(|active| *active == connection_id) {
                self.active_connections.remove(&peer);
            }
            let _ = self.swarm.close_connection(connection_id);
            warn!(transport_peer = %peer, %connection_id, "discovery authentication challenge expired; closing silent connection");
        }
    }
}

pub fn public_directory_key() -> kad::RecordKey {
    kad::RecordKey::new(&PUBLIC_DIRECTORY_KEY)
}

pub fn world_discovery_key(world: WorldId) -> kad::RecordKey {
    let mut bytes = Vec::with_capacity(WORLD_KEY_PREFIX.len() + 32);
    bytes.extend_from_slice(WORLD_KEY_PREFIX);
    bytes.extend_from_slice(&world.0);
    kad::RecordKey::new(&bytes)
}

pub fn friend_presence_key(peer: PeerId, requester: PeerId) -> kad::RecordKey {
    let mut bytes = Vec::with_capacity(FRIEND_KEY_PREFIX.len() + 64);
    bytes.extend_from_slice(FRIEND_KEY_PREFIX);
    bytes.extend_from_slice(&peer.0);
    bytes.extend_from_slice(&requester.0);
    kad::RecordKey::new(&bytes)
}

fn configured_multiaddrs(name: &str) -> Result<Vec<Multiaddr>> {
    let Some(value) = env::var_os(name) else {
        return Ok(Vec::new());
    };
    value
        .to_string_lossy()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<Multiaddr>().with_context(|| format!("invalid {name} multiaddress: {value}")))
        .collect()
}

fn transport_peer_from_address(address: &Multiaddr) -> Option<TransportPeerId> {
    address
        .iter()
        .filter_map(|protocol| match protocol {
            libp2p::multiaddr::Protocol::P2p(peer) => Some(peer),
            _ => None,
        })
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;
    use swarm_protocol::{peer_id_from_public_key, PeerHelloV1};

    #[test]
    fn discovery_keys_separate_public_world_and_friend_namespaces() {
        let world_a = world_discovery_key(WorldId([1; 32]));
        let world_b = world_discovery_key(WorldId([2; 32]));
        let friend = friend_presence_key(PeerId([1; 32]), PeerId([3; 32]));
        let other_requester = friend_presence_key(PeerId([1; 32]), PeerId([4; 32]));
        assert_ne!(world_a, world_b);
        assert_ne!(world_a, friend);
        assert_ne!(friend, other_requester);
        assert_ne!(world_a, public_directory_key());
    }

    #[test]
    fn node_rejects_invalid_local_application_identity() {
        let public_key = [7; 32];
        let hello = PeerHelloV1 {
            peer_id: peer_id_from_public_key(&public_key),
            public_key,
            protocol_versions: vec![PROTOCOL_VERSION],
            capabilities: vec!["discovery-v1".into()],
            nonce: [0; 32],
            signature: vec![0; 64],
        };
        assert!(DiscoveryNode::new(Keypair::generate_ed25519(), hello, SigningKey::generate(&mut OsRng)).is_err());
    }
}
