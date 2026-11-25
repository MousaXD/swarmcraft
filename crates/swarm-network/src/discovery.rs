use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;
use futures::StreamExt;
use libp2p::{
    connection_limits, identify,
    identity::Keypair,
    kad::{
        self,
        store::{MemoryStore, MemoryStoreConfig, RecordStore},
    },
    mdns, noise, ping,
    request_response::{self, cbor, ProtocolSupport},
    swarm::{behaviour::toggle::Toggle, dial_opts::DialOpts, ConnectionId, NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId as TransportPeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use rand_core::{OsRng, RngCore};
use std::{
    collections::{HashMap, HashSet},
    env,
    time::{Duration, Instant},
};
use swarm_protocol::{PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tracing::{debug, warn};

use crate::{
    admission::{
        application_connection_allowed, auth_challenge_expired, discovery_connection_limits, AdmissionController,
    },
    build_peer_hello_proof, verify_peer_hello, verify_peer_hello_proof, WireRequest, WireResponse, BOOTSTRAP_ENV,
};

pub const DISCOVERY_WIRE_PROTOCOL: &str = "/swarmcraft/discovery/1";
const PUBLIC_DIRECTORY_KEY: &[u8] = b"swarmcraft/discovery/public/v1";
const WORLD_KEY_PREFIX: &[u8] = b"swarmcraft/discovery/world/v1\0";
const FRIEND_KEY_PREFIX: &[u8] = b"swarmcraft/discovery/friend/v2\0";
// A peer can settle its duplicate connections after our first challenge
// arrives. Retry with a fresh nonce, inside the original ten-second
// authentication deadline and below the unauthenticated request budget.
const AUTH_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const MAX_AUTH_CHALLENGE_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy)]
struct PendingDiscoveryChallenge {
    connection: ConnectionId,
    nonce: [u8; 32],
    started: Instant,
    last_sent: Instant,
    attempts: u8,
}

impl PendingDiscoveryChallenge {
    fn retry_due(&self, now: Instant) -> bool {
        !auth_challenge_expired(self.started, now)
            && self.attempts < MAX_AUTH_CHALLENGE_ATTEMPTS
            && now.saturating_duration_since(self.last_sent) >= AUTH_RETRY_INTERVAL
    }
}

// libp2p-swarm-derive 0.36.0 emits unreachable code for an uninhabited
// connection_limits event on Rust 1.100. Only the generated behaviour lives
// under this lint scope; all actual transport/discovery logic remains subject
// to strict workspace Clippy (-D warnings).
#[allow(unreachable_code)]
mod generated_behaviour {
    use super::*;

    #[derive(NetworkBehaviour)]
    pub(super) struct DiscoveryBehaviour {
        // Derived behaviours admit connections in declaration order. The
        // limits gate must run before request-response preloads its handler;
        // otherwise a rejected duplicate leaves phantom connection state.
        pub(super) limits: connection_limits::Behaviour,
        pub(super) request_response: cbor::Behaviour<WireRequest, WireResponse>,
        pub(super) mdns: Toggle<mdns::tokio::Behaviour>,
        pub(super) identify: identify::Behaviour,
        pub(super) ping: ping::Behaviour,
        pub(super) kad: kad::Behaviour<MemoryStore>,
    }
}
use generated_behaviour::{DiscoveryBehaviour, DiscoveryBehaviourEvent};

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
    pending_challenges: HashMap<TransportPeerId, PendingDiscoveryChallenge>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    established_connections: HashMap<TransportPeerId, HashSet<ConnectionId>>,
    connection_directions: HashMap<TransportPeerId, HashMap<ConnectionId, bool>>,
    connection_counts: HashMap<TransportPeerId, usize>,
    pending_dials: HashSet<TransportPeerId>,
    admission: AdmissionController,
    locator_only: bool,
}

