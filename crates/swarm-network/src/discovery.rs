use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use libp2p::{
    identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    mdns, noise, ping,
    request_response::{self, cbor, ProtocolSupport},
    swarm::{dial_opts::DialOpts, NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId as TransportPeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use std::{collections::HashMap, env, time::Duration};
use swarm_protocol::{PeerHelloV1, PeerId, WorldId, PROTOCOL_VERSION};
use tracing::{debug, warn};

use crate::{verify_peer_hello, WireRequest, WireResponse, BOOTSTRAP_ENV};

pub const DISCOVERY_WIRE_PROTOCOL: &str = "/swarmcraft/discovery/1";
const PUBLIC_DIRECTORY_KEY: &[u8] = b"swarmcraft/discovery/public/v1";
const WORLD_KEY_PREFIX: &[u8] = b"swarmcraft/discovery/world/v1\0";
const FRIEND_KEY_PREFIX: &[u8] = b"swarmcraft/discovery/friend/v1\0";

#[derive(NetworkBehaviour)]
struct DiscoveryBehaviour {
    request_response: cbor::Behaviour<WireRequest, WireResponse>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    kad: kad::Behaviour<MemoryStore>,
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
    authenticated: HashMap<TransportPeerId, PeerId>,
}

impl DiscoveryNode {
    pub fn new(transport_key: Keypair, local_hello: PeerHelloV1) -> Result<Self> {
        verify_peer_hello(&local_hello).context("local discovery PeerHello must be valid")?;
        let local_peer = transport_key.public().to_peer_id();
        let request_response = cbor::Behaviour::new(
            [(StreamProtocol::new(DISCOVERY_WIRE_PROTOCOL), ProtocolSupport::Full)],
            request_response::Config::default()
                .with_request_timeout(Duration::from_secs(15))
                .with_max_concurrent_streams(64),
        );
        let mdns =
            mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer).context("failed to initialize discovery mDNS")?;
        let identify = identify::Behaviour::new(identify::Config::new(
            DISCOVERY_WIRE_PROTOCOL.to_owned(),
            transport_key.public(),
        ));
        let mut kad_config = kad::Config::default();
        kad_config.set_query_timeout(Duration::from_secs(15));
        let mut kad = kad::Behaviour::with_config(local_peer, MemoryStore::new(local_peer), kad_config);
        // Discovery nodes must answer provider lookups even on private/LAN test
        // networks where AutoNAT has not classified an external address yet.
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
            })?
            .build();

        let mut node = Self { swarm, local_hello, authenticated: HashMap::new() };
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
        self.authenticated.get(transport_peer).copied()
    }

    pub fn add_peer_address(&mut self, peer: TransportPeerId, address: Multiaddr) {
        self.swarm.behaviour_mut().kad.add_address(&peer, address);
    }

    pub fn add_bootstrap_address(&mut self, address: Multiaddr) -> Result<TransportPeerId> {
        let peer = transport_peer_from_address(&address)
            .ok_or_else(|| anyhow!("bootstrap address must contain /p2p/<peer-id>"))?;
        self.add_peer_address(peer, address.clone());
        if !self.swarm.is_connected(&peer) {
            let options = DialOpts::peer_id(peer).addresses(vec![address]).build();
            self.swarm.dial(options).context("failed to dial discovery bootstrap peer")?;
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
            return Ok(());
        }
        self.swarm
            .dial(DialOpts::peer_id(peer).build())
            .context("failed to dial discovery provider")
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

    pub fn start_providing_friend_presence(&mut self, peer: PeerId) -> Result<kad::QueryId> {
        self.swarm
            .behaviour_mut()
            .kad
            .start_providing(friend_presence_key(peer))
            .context("failed to publish friend presence provider")
    }

    pub fn find_public_providers(&mut self) -> kad::QueryId {
        self.swarm.behaviour_mut().kad.get_providers(public_directory_key())
    }

    pub fn find_world_providers(&mut self, world: WorldId) -> kad::QueryId {
        self.swarm.behaviour_mut().kad.get_providers(world_discovery_key(world))
    }

    pub fn find_friend_providers(&mut self, peer: PeerId) -> kad::QueryId {
        self.swarm.behaviour_mut().kad.get_providers(friend_presence_key(peer))
    }

    pub async fn next_event(&mut self) -> Result<DiscoveryNetworkEvent> {
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => {
                    return Ok(DiscoveryNetworkEvent::Listening { address });
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(&peer_id, WireRequest::Hello(self.local_hello.clone()));
                }
                SwarmEvent::ConnectionClosed { peer_id, num_established, .. } => {
                    if num_established == 0 {
                        let application_peer = self.authenticated.remove(&peer_id);
                        return Ok(DiscoveryNetworkEvent::Disconnected { transport_peer: peer_id, application_peer });
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
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                    for address in info.listen_addrs {
                        self.swarm.behaviour_mut().kad.add_address(&peer_id, address);
                    }
                }
                SwarmEvent::Behaviour(DiscoveryBehaviourEvent::RequestResponse(event)) => match event {
                    request_response::Event::Message { peer, message, .. } => match message {
                        request_response::Message::Request { request, channel, .. } => {
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
                                WireRequest::Hello(hello) => match verify_peer_hello(&hello) {
                                    Ok(()) => {
                                        self.authenticated.insert(peer, hello.peer_id);
                                        self.respond(
                                            channel,
                                            WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                        )?;
                                        return Ok(DiscoveryNetworkEvent::Authenticated {
                                            transport_peer: peer,
                                            application_peer: hello.peer_id,
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
                                },
                                request => {
                                    if let Some(application_peer) = self.authenticated.get(&peer).copied() {
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
                                            message: "authenticate with PeerHello before discovery requests".into(),
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
                    kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. })) => {
                        return Ok(DiscoveryNetworkEvent::ProvidersFinished { query_id: id });
                    }
                    kad::QueryResult::GetProviders(Err(error)) => {
                        return Ok(DiscoveryNetworkEvent::ProvidersFailed { query_id: id, error: error.to_string() });
                    }
                    kad::QueryResult::StartProviding(Ok(_)) => {
                        return Ok(DiscoveryNetworkEvent::ProviderPublished { query_id: id });
                    }
                    kad::QueryResult::StartProviding(Err(error)) => {
                        return Ok(DiscoveryNetworkEvent::ProviderPublishFailed { query_id: id, error: error.to_string() });
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
}

pub fn public_directory_key() -> kad::RecordKey {
    kad::RecordKey::new(PUBLIC_DIRECTORY_KEY)
}

pub fn world_discovery_key(world: WorldId) -> kad::RecordKey {
    let mut bytes = Vec::with_capacity(WORLD_KEY_PREFIX.len() + 32);
    bytes.extend_from_slice(WORLD_KEY_PREFIX);
    bytes.extend_from_slice(&world.0);
    kad::RecordKey::new(&bytes)
}

pub fn friend_presence_key(peer: PeerId) -> kad::RecordKey {
    let mut bytes = Vec::with_capacity(FRIEND_KEY_PREFIX.len() + 32);
    bytes.extend_from_slice(FRIEND_KEY_PREFIX);
    bytes.extend_from_slice(&peer.0);
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
    use swarm_protocol::{peer_id_from_public_key, PeerHelloV1};

    #[test]
    fn discovery_keys_separate_public_world_and_friend_namespaces() {
        let world_a = world_discovery_key(WorldId([1; 32]));
        let world_b = world_discovery_key(WorldId([2; 32]));
        let friend = friend_presence_key(PeerId([1; 32]));
        assert_ne!(world_a, world_b);
        assert_ne!(world_a, friend);
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
        assert!(DiscoveryNode::new(Keypair::generate_ed25519(), hello).is_err());
    }
}
