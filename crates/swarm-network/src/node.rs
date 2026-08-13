use crate::{verify_peer_hello, wire::WireRequest, wire::WireResponse};
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use libp2p::{
    identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    mdns, ping,
    request_response::{self, cbor, ProtocolSupport},
    swarm::{dial_opts::DialOpts, NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId as TransportPeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use std::{collections::HashMap, time::Duration};
use swarm_protocol::{PeerHelloV1, PeerId, PROTOCOL_VERSION};
use tracing::{debug, info, warn};

pub const WIRE_PROTOCOL: &str = "/swarmcraft/1";

#[derive(NetworkBehaviour)]
struct Behaviour {
    request_response: cbor::Behaviour<WireRequest, WireResponse>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    kad: kad::Behaviour<MemoryStore>,
}

impl Behaviour {
    fn new(key: &Keypair) -> Result<Self> {
        let local_peer = key.public().to_peer_id();
        let request_response = cbor::Behaviour::new(
            [(StreamProtocol::new(WIRE_PROTOCOL), ProtocolSupport::Full)],
            request_response::Config::default()
                .with_request_timeout(Duration::from_secs(30))
                .with_max_concurrent_streams(128),
        );
        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer)
            .context("failed to initialize mDNS")?;
        let identify = identify::Behaviour::new(identify::Config::new(WIRE_PROTOCOL.to_owned(), key.public()));
        let mut kad_config = kad::Config::default();
        kad_config.set_query_timeout(Duration::from_secs(30));
        let kad = kad::Behaviour::with_config(local_peer, MemoryStore::new(local_peer), kad_config);
        Ok(Self { request_response, mdns, identify, ping: ping::Behaviour::default(), kad })
    }
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
}

impl SwarmNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1) -> Result<Self> {
        verify_peer_hello(&local_hello).context("local peer hello must be valid before networking starts")?;
        let swarm = SwarmBuilder::with_existing_identity(transport_key)
            .with_tokio()
            .with_quic()
            .with_behaviour(Behaviour::new)?
            .build();
        Ok(Self { swarm, local_hello, authenticated: HashMap::new() })
    }

    pub fn local_transport_peer_id(&self) -> TransportPeerId {
        *self.swarm.local_peer_id()
    }

    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).copied()
    }

    pub fn listen(&mut self, address: Multiaddr) -> Result<()> {
        self.swarm.listen_on(address).map(|_| ()).context("failed to listen")
    }

    pub fn dial(&mut self, address: Multiaddr) -> Result<()> {
        self.swarm.dial(address).context("failed to dial peer")
    }

    pub fn dial_known_peer(&mut self, peer: TransportPeerId, address: Multiaddr) -> Result<()> {
        self.swarm.behaviour_mut().kad.add_address(&peer, address.clone());
        self.swarm
            .dial(DialOpts::peer_id(peer).addresses(vec![address]).build())
            .context("failed to dial known peer")
    }

    pub fn add_bootstrap_peer(&mut self, peer: TransportPeerId, address: Multiaddr) {
        self.swarm.behaviour_mut().kad.add_address(&peer, address);
    }

    pub fn bootstrap(&mut self) -> Result<()> {
        self.swarm.behaviour_mut().kad.bootstrap().map(|_| ()).context("Kademlia bootstrap failed")
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
                    info!(%address, "network listening");
                    return Ok(NetworkEvent::Listening { address });
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    debug!(transport_peer = %peer_id, "peer connected");
                    self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(&peer_id, WireRequest::Hello(self.local_hello.clone()));
                    return Ok(NetworkEvent::Connected { transport_peer: peer_id });
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    self.authenticated.remove(&peer_id);
                    return Ok(NetworkEvent::Disconnected { transport_peer: peer_id });
                }
                SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                    for (peer, address) in peers {
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
                SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                    for address in info.listen_addrs {
                        self.swarm.behaviour_mut().kad.add_address(&peer_id, address);
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(event)) => match event {
                    request_response::Event::Message { peer, message, .. } => match message {
                        request_response::Message::Request { request, channel, .. } => {
                            request.validate_limits()?;
                            match request {
                                WireRequest::Hello(hello) => match verify_peer_hello(&hello) {
                                    Ok(()) => {
                                        self.authenticated.insert(peer, hello.peer_id);
                                        self.respond(
                                            channel,
                                            WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                        )?;
                                        return Ok(NetworkEvent::Authenticated {
                                            transport_peer: peer,
                                            application_peer: hello.peer_id,
                                        });
                                    }
                                    Err(error) => {
                                        self.respond(
                                            channel,
                                            WireResponse::Error {
                                                code: "PEER_AUTHENTICATION_FAILED".into(),
                                                message: error.to_string(),
                                            },
                                        )?;
                                    }
                                },
                                request if self.authenticated.contains_key(&peer) => {
                                    return Ok(NetworkEvent::InboundRequest {
                                        transport_peer: peer,
                                        request,
                                        channel,
                                    });
                                }
                                _ => {
                                    self.respond(
                                        channel,
                                        WireResponse::Error {
                                            code: "HANDSHAKE_REQUIRED".into(),
                                            message: "authenticate with PeerHello before other requests".into(),
                                        },
                                    )?;
                                }
                            }
                        }
                        request_response::Message::Response { request_id, response } => {
                            return Ok(NetworkEvent::Response {
                                transport_peer: peer,
                                request_id,
                                response,
                            });
                        }
                    },
                    request_response::Event::OutboundFailure { peer, request_id, error } => {
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
                SwarmEvent::Behaviour(BehaviourEvent::Kad(_))
                | SwarmEvent::Behaviour(BehaviourEvent::Ping(_))
                | SwarmEvent::Behaviour(BehaviourEvent::Identify(_))
                | SwarmEvent::Behaviour(BehaviourEvent::Mdns(_)) => {}
                other => debug!(event = ?other, "network event"),
            }
        }
    }
}