impl DiscoveryNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1, application_signing_key: SigningKey) -> Result<Self> {
        Self::with_mode(transport_key, local_hello, application_signing_key, false)
    }

    pub(crate) fn new_locator(
        transport_key: Keypair,
        local_hello: PeerHelloV1,
        application_signing_key: SigningKey,
    ) -> Result<Self> {
        Self::with_mode(transport_key, local_hello, application_signing_key, true)
    }

    fn with_mode(
        transport_key: Keypair,
        local_hello: PeerHelloV1,
        application_signing_key: SigningKey,
        locator_only: bool,
    ) -> Result<Self> {
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
        let mdns_enabled = if locator_only {
            false
        } else {
            match env::var("SWARMCRAFT_DISCOVERY_MDNS").as_deref() {
                Ok("0") => false,
                Ok("1") | Err(env::VarError::NotPresent) => true,
                _ => return Err(anyhow!("SWARMCRAFT_DISCOVERY_MDNS must be 0 or 1")),
            }
        };
        let mdns = if !mdns_enabled {
            None
        } else {
            Some(
                mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer)
                    .context("failed to initialize discovery mDNS")?,
            )
        };
        let identify =
            identify::Behaviour::new(identify::Config::new(DISCOVERY_WIRE_PROTOCOL.to_owned(), transport_key.public()));
        let mut kad_config = kad::Config::default();
        kad_config.set_query_timeout(Duration::from_secs(15));
        // Locator entries expire after ten minutes. Republish bounded
        // provider hints periodically even when the local signed world
        // configuration has not changed (including after locator restart).
        kad_config.set_provider_publication_interval(Some(Duration::from_secs(60)));
        let store = if locator_only {
            kad_config.set_record_filtering(kad::StoreInserts::FilterBoth);
            kad_config.set_provider_record_ttl(Some(Duration::from_secs(10 * 60)));
            MemoryStore::with_config(
                local_peer,
                MemoryStoreConfig {
                    max_records: 0,
                    max_value_bytes: 0,
                    max_providers_per_key: 20,
                    max_provided_keys: 1024,
                },
            )
        } else {
            MemoryStore::new(local_peer)
        };
        let mut kad = kad::Behaviour::with_config(local_peer, store, kad_config);
        kad.set_mode(Some(kad::Mode::Server));

        let swarm = SwarmBuilder::with_existing_identity(transport_key)
            .with_tokio()
            .with_tcp(tcp::Config::default().nodelay(true), noise::Config::new, yamux::Config::default)?
            .with_quic()
            .with_dns()?
            .with_behaviour(move |_| DiscoveryBehaviour {
                request_response,
                mdns: Toggle::from(mdns),
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
            established_connections: HashMap::new(),
            connection_directions: HashMap::new(),
            connection_counts: HashMap::new(),
            pending_dials: HashSet::new(),
            admission: AdmissionController::new(),
            locator_only,
        };
        if !locator_only {
            node.configure_from_environment()?;
        }
        Ok(node)
    }

    pub(crate) fn add_locator_external_address(&mut self, address: Multiaddr) -> Result<()> {
        if !self.locator_only {
            return Err(anyhow!("explicit locator advertisement requires locator-only service"));
        }
        self.swarm.add_external_address(address);
        Ok(())
    }

    /// Publish the address of an actual bound listener for explicit
    /// discovery providers. Only locators, never game routes or authority.
    pub fn advertise_local_listener(&mut self, address: Multiaddr) -> Result<Multiaddr> {
        crate::infrastructure::validate_endpoint(&address, false, false)?;
        if !self.swarm.listeners().any(|listener| listener == &address) {
            return Err(anyhow!("discovery address is not an active local listener"));
        }
        let address = address.with(libp2p::multiaddr::Protocol::P2p(self.local_transport_peer_id()));
        self.swarm.add_external_address(address.clone());
        Ok(address)
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

    pub fn is_connected(&self, transport_peer: &TransportPeerId) -> bool {
        self.established_connections.get(transport_peer).is_some_and(|connections| !connections.is_empty())
    }

    pub fn is_authenticated(&self, transport_peer: &TransportPeerId) -> bool {
        self.authenticated.get(transport_peer).is_some_and(|(_, connection_id)| {
            self.established_connections
                .get(transport_peer)
                .is_some_and(|connections| connections.contains(connection_id))
        })
    }

    pub fn established_connection_count(&self, transport_peer: &TransportPeerId) -> usize {
        self.established_connections.get(transport_peer).map_or(0, HashSet::len)
    }

    /// Actively closes the current peer connection. Useful to test recovery
    /// when a still-listening authenticated provider loses its QUIC stream.
    /// This does not change membership, authorization or provider records.
    pub fn disconnect_peer(&mut self, transport_peer: TransportPeerId) -> Result<()> {
        self.swarm.disconnect_peer_id(transport_peer).map_err(|_| anyhow!("discovery peer is not connected"))
    }

    pub fn add_peer_address(&mut self, peer: TransportPeerId, address: Multiaddr) {
        self.swarm.behaviour_mut().kad.add_address(&peer, address);
    }

    pub fn add_bootstrap_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let peer = transport_peer_from_address(&address)
            .ok_or_else(|| anyhow!("bootstrap address must contain /p2p/<peer-id>"))?;
        self.add_peer_address(peer, address.clone());
        if !self.swarm.is_connected(&peer) && self.pending_dials.insert(peer) {
            if let Err(error) = self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build()) {
                self.pending_dials.remove(&peer);
                return Err(error).context("failed to dial discovery bootstrap peer");
            }
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
        if self.swarm.is_connected(&peer) {
            debug!(transport_peer = %peer, "discovery dial suppressed; live connection already exists");
            return Ok(());
        }
        if !self.pending_dials.insert(peer) {
            debug!(transport_peer = %peer, "discovery dial suppressed; outbound dial already pending");
            return Ok(());
        }
        debug!(transport_peer = %peer, "discovery outbound provider dial attempt");
        if let Err(error) = self.swarm.dial(DialOpts::peer_id(peer).build()) {
            self.pending_dials.remove(&peer);
            return Err(error).context("failed to dial discovery provider");
        }
        Ok(())
    }

    pub fn send_request(
        &mut self,
        peer: &TransportPeerId,
        request: WireRequest,
    ) -> Result<request_response::OutboundRequestId> {
        request.validate_limits()?;
        let request_id = self.swarm.behaviour_mut().request_response.send_request(peer, request);
        debug!(
            transport_peer = %peer,
            active_connection = ?self.active_connections.get(peer),
            ?request_id,
            "discovery outbound request queued"
        );
        Ok(request_id)
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
            self.drive_auth_challenges();
            let event = match tokio::time::timeout(AUTH_RETRY_INTERVAL, self.swarm.select_next_some()).await {
                Ok(event) => event,
                Err(_) => continue,
            };
            self.drive_auth_challenges();
            match event {
                SwarmEvent::NewListenAddr { address, .. } => return Ok(DiscoveryNetworkEvent::Listening { address }),
                SwarmEvent::ListenerError { .. } | SwarmEvent::ListenerClosed { .. } if self.locator_only => {
                    return Err(anyhow!("locator listener stopped"));
                }
                SwarmEvent::ConnectionEstablished { peer_id, connection_id, endpoint, num_established, .. } => {
                    self.pending_dials.remove(&peer_id);
                    let connection_is_dialer = endpoint.is_dialer();
                    let prefer_dialer = *self.swarm.local_peer_id() < peer_id;
                    self.connection_directions.entry(peer_id).or_default().insert(connection_id, connection_is_dialer);
                    let established_count = {
                        let established = self.established_connections.entry(peer_id).or_default();
                        established.insert(connection_id);
                        established.len()
                    };
                    self.connection_counts.insert(peer_id, established_count);
                    debug_assert_eq!(established_count, num_established.get() as usize);
                    debug!(
                        transport_peer = %peer_id,
                        %connection_id,
                        connection_is_dialer,
                        prefer_dialer,
                        tracked_established = established_count,
                        reported_established = num_established.get(),
                        active_connection = ?self.active_connections.get(&peer_id),
                        "discovery connection established"
                    );

                    if !application_connection_allowed(
                        self.active_connections.len(),
                        self.active_connections.contains_key(&peer_id),
                    ) {
                        warn!(transport_peer = %peer_id, %connection_id, "discovery connection admission limit reached");
                        let _ = self.swarm.close_connection(connection_id);
                        continue;
                    }

                    if let Some(active) = self.active_connections.get(&peer_id).copied() {
                        let active_is_live = self
                            .established_connections
                            .get(&peer_id)
                            .is_some_and(|connections| connections.contains(&active));
                        if active != connection_id && active_is_live {
                            let active_is_dialer = self
                                .connection_directions
                                .get(&peer_id)
                                .and_then(|connections| connections.get(&active))
                                .copied()
                                .unwrap_or(false);
                            let active_is_preferred = active_is_dialer == prefer_dialer;
                            let newcomer_is_preferred = connection_is_dialer == prefer_dialer;

                            if newcomer_is_preferred && !active_is_preferred {
                                debug!(
                                    transport_peer = %peer_id,
                                    old_connection = %active,
                                    new_connection = %connection_id,
                                    old_is_dialer = active_is_dialer,
                                    new_is_dialer = connection_is_dialer,
                                    "switching discovery peer to deterministic canonical connection"
                                );
                                if self
                                    .authenticated
                                    .get(&peer_id)
                                    .is_some_and(|(_, authenticated_connection)| *authenticated_connection == active)
                                {
                                    self.authenticated.remove(&peer_id);
                                }
                                if self
                                    .pending_challenges
                                    .get(&peer_id)
                                    .is_some_and(|challenge| challenge.connection == active)
                                {
                                    self.pending_challenges.remove(&peer_id);
                                }
                                self.active_connections.insert(peer_id, connection_id);
                                // Defer the new application challenge until the old physical
                                // connection has closed and request-response sees one connection.
                                let _ = self.swarm.close_connection(active);
                            } else {
                                debug!(
                                    transport_peer = %peer_id,
                                    active_connection = %active,
                                    duplicate_connection = %connection_id,
                                    active_is_dialer,
                                    duplicate_is_dialer = connection_is_dialer,
                                    "closing noncanonical duplicate discovery connection"
                                );
                                let _ = self.swarm.close_connection(connection_id);
                            }
                            continue;
                        }
                    }

                    self.active_connections.insert(peer_id, connection_id);
                    if established_count == 1 {
                        self.ensure_auth_challenge(peer_id, connection_id)?;
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    let established_count = {
                        let established = self.established_connections.entry(peer_id).or_default();
                        established.remove(&connection_id);
                        established.len()
                    };
                    if let Some(directions) = self.connection_directions.get_mut(&peer_id) {
                        directions.remove(&connection_id);
                        if directions.is_empty() {
                            self.connection_directions.remove(&peer_id);
                        }
                    }
                    self.connection_counts.insert(peer_id, established_count);
                    debug_assert_eq!(established_count, num_established as usize);
                    debug!(
                        transport_peer = %peer_id,
                        %connection_id,
                        tracked_remaining = established_count,
                        remaining_established = num_established,
                        active_connection = ?self.active_connections.get(&peer_id),
                        another_connection_live = established_count > 0,
                        "discovery connection closed"
                    );

                    let closing_application_peer = self
                        .authenticated
                        .get(&peer_id)
                        .filter(|(_, authenticated_connection)| *authenticated_connection == connection_id)
                        .map(|(peer, _)| *peer);
                    if closing_application_peer.is_some() {
                        self.authenticated.remove(&peer_id);
                    }
                    if self
                        .pending_challenges
                        .get(&peer_id)
                        .is_some_and(|challenge| challenge.connection == connection_id)
                    {
                        self.pending_challenges.remove(&peer_id);
                    }

                    if established_count == 0 {
                        self.established_connections.remove(&peer_id);
                        self.connection_directions.remove(&peer_id);
                        self.active_connections.remove(&peer_id);
                        self.connection_counts.remove(&peer_id);
                        self.pending_dials.remove(&peer_id);
                        let application_peer = self.authenticated.remove(&peer_id).map(|(peer, _)| peer);
                        self.pending_challenges.remove(&peer_id);
                        self.admission.forget_peer(peer_id);
                        return Ok(DiscoveryNetworkEvent::Disconnected {
                            transport_peer: peer_id,
                            application_peer: application_peer.or(closing_application_peer),
                        });
                    }

                    let active_is_live = self.active_connections.get(&peer_id).is_some_and(|active| {
                        self.established_connections
                            .get(&peer_id)
                            .is_some_and(|connections| connections.contains(active))
                    });
                    if !active_is_live {
                        let prefer_dialer = *self.swarm.local_peer_id() < peer_id;
                        let survivor = self
                            .established_connections
                            .get(&peer_id)
                            .and_then(|connections| {
                                connections
                                    .iter()
                                    .copied()
                                    .find(|candidate| {
                                        self.connection_directions
                                            .get(&peer_id)
                                            .and_then(|directions| directions.get(candidate))
                                            .is_some_and(|is_dialer| *is_dialer == prefer_dialer)
                                    })
                                    .or_else(|| connections.iter().copied().next())
                            })
                            .expect("nonempty established discovery connection set");
                        self.active_connections.insert(peer_id, survivor);
                    }

                    // The request-response behaviour must see exactly one physical
                    // connection before connection-bound application auth resumes.
                    if established_count == 1 {
                        if let Some(active) = self.active_connections.get(&peer_id).copied() {
                            self.ensure_auth_challenge(peer_id, active)?;
                        }
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                    for (peer, address) in peers {
                        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
                        if !self.swarm.is_connected(&peer)
                            && self.pending_dials.insert(peer)
                            && self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build()).is_err()
                        {
                            self.pending_dials.remove(&peer);
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
                                    if let Err(error) = self.respond(channel, WireResponse::HelloChallengeAccepted) {
                                        debug!(
                                            transport_peer = %peer,
                                            %connection_id,
                                            %error,
                                            "discovery auth challenge acknowledgement channel closed; isolating peer request"
                                        );
                                    }
                                    self.swarm
                                        .behaviour_mut()
                                        .request_response
                                        .send_request(&peer, WireRequest::HelloProof(Box::new(proof)));
                                }
                                WireRequest::HelloProof(proof) => {
                                    let Some(challenge) = self.pending_challenges.get(&peer).copied() else {
                                        let _ = self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "PEER_AUTHENTICATION_FAILED".into(),
                                                message: "no live receiver challenge exists for this proof".into(),
                                            },
                                        );
                                        continue;
                                    };
                                    if challenge.connection != connection_id
                                        || proof.challenge != challenge.nonce
                                        || auth_challenge_expired(challenge.started, Instant::now())
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
                                        challenge.nonce,
                                        &peer,
                                        self.swarm.local_peer_id(),
                                    ) {
                                        Ok(()) => {
                                            self.pending_challenges.remove(&peer);
                                            self.authenticated.insert(peer, (proof.hello.peer_id, connection_id));
                                            if let Err(error) = self.respond(
                                                channel,
                                                WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                            ) {
                                                debug!(
                                                    transport_peer = %peer,
                                                    %connection_id,
                                                    %error,
                                                    "discovery auth acceptance channel closed after proof verification; keeping failure peer-local"
                                                );
                                            }
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
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Kad(kad::Event::InboundRequest {
                    request: kad::InboundRequest::AddProvider { record: Some(record) },
                })) if self.locator_only => {
                    // DHT provider claims are untrusted locator hints only.
                    // Actual signed announcement and quorum are checked by clients.
                    if locator_provider_key(record.key.as_ref())
                        && record.addresses.len() <= 8
                        && record.addresses.iter().all(|address| address.to_vec().len() <= 512)
                    {
                        let _ = self.swarm.behaviour_mut().kad.store_mut().add_provider(record);
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Ping(_))
                | SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Identify(_))
                | SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Kad(_)) => {}
                SwarmEvent::Dialing { peer_id: Some(peer_id), .. } => {
                    self.pending_dials.insert(peer_id);
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    if let Some(peer_id) = peer_id {
                        self.pending_dials.remove(&peer_id);
                    }
                    debug!(transport_peer = ?peer_id, %error, "discovery outgoing connection failed");
                }
                other => debug!(event = ?other, "discovery network event"),
            }
        }
    }
    fn ensure_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        if self
            .authenticated
            .get(&peer)
            .is_some_and(|(_, authenticated_connection)| *authenticated_connection == connection_id)
            || self.pending_challenges.get(&peer).is_some_and(|challenge| challenge.connection == connection_id)
        {
            return Ok(());
        }
        self.issue_auth_challenge(peer, connection_id)
    }

    fn issue_auth_challenge(&mut self, peer: TransportPeerId, connection_id: ConnectionId) -> Result<()> {
        let mut challenge = [0_u8; 32];
        OsRng.fill_bytes(&mut challenge);
        self.authenticated.remove(&peer);
        let now = Instant::now();
        self.pending_challenges.insert(
            peer,
            PendingDiscoveryChallenge {
                connection: connection_id,
                nonce: challenge,
                started: now,
                last_sent: now,
                attempts: 1,
            },
        );
        self.swarm.behaviour_mut().request_response.send_request(&peer, WireRequest::HelloChallenge { challenge });
        Ok(())
    }

    fn drive_auth_challenges(&mut self) {
        let now = Instant::now();
        let stale = self
            .pending_challenges
            .iter()
            .filter_map(|(peer, challenge)| {
                auth_challenge_expired(challenge.started, now).then_some((*peer, challenge.connection))
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
        for (peer, challenge) in &mut self.pending_challenges {
            if !challenge.retry_due(now)
                || self.active_connections.get(peer) != Some(&challenge.connection)
                || self.connection_counts.get(peer).copied() != Some(1)
            {
                continue;
            }
            // Replaced nonces and wrong-connection proofs never consume the
            // current challenge. All retries stay inside its original deadline.
            OsRng.fill_bytes(&mut challenge.nonce);
            challenge.attempts += 1;
            challenge.last_sent = now;
            self.swarm
                .behaviour_mut()
                .request_response
                .send_request(peer, WireRequest::HelloChallenge { challenge: challenge.nonce });
        }
    }
}

fn locator_provider_key(key: &[u8]) -> bool {
    key == PUBLIC_DIRECTORY_KEY
        || (key.starts_with(WORLD_KEY_PREFIX) && key.len() == WORLD_KEY_PREFIX.len() + 32)
        || (key.starts_with(FRIEND_KEY_PREFIX) && key.len() == FRIEND_KEY_PREFIX.len() + 64)
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
    use ed25519_dalek::{Signer, SigningKey};
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

    #[test]
    fn discovery_auth_retries_are_bounded_by_original_deadline() {
        let started = Instant::now();
        let mut challenge = PendingDiscoveryChallenge {
            connection: ConnectionId::new_unchecked(7),
            nonce: [1; 32],
            started,
            last_sent: started,
            attempts: 1,
        };
        assert!(!challenge.retry_due(started));
        assert!(challenge.retry_due(started + AUTH_RETRY_INTERVAL));
        challenge.last_sent = started + AUTH_RETRY_INTERVAL;
        challenge.attempts += 1;
        assert!(!challenge.retry_due(challenge.last_sent));
        assert!(challenge.retry_due(started + AUTH_RETRY_INTERVAL * 2));
        challenge.attempts = MAX_AUTH_CHALLENGE_ATTEMPTS;
        assert!(!challenge.retry_due(started + AUTH_RETRY_INTERVAL * 3));
        challenge.attempts = 1;
        assert!(!challenge.retry_due(started + crate::admission::AUTH_CHALLENGE_TIMEOUT));
    }

    #[tokio::test]
    async fn denied_connection_does_not_preload_request_response_state() {
        let key = SigningKey::generate(&mut OsRng);
        let public_key = key.verifying_key().to_bytes();
        let mut hello = PeerHelloV1 {
            peer_id: peer_id_from_public_key(&public_key),
            public_key,
            protocol_versions: vec![PROTOCOL_VERSION],
            capabilities: vec!["discovery-v1".into()],
            nonce: [0; 32],
            signature: Vec::new(),
        };
        hello.signature = key.sign(&hello.signing_bytes().unwrap()).to_bytes().to_vec();
        let mut node = DiscoveryNode::new(Keypair::generate_ed25519(), hello, key).unwrap();
        let behaviour = node.swarm.behaviour_mut();
        // Force admission denial deterministically, without needing to race
        // several real duplicate connections against the QUIC event loop.
        behaviour.limits = connection_limits::Behaviour::new(
            connection_limits::ConnectionLimits::default().with_max_established_per_peer(Some(0)),
        );
        let remote = Keypair::generate_ed25519().public().to_peer_id();
        let address: Multiaddr = "/ip4/127.0.0.1/udp/40000/quic-v1".parse().unwrap();
        assert!(behaviour
            .handle_established_inbound_connection(ConnectionId::new_unchecked(1), remote, &address, &address)
            .is_err());
        assert!(!behaviour.request_response.is_connected(&remote));
    }
}
