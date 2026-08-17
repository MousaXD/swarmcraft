use crate::{verify_peer_hello, wire::WireRequest, wire::WireResponse, ConnectivityDiagnosticsV1, NatStatusV1};
use anyhow::{anyhow, Context, Result};
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
use std::{collections::HashMap, env, time::Duration};
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

pub struct SwarmNode {
    swarm: Swarm<Behaviour>,
    local_hello: PeerHelloV1,
    authenticated: HashMap<TransportPeerId, PeerId>,
    active_connections: HashMap<TransportPeerId, ConnectionId>,
    diagnostics: ConnectivityDiagnosticsV1,
}

impl SwarmNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1) -> Result<Self> {
        verify_peer_hello(&local_hello).context("local peer hello must be valid before networking starts")?;

        let local_peer = transport_key.public().to_peer_id();
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
            authenticated: HashMap::new(),
            active_connections: HashMap::new(),
            diagnostics: ConnectivityDiagnosticsV1::default(),
        };
        node.configure_from_environment()?;
        Ok(node)
    }

    pub fn local_transport_peer_id(&self) -> TransportPeerId {
        *self.swarm.local_peer_id()
    }

    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).copied()
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
        self.swarm.dial(address).context("failed to dial peer")
    }

    pub fn dial_known_peer(&mut self, peer: TransportPeerId, address: Multiaddr) -> Result<()> {
        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
        self.swarm.dial(DialOpts::peer_id(peer).addresses(vec![address]).build()).context("failed to dial known peer")
    }

    pub fn add_bootstrap_peer(&mut self, peer: TransportPeerId, address: Multiaddr) {
        self.swarm.behaviour_mut().kad.add_address(&peer, address);
    }

    pub fn add_bootstrap_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let peer = transport_peer_from_address(&address).context("bootstrap address must contain /p2p/<peer-id>")?;
        self.add_bootstrap_peer(peer, address.clone());
        if !self.swarm.is_connected(&peer) {
            self.swarm.dial(address).context("failed to dial bootstrap peer")?;
        }
        Ok(peer)
    }

    pub fn bootstrap(&mut self) -> Result<()> {
        self.swarm.behaviour_mut().kad.bootstrap().map(|_| ()).context("Kademlia bootstrap failed")
    }

    pub fn configure_relay(&mut self, relay_peer: TransportPeerId, relay_address: Multiaddr) -> Result<()> {
        let relay_address = ensure_peer_suffix(relay_address, relay_peer);
        self.diagnostics.selected_relay = Some(relay_address.to_string());
        self.swarm.behaviour_mut().kad.add_address(&relay_peer, relay_address.clone());
        self.swarm.behaviour_mut().auto_nat.add_server(relay_peer, Some(relay_address.clone()));
        if !self.swarm.is_connected(&relay_peer) {
            self.swarm.dial(relay_address.clone()).context("failed to dial relay")?;
        }
        self.swarm
            .listen_on(relay_address.with(Protocol::P2pCircuit))
            .map(|_| ())
            .context("failed to request relay reservation")
    }

    pub fn configure_relay_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let peer = transport_peer_from_address(&address).context("relay address must contain /p2p/<peer-id>")?;
        self.configure_relay(peer, address)?;
        Ok(peer)
    }

    pub fn dial_via_relay(
        &mut self,
        relay_peer: TransportPeerId,
        relay_address: Multiaddr,
        remote_peer: TransportPeerId,
    ) -> Result<()> {
        let address =
            ensure_peer_suffix(relay_address, relay_peer).with(Protocol::P2pCircuit).with(Protocol::P2p(remote_peer));
        self.swarm.dial(address).context("failed to dial peer through relay")
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

    fn respond_best_effort(
        &mut self,
        peer: TransportPeerId,
        channel: request_response::ResponseChannel<WireResponse>,
        response: WireResponse,
    ) {
        if self.respond(channel, response).is_err() {
            debug!(transport_peer = %peer, "response channel closed before response could be sent");
        }
    }

    pub async fn next_event(&mut self) -> Result<NetworkEvent> {
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => {
                    self.diagnostics.record_local_address(address.to_string());
                    if address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)) {
                        self.diagnostics.relay_connectivity = true;
                    }
                    info!(%address, "network listening");
                    return Ok(NetworkEvent::Listening { address });
                }
                SwarmEvent::ConnectionEstablished { peer_id, connection_id, endpoint, num_established, .. } => {
                    let endpoint_debug = format!("{endpoint:?}");
                    if endpoint_debug.contains("p2p-circuit") || endpoint_debug.contains("P2pCircuit") {
                        self.diagnostics.relay_connectivity = true;
                    } else {
                        self.diagnostics.record_direct_success();
                    }
                    debug!(transport_peer = %peer_id, %connection_id, %num_established, "peer connected");

                    // request-response chooses a connection by peer ID, not by connection
                    // ID. After a hard peer restart libp2p can briefly retain the dead
                    // connection while the replacement is already established. Sending
                    // PeerHello immediately can therefore land on the dead connection and
                    // leave the replacement waiting forever for authentication. Make the
                    // newest connection canonical, close the superseded connection, and
                    // send Hello after that close is observed so request-response has only
                    // the live route left.
                    let previous = self.active_connections.insert(peer_id, connection_id);
                    let defer_hello = previous
                        .filter(|previous| *previous != connection_id && num_established.get() > 1)
                        .is_some_and(|previous| self.swarm.close_connection(previous));
                    if !defer_hello {
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&peer_id, WireRequest::Hello(self.local_hello.clone()));
                    }
                    return Ok(NetworkEvent::Connected { transport_peer: peer_id });
                }
                SwarmEvent::ConnectionClosed { peer_id, connection_id, num_established, .. } => {
                    // A peer can have multiple libp2p connections at once, especially
                    // during reconnects. Closing an older connection must not erase
                    // authentication established by a newer live connection.
                    if num_established == 0 {
                        self.active_connections.remove(&peer_id);
                        self.authenticated.remove(&peer_id);
                        return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });
                    }

                    if self.active_connections.get(&peer_id).is_some_and(|active| *active != connection_id) {
                        // The superseded connection is gone. Re-send the signed hello now
                        // that request-response can route it over the replacement.
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&peer_id, WireRequest::Hello(self.local_hello.clone()));
                    }
                    debug!(transport_peer = %peer_id, %connection_id, remaining_connections = num_established, "peer connection closed; keeping authentication for remaining connection");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                    if let Some((peer, address)) = peers.into_iter().next() {
                        debug!(transport_peer = %peer, %address, "mDNS discovered peer");
                        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
                        if !self.swarm.is_connected(&peer) {
                            if let Err(error) = self.swarm.dial(address.clone()) {
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
                SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(event)) => match event {
                    request_response::Event::Message { peer, message, .. } => match message {
                        request_response::Message::Request { request, channel, .. } => {
                            if let Err(error) = request.validate_limits() {
                                warn!(transport_peer = %peer, %error, "inbound request exceeded protocol limits");
                                self.respond_best_effort(
                                    peer,
                                    channel,
                                    WireResponse::Error {
                                        code: "REQUEST_LIMIT_EXCEEDED".into(),
                                        message: error.to_string(),
                                    },
                                );
                                continue;
                            }
                            match request {
                                WireRequest::Hello(hello) => match verify_peer_hello(&hello) {
                                    Ok(()) => {
                                        self.authenticated.insert(peer, hello.peer_id);
                                        self.respond_best_effort(
                                            peer,
                                            channel,
                                            WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                        );
                                        return Ok(NetworkEvent::Authenticated {
                                            transport_peer: peer,
                                            application_peer: hello.peer_id,
                                        });
                                    }
                                    Err(error) => {
                                        self.respond_best_effort(
                                            peer,
                                            channel,
                                            WireResponse::Error {
                                                code: "PEER_AUTHENTICATION_FAILED".into(),
                                                message: error.to_string(),
                                            },
                                        );
                                    }
                                },
                                request if self.authenticated.contains_key(&peer) => {
                                    return Ok(NetworkEvent::InboundRequest { transport_peer: peer, request, channel });
                                }
                                _ => {
                                    self.respond_best_effort(
                                        peer,
                                        channel,
                                        WireResponse::Error {
                                            code: "HANDSHAKE_REQUIRED".into(),
                                            message: "authenticate with PeerHello before other requests".into(),
                                        },
                                    );
                                }
                            }
                        }
                        request_response::Message::Response { request_id, response } => {
                            return Ok(NetworkEvent::Response { transport_peer: peer, request_id, response });
                        }
                    },
                    request_response::Event::OutboundFailure { peer, request_id, error, .. } => {
                        self.diagnostics.record_direct_failure(error.to_string());
                        return Ok(NetworkEvent::OutboundFailure {
                            transport_peer: peer,
                            request_id,
                            error: error.to_string(),
                        });
                    }
                    request_response::Event::InboundFailure { peer, error, .. } => {
                        warn!(transport_peer = %peer, %error, "inbound request failed");
                    }
                    request_response::Event::ResponseSent { .. } => {}
                },
                SwarmEvent::Behaviour(BehaviourEvent::RelayClient(event)) => {
                    let event_debug = format!("{event:?}");
                    if event_debug.contains("ReservationReqAccepted") || event_debug.contains("Reservation") {
                        self.diagnostics.relay_connectivity = true;
                    }
                    debug!(?event, "relay client event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Dcutr(event)) => {
                    let event_debug = format!("{event:?}");
                    self.diagnostics.start_hole_punch();
                    if event_debug.contains("Success") {
                        self.diagnostics.finish_hole_punch(Ok::<(), String>(()));
                        self.diagnostics.direct_connectivity = true;
                    } else if event_debug.contains("Error") || event_debug.contains("Failed") {
                        self.diagnostics.finish_hole_punch(Err(event_debug.clone()));
                    }
                    info!(?event, "DCUtR hole-punch event");
                }
                SwarmEvent::Behaviour(BehaviourEvent::AutoNat(event)) => {
                    let event_debug = format!("{event:?}");
                    if event_debug.contains("Public") {
                        self.diagnostics.nat_status = NatStatusV1::Public;
                    } else if event_debug.contains("Private") {
                        self.diagnostics.nat_status = NatStatusV1::Private;
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
