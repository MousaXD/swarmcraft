use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swarm_core::{
    random_nonce, sign_friend_presence, sign_world_announcement, verify_friend_presence, verify_membership_signature,
    verify_world_announcement, verify_world_config_signature, AnnouncementReplayGuard, DataPaths, DiscoveryRecordError,
    PeerIdentity,
};
use swarm_network::{
    generate_transport_key, load_or_create_transport_key, DiscoveryNetworkEvent, DiscoveryNode, WireRequest,
    WireResponse, MAX_DISCOVERY_RESULTS,
};
use swarm_protocol::{
    peer_id_from_public_key, DiscoveryCompatibilityV1, DiscoveryFilterV1, FriendPresenceV1, MembershipPolicyV1,
    PeerId, WorldAnnouncementV1, WorldId, WorldVisibilityV1, PROTOCOL_VERSION,
};
use swarm_storage::Storage;
use tokio::time::{timeout, MissedTickBehavior};
use tracing::{debug, info, warn};

pub const DISCOVERY_CAPABILITY: &str = "discovery-v1";
pub const DISCOVERY_LISTEN_DEFAULT: &str = "/ip4/0.0.0.0/udp/0/quic-v1";
pub const WORLD_ANNOUNCEMENT_TTL_MS: u64 = 5 * 60 * 1_000;
pub const FRIEND_PRESENCE_TTL_MS: u64 = 30 * 1_000;
const DISCOVERY_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const DISCOVERY_QUERY_TIMEOUT: Duration = Duration::from_secs(8);
const FRIENDS_FILE_VERSION: u16 = 1;
const MAX_FRIEND_LABEL_BYTES: usize = 96;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendContactV1 {
    pub peer_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub accepted_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FriendStoreV1 {
    version: u16,
    friends: Vec<FriendContactV1>,
}

impl Default for FriendStoreV1 {
    fn default() -> Self {
        Self { version: FRIENDS_FILE_VERSION, friends: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedWorldV1 {
    pub world_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendCardV1 {
    pub peer_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub accepted_unix_ms: u64,
    /// `unknown` until a live presence probe is performed.
    pub presence: String,
    pub shared_worlds: Vec<SharedWorldV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendPresenceReportV1 {
    pub peer_id: String,
    /// online, offline, network_unavailable, stale, or invalid.
    pub state: String,
    pub observed_unix_ms: Option<u64>,
    pub expires_unix_ms: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldDiscoveryCardV1 {
    pub world_id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub approximate_region: Option<String>,
    pub minecraft_version: String,
    pub loader_id: String,
    pub loader_version: String,
    pub compatibility_fingerprint: String,
    pub visibility: String,
    pub membership_policy: String,
    /// request_join, request_approval, or invite_required. This is descriptive;
    /// discovery itself never changes canonical membership.
    pub join_action: String,
    pub announcer_peer_id: String,
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicWorldSearchReportV1 {
    /// ok, empty, provider_unavailable, network_unavailable, or partial.
    pub state: String,
    pub results: Vec<WorldDiscoveryCardV1>,
    pub providers_seen: usize,
    pub rejected_invalid: usize,
    pub rejected_stale: usize,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolveWorldReportV1 {
    /// found, not_found, provider_unavailable, network_unavailable, stale, or invalid.
    pub state: String,
    pub world: Option<WorldDiscoveryCardV1>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoverySearchInputV1 {
    pub query: Option<String>,
    pub minecraft_version: Option<String>,
    pub loader_id: Option<String>,
    pub loader_version: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub approximate_region: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: u16,
}

fn default_search_limit() -> u16 {
    32
}

impl Default for DiscoverySearchInputV1 {
    fn default() -> Self {
        Self {
            query: None,
            minecraft_version: None,
            loader_id: Some("fabric".into()),
            loader_version: None,
            tags: Vec::new(),
            approximate_region: None,
            limit: default_search_limit(),
        }
    }
}

impl From<DiscoverySearchInputV1> for DiscoveryFilterV1 {
    fn from(value: DiscoverySearchInputV1) -> Self {
        Self {
            query: clean_optional(value.query),
            minecraft_version: clean_optional(value.minecraft_version),
            loader_id: clean_optional(value.loader_id),
            loader_version: clean_optional(value.loader_version),
            tags: value
                .tags
                .into_iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect(),
            approximate_region: clean_optional(value.approximate_region),
            limit: value.limit.clamp(1, MAX_DISCOVERY_RESULTS as u16),
        }
    }
}

#[derive(Debug, Default)]
struct PublishedDiscoveryState {
    announcements: HashMap<WorldId, WorldAnnouncementV1>,
    provided_worlds: HashSet<WorldId>,
    public_directory: bool,
    sequences: HashMap<WorldId, u64>,
}

pub async fn serve(paths: DataPaths, listen: String) -> Result<()> {
    let storage = Storage::open(paths.root.clone())?;
    let identity = PeerIdentity::load_or_create(&paths)?;
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()])?;
    let transport_path = paths.identity_dir().join("discovery-transport.key");
    let transport_key = load_or_create_transport_key(&transport_path)?;
    let mut node = DiscoveryNode::new(transport_key, hello)?;
    node.listen(listen.parse().context("invalid discovery listen multiaddress")?)?;
    let _ = node.start_providing_friend_presence(identity.peer_id())?;

    let mut published = PublishedDiscoveryState::default();
    let mut refresh = tokio::time::interval(DISCOVERY_REFRESH_INTERVAL);
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    refresh_publications(&storage, &identity, &mut node, &mut published)?;

    info!(peer = %identity.peer_id(), "discovery service starting");
    loop {
        tokio::select! {
            _ = refresh.tick() => {
                if let Err(error) = refresh_publications(&storage, &identity, &mut node, &mut published) {
                    warn!(%error, "discovery publication refresh failed");
                }
            }
            event = node.next_event() => {
                match event? {
                    DiscoveryNetworkEvent::Listening { address } => {
                        info!(%address, "discovery listening");
                    }
                    DiscoveryNetworkEvent::InboundRequest { application_peer, request, channel, .. } => {
                        if let Err(error) = handle_discovery_request(
                            &identity,
                            &mut node,
                            &published,
                            application_peer,
                            request,
                            channel,
                        ) {
                            warn!(peer = %application_peer, %error, "discovery request rejected");
                        }
                    }
                    DiscoveryNetworkEvent::ProviderPublishFailed { error, .. } => {
                        warn!(%error, "discovery provider publication failed");
                    }
                    DiscoveryNetworkEvent::ProvidersFailed { error, .. } => {
                        debug!(%error, "discovery provider lookup failed");
                    }
                    _ => {}
                }
            }
        }
    }
}

fn refresh_publications(
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut DiscoveryNode,
    state: &mut PublishedDiscoveryState,
) -> Result<()> {
    let now = unix_millis()?;
    let mut next_announcements = HashMap::new();
    let mut next_worlds = HashSet::new();
    let mut any_public = false;

    for metadata in storage.list_worlds()? {
        let world = metadata.world_id;
        let Ok(config) = storage.load_world_config(world) else { continue };
        verify_world_config_signature(&config)?;
        let Ok(membership) = storage.load_membership_record(world) else { continue };
        verify_membership_signature(&membership)?;
        let Some(local_member) = membership.members.iter().find(|member| member.peer_id == identity.peer_id()) else {
            continue;
        };
        if local_member.banned || local_member.public_key != identity.public_key() {
            continue;
        }
        let Ok(epoch) = storage.load_epoch_record(world) else { continue };
        if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {
            continue;
        }

        match config.visibility {
            WorldVisibilityV1::Private => continue,
            WorldVisibilityV1::Unlisted | WorldVisibilityV1::Public => {}
        }

        let previous = state.sequences.get(&world).copied().unwrap_or(0);
        let sequence = now.max(previous.saturating_add(1));
        state.sequences.insert(world, sequence);
        let mut announcement = WorldAnnouncementV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            presentation: config.presentation.clone(),
            compatibility: DiscoveryCompatibilityV1 {
                minecraft_version: config.compatibility.minecraft_version.clone(),
                loader_id: config.compatibility.loader_id.clone(),
                loader_version: config.compatibility.loader_version.clone(),
                fabric_adapter_version: config.compatibility.fabric_adapter_version.clone(),
                compatibility_fingerprint: config.compatibility.fingerprint()?,
            },
            visibility: config.visibility,
            membership_policy: config.membership_policy,
            config_sequence: config.sequence,
            config_hash: config.config_hash()?,
            authority_epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            announcement_sequence: sequence,
            issued_unix_ms: now,
            expires_unix_ms: now.saturating_add(WORLD_ANNOUNCEMENT_TTL_MS),
            announcer_peer_id: identity.peer_id(),
            announcer_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        sign_world_announcement(identity, &mut announcement)?;
        verify_world_announcement(&announcement, now).map_err(|error| anyhow!(error))?;
        next_worlds.insert(world);
        next_announcements.insert(world, announcement);
        if matches!(config.visibility, WorldVisibilityV1::Public) {
            any_public = true;
        }
    }

    for world in state.provided_worlds.difference(&next_worlds).copied().collect::<Vec<_>>() {
        node.stop_providing_world(world);
    }
    for world in next_worlds.difference(&state.provided_worlds).copied().collect::<Vec<_>>() {
        let _ = node.start_providing_world(world)?;
    }
    if any_public && !state.public_directory {
        let _ = node.start_providing_public_directory()?;
    } else if !any_public && state.public_directory {
        node.stop_providing_public_directory();
    }

    state.announcements = next_announcements;
    state.provided_worlds = next_worlds;
    state.public_directory = any_public;
    Ok(())
}

fn handle_discovery_request(
    identity: &PeerIdentity,
    node: &mut DiscoveryNode,
    state: &PublishedDiscoveryState,
    application_peer: PeerId,
    request: WireRequest,
    channel: swarm_network::ResponseChannel<WireResponse>,
) -> Result<()> {
    match request {
        WireRequest::DiscoveryPublic { filter } => {
            let now = unix_millis()?;
            let limit = usize::from(filter.limit.clamp(1, MAX_DISCOVERY_RESULTS as u16));
            let values = state
                .announcements
                .values()
                .filter(|value| matches!(value.visibility, WorldVisibilityV1::Public))
                .filter(|value| value.expires_unix_ms >= now)
                .filter(|value| announcement_matches(value, &filter))
                .take(limit)
                .cloned()
                .collect();
            node.respond(channel, WireResponse::DiscoveryWorlds(values))?;
        }
        WireRequest::DiscoveryResolve { world_id } => {
            let now = unix_millis()?;
            let value = state
                .announcements
                .get(&world_id)
                .filter(|value| value.expires_unix_ms >= now)
                .filter(|value| !matches!(value.visibility, WorldVisibilityV1::Private))
                .cloned();
            node.respond(channel, WireResponse::DiscoveryResolved(value))?;
        }
        WireRequest::FriendPresence { expected_peer_id, requester_peer_id, nonce } => {
            if application_peer != requester_peer_id {
                bail!("presence requester does not match authenticated peer identity");
            }
            if expected_peer_id != identity.peer_id() {
                node.respond(channel, WireResponse::FriendPresence(None))?;
                return Ok(());
            }
            let now = unix_millis()?;
            let mut presence = FriendPresenceV1 {
                protocol_version: PROTOCOL_VERSION,
                peer_id: identity.peer_id(),
                public_key: identity.public_key(),
                requester_peer_id,
                nonce,
                issued_unix_ms: now,
                expires_unix_ms: now.saturating_add(FRIEND_PRESENCE_TTL_MS),
                signature: Vec::new(),
            };
            sign_friend_presence(identity, &mut presence)?;
            node.respond(channel, WireResponse::FriendPresence(Some(presence)))?;
        }
        _ => {
            node.respond(
                channel,
                WireResponse::Error {
                    code: "DISCOVERY_REQUEST_REQUIRED".into(),
                    message: "this endpoint accepts discovery requests only".into(),
                },
            )?;
        }
    }
    Ok(())
}

pub fn add_friend(paths: &DataPaths, peer: &str, public_key_hex: &str, label: &str) -> Result<FriendContactV1> {
    let peer_id = PeerId::from_str(peer).context("invalid friend peer ID")?;
    let public_key = parse_public_key(public_key_hex)?;
    if peer_id_from_public_key(&public_key) != peer_id {
        bail!("friend peer ID does not match the supplied public key");
    }
    let label = label.trim();
    if label.as_bytes().len() > MAX_FRIEND_LABEL_BYTES {
        bail!("friend label is too long");
    }
    let mut store = load_friend_store(paths)?;
    if let Some(existing) = store.friends.iter_mut().find(|friend| friend.peer_id == peer_id.to_string()) {
        if existing.public_key_hex != hex::encode(public_key) {
            bail!("friend identity collision: the peer ID is already stored with another public key");
        }
        existing.label = label.to_owned();
        save_friend_store(paths, &store)?;
        return Ok(existing.clone());
    }
    let contact = FriendContactV1 {
        peer_id: peer_id.to_string(),
        public_key_hex: hex::encode(public_key),
        label: label.to_owned(),
        accepted_unix_ms: unix_millis()?,
    };
    store.friends.push(contact.clone());
    store.friends.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    save_friend_store(paths, &store)?;
    Ok(contact)
}

pub fn remove_friend(paths: &DataPaths, peer: &str) -> Result<bool> {
    let peer_id = PeerId::from_str(peer).context("invalid friend peer ID")?;
    let mut store = load_friend_store(paths)?;
    let before = store.friends.len();
    store.friends.retain(|friend| friend.peer_id != peer_id.to_string());
    let changed = store.friends.len() != before;
    if changed {
        save_friend_store(paths, &store)?;
    }
    Ok(changed)
}

pub fn list_friends(paths: &DataPaths) -> Result<Vec<FriendCardV1>> {
    let store = load_friend_store(paths)?;
    let storage = Storage::open(paths.root.clone())?;
    let identity = PeerIdentity::load_or_create(paths)?;
    store
        .friends
        .into_iter()
        .map(|friend| {
            let peer_id = PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid")?;
            let public_key = parse_public_key(&friend.public_key_hex)?;
            if peer_id_from_public_key(&public_key) != peer_id {
                bail!("stored friend identity is inconsistent");
            }
            Ok(FriendCardV1 {
                shared_worlds: shared_worlds(&storage, identity.peer_id(), peer_id, public_key)?,
                peer_id: friend.peer_id,
                public_key_hex: friend.public_key_hex,
                label: friend.label,
                accepted_unix_ms: friend.accepted_unix_ms,
                presence: "unknown".into(),
            })
        })
        .collect()
}

pub async fn search_public_worlds(
    paths: &DataPaths,
    input: DiscoverySearchInputV1,
    bootstrap_addrs: &[String],
) -> Result<PublicWorldSearchReportV1> {
    let filter: DiscoveryFilterV1 = input.into();
    let identity = PeerIdentity::load_or_create(paths)?;
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()])?;
    let mut node = DiscoveryNode::new(generate_transport_key(), hello)?;
    node.listen("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())?;
    add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_public_providers();

    let mut providers = HashSet::new();
    let mut requested = HashSet::new();
    let mut pending = HashSet::new();
    let mut results = HashMap::<WorldId, WorldAnnouncementV1>::new();
    let mut replay = AnnouncementReplayGuard::default();
    let mut rejected_invalid = 0usize;
    let mut rejected_stale = 0usize;
    let mut query_finished = false;
    let mut network_error = None;

    let run = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        loop {
            let event = node.next_event().await?;
            match event {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for peer in found {
                        if providers.insert(peer) {
                            if let Err(error) = node.dial_peer(peer) {
                                debug!(transport_peer = %peer, %error, "could not dial public discovery provider");
                            }
                        }
                    }
                }
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query => {
                    query_finished = true;
                    if providers.is_empty() {
                        break;
                    }
                }
                DiscoveryNetworkEvent::ProvidersFailed { query_id, error } if query_id == query => {
                    network_error = Some(error);
                    query_finished = true;
                    if providers.is_empty() {
                        break;
                    }
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, .. } if providers.contains(&transport_peer) => {
                    if requested.insert(transport_peer) {
                        let request_id = node.send_request(&transport_peer, WireRequest::DiscoveryPublic { filter: filter.clone() })?;
                        pending.insert(format!("{request_id:?}"));
                    }
                }
                DiscoveryNetworkEvent::Response { request_id, response, .. } => {
                    pending.remove(&format!("{request_id:?}"));
                    if let WireResponse::DiscoveryWorlds(values) = response {
                        for value in values {
                            if !matches!(value.visibility, WorldVisibilityV1::Public) || !announcement_matches(&value, &filter) {
                                rejected_invalid += 1;
                                continue;
                            }
                            match verify_world_announcement(&value, unix_millis()?) {
                                Ok(()) => match replay.accept(&value) {
                                    Ok(()) => {
                                        results.insert(value.world_id, value);
                                    }
                                    Err(DiscoveryRecordError::Replay) => {
                                        // Multiple providers may legitimately return the same signed record.
                                        if !results.contains_key(&value.world_id) {
                                            rejected_stale += 1;
                                        }
                                    }
                                    Err(_) => rejected_invalid += 1,
                                },
                                Err(DiscoveryRecordError::Expired) => rejected_stale += 1,
                                Err(_) => rejected_invalid += 1,
                            }
                        }
                    }
                }
                DiscoveryNetworkEvent::OutboundFailure { request_id, error, .. } => {
                    pending.remove(&format!("{request_id:?}"));
                    network_error.get_or_insert(error);
                }
                _ => {}
            }
            if query_finished && !providers.is_empty() && requested.len() == providers.len() && pending.is_empty() {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;

    if let Ok(result) = run {
        result?;
    }

    let mut values: Vec<_> = results.into_values().collect();
    values.sort_by(|a, b| {
        a.presentation
            .name
            .to_lowercase()
            .cmp(&b.presentation.name.to_lowercase())
            .then_with(|| a.world_id.cmp(&b.world_id))
    });
    values.truncate(usize::from(filter.limit.clamp(1, MAX_DISCOVERY_RESULTS as u16)));
    let cards = values.into_iter().map(WorldDiscoveryCardV1::from).collect::<Vec<_>>();
    let state = if providers.is_empty() {
        if network_error.is_some() { "network_unavailable" } else { "provider_unavailable" }
    } else if cards.is_empty() {
        if rejected_invalid > 0 || rejected_stale > 0 { "partial" } else { "empty" }
    } else if rejected_invalid > 0 || rejected_stale > 0 {
        "partial"
    } else {
        "ok"
    };
    Ok(PublicWorldSearchReportV1 {
        state: state.into(),
        results: cards,
        providers_seen: providers.len(),
        rejected_invalid,
        rejected_stale,
        detail: network_error,
    })
}

pub async fn resolve_world(
    paths: &DataPaths,
    world: WorldId,
    bootstrap_addrs: &[String],
) -> Result<ResolveWorldReportV1> {
    let identity = PeerIdentity::load_or_create(paths)?;
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()])?;
    let mut node = DiscoveryNode::new(generate_transport_key(), hello)?;
    node.listen("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())?;
    add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_world_providers(world);
    let mut providers = HashSet::new();
    let mut requested = HashSet::new();
    let mut result = None;
    let mut invalid = false;
    let mut stale = false;
    let mut detail = None;

    let run = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        loop {
            match node.next_event().await? {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for peer in found {
                        providers.insert(peer);
                        let _ = node.dial_peer(peer);
                    }
                }
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query && providers.is_empty() => break,
                DiscoveryNetworkEvent::ProvidersFailed { query_id, error } if query_id == query => {
                    detail = Some(error);
                    if providers.is_empty() { break; }
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, .. } if providers.contains(&transport_peer) => {
                    if requested.insert(transport_peer) {
                        node.send_request(&transport_peer, WireRequest::DiscoveryResolve { world_id: world })?;
                    }
                }
                DiscoveryNetworkEvent::Response { response: WireResponse::DiscoveryResolved(Some(value)), .. } => {
                    if value.world_id != world || matches!(value.visibility, WorldVisibilityV1::Private) {
                        invalid = true;
                        continue;
                    }
                    match verify_world_announcement(&value, unix_millis()?) {
                        Ok(()) => {
                            result = Some(value);
                            break;
                        }
                        Err(DiscoveryRecordError::Expired) => stale = true,
                        Err(_) => invalid = true,
                    }
                }
                DiscoveryNetworkEvent::OutboundFailure { error, .. } => detail.get_or_insert(error),
                _ => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;
    if let Ok(value) = run { value?; }

    let state = if result.is_some() {
        "found"
    } else if stale {
        "stale"
    } else if invalid {
        "invalid"
    } else if providers.is_empty() {
        if detail.is_some() { "network_unavailable" } else { "provider_unavailable" }
    } else {
        "not_found"
    };
    Ok(ResolveWorldReportV1 { state: state.into(), world: result.map(WorldDiscoveryCardV1::from), detail })
}

pub async fn friend_presence(
    paths: &DataPaths,
    peer: PeerId,
    bootstrap_addrs: &[String],
) -> Result<FriendPresenceReportV1> {
    let contacts = load_friend_store(paths)?;
    let contact = contacts
        .friends
        .iter()
        .find(|friend| friend.peer_id == peer.to_string())
        .context("peer is not an accepted local friend")?;
    let public_key = parse_public_key(&contact.public_key_hex)?;
    if peer_id_from_public_key(&public_key) != peer {
        bail!("stored friend identity is inconsistent");
    }

    let identity = PeerIdentity::load_or_create(paths)?;
    let requester = identity.peer_id();
    let nonce = random_nonce();
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()])?;
    let mut node = DiscoveryNode::new(generate_transport_key(), hello)?;
    node.listen("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())?;
    add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_friend_providers(peer);
    let mut providers = HashSet::new();
    let mut requested = HashSet::new();
    let mut detail = None;
    let mut report = None;

    let run = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        loop {
            match node.next_event().await? {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for provider in found {
                        providers.insert(provider);
                        let _ = node.dial_peer(provider);
                    }
                }
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query && providers.is_empty() => break,
                DiscoveryNetworkEvent::ProvidersFailed { query_id, error } if query_id == query => {
                    detail = Some(error);
                    if providers.is_empty() { break; }
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, application_peer } if providers.contains(&transport_peer) => {
                    if application_peer != peer {
                        continue;
                    }
                    if requested.insert(transport_peer) {
                        node.send_request(
                            &transport_peer,
                            WireRequest::FriendPresence {
                                expected_peer_id: peer,
                                requester_peer_id: requester,
                                nonce,
                            },
                        )?;
                    }
                }
                DiscoveryNetworkEvent::Response { response: WireResponse::FriendPresence(Some(presence)), .. } => {
                    let now = unix_millis()?;
                    let state = match verify_friend_presence(&presence, peer, requester, nonce, now) {
                        Ok(()) if presence.public_key == public_key => "online",
                        Ok(()) => "invalid",
                        Err(DiscoveryRecordError::Expired) => "stale",
                        Err(_) => "invalid",
                    };
                    report = Some(FriendPresenceReportV1 {
                        peer_id: peer.to_string(),
                        state: state.into(),
                        observed_unix_ms: Some(now),
                        expires_unix_ms: Some(presence.expires_unix_ms),
                        detail: None,
                    });
                    break;
                }
                DiscoveryNetworkEvent::OutboundFailure { error, .. } => detail.get_or_insert(error),
                _ => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;
    if let Ok(value) = run { value?; }

    Ok(report.unwrap_or_else(|| FriendPresenceReportV1 {
        peer_id: peer.to_string(),
        state: if providers.is_empty() && detail.is_some() {
            "network_unavailable".into()
        } else {
            "offline".into()
        },
        observed_unix_ms: None,
        expires_unix_ms: None,
        detail,
    }))
}

fn shared_worlds(storage: &Storage, local: PeerId, friend: PeerId, friend_key: [u8; 32]) -> Result<Vec<SharedWorldV1>> {
    let mut worlds = Vec::new();
    for metadata in storage.list_worlds()? {
        let Ok(membership) = storage.load_membership_record(metadata.world_id) else { continue };
        if verify_membership_signature(&membership).is_err() {
            continue;
        }
        let local_member = membership.members.iter().find(|member| member.peer_id == local && !member.banned);
        let friend_member = membership
            .members
            .iter()
            .find(|member| member.peer_id == friend && member.public_key == friend_key && !member.banned);
        if local_member.is_some() && friend_member.is_some() {
            worlds.push(SharedWorldV1 { world_id: metadata.world_id.to_string(), name: metadata.display_name });
        }
    }
    worlds.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.world_id.cmp(&b.world_id)));
    Ok(worlds)
}

fn announcement_matches(value: &WorldAnnouncementV1, filter: &DiscoveryFilterV1) -> bool {
    if let Some(expected) = filter.minecraft_version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        if !value.compatibility.minecraft_version.eq_ignore_ascii_case(expected) {
            return false;
        }
    }
    if let Some(expected) = filter.loader_id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        if !value.compatibility.loader_id.eq_ignore_ascii_case(expected) {
            return false;
        }
    }
    if let Some(expected) = filter.loader_version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        if !value.compatibility.loader_version.eq_ignore_ascii_case(expected) {
            return false;
        }
    }
    if let Some(expected) = filter.approximate_region.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        if !value
            .presentation
            .approximate_region
            .as_deref()
            .is_some_and(|region| region.eq_ignore_ascii_case(expected))
        {
            return false;
        }
    }
    for tag in filter.tags.iter().map(|tag| tag.trim()).filter(|tag| !tag.is_empty()) {
        if !value.presentation.tags.iter().any(|candidate| candidate.eq_ignore_ascii_case(tag)) {
            return false;
        }
    }
    if let Some(query) = filter.query.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let query = query.to_lowercase();
        let found = value.presentation.name.to_lowercase().contains(&query)
            || value.presentation.description.to_lowercase().contains(&query)
            || value.presentation.tags.iter().any(|tag| tag.to_lowercase().contains(&query));
        if !found {
            return false;
        }
    }
    true
}

impl From<WorldAnnouncementV1> for WorldDiscoveryCardV1 {
    fn from(value: WorldAnnouncementV1) -> Self {
        let (membership_policy, join_action) = match value.membership_policy {
            MembershipPolicyV1::Open => ("open", "request_join"),
            MembershipPolicyV1::ApprovalRequired => ("approval_required", "request_approval"),
            MembershipPolicyV1::InviteOnly => ("invite_only", "invite_required"),
        };
        Self {
            world_id: value.world_id.to_string(),
            name: value.presentation.name,
            description: value.presentation.description,
            tags: value.presentation.tags,
            approximate_region: value.presentation.approximate_region,
            minecraft_version: value.compatibility.minecraft_version,
            loader_id: value.compatibility.loader_id,
            loader_version: value.compatibility.loader_version,
            compatibility_fingerprint: value.compatibility.compatibility_fingerprint.to_string(),
            visibility: match value.visibility {
                WorldVisibilityV1::Private => "private",
                WorldVisibilityV1::Unlisted => "unlisted",
                WorldVisibilityV1::Public => "public",
            }
            .into(),
            membership_policy: membership_policy.into(),
            join_action: join_action.into(),
            announcer_peer_id: value.announcer_peer_id.to_string(),
            issued_unix_ms: value.issued_unix_ms,
            expires_unix_ms: value.expires_unix_ms,
        }
    }
}

fn add_explicit_bootstraps(node: &mut DiscoveryNode, values: &[String]) -> Result<()> {
    let mut any = false;
    for value in values.iter().map(|value| value.trim()).filter(|value| !value.is_empty()) {
        node.add_bootstrap_address(value.parse().with_context(|| format!("invalid discovery bootstrap address: {value}"))?)?;
        any = true;
    }
    if any {
        node.bootstrap()?;
    }
    Ok(())
}

fn load_friend_store(paths: &DataPaths) -> Result<FriendStoreV1> {
    let path = friends_path(paths);
    if !path.exists() {
        return Ok(FriendStoreV1::default());
    }
    let bytes = fs::read(&path).with_context(|| format!("could not read {}", path.display()))?;
    let mut store: FriendStoreV1 = serde_json::from_slice(&bytes).context("friend store is malformed")?;
    if store.version != FRIENDS_FILE_VERSION {
        bail!("unsupported friend store version {}", store.version);
    }
    store.friends.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    store.friends.dedup_by(|a, b| a.peer_id == b.peer_id && a.public_key_hex == b.public_key_hex);
    for friend in &store.friends {
        let peer = PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid")?;
        let public_key = parse_public_key(&friend.public_key_hex)?;
        if peer_id_from_public_key(&public_key) != peer {
            bail!("stored friend peer ID does not match its public key");
        }
    }
    Ok(store)
}

fn save_friend_store(paths: &DataPaths, store: &FriendStoreV1) -> Result<()> {
    paths.ensure()?;
    let path = friends_path(paths);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(store)?;
    let mut file = private_file(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, &path).with_context(|| format!("could not persist {}", path.display()))?;
    sync_parent(path.parent().context("friend store has no parent")?)?;
    Ok(())
}

fn friends_path(paths: &DataPaths) -> PathBuf {
    paths.root.join("friends.json")
}

fn private_file(path: &Path) -> Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        Ok(fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(path)?)
    }
    #[cfg(not(unix))]
    {
        Ok(fs::OpenOptions::new().create(true).truncate(true).write(true).open(path)?)
    }
}

fn sync_parent(parent: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

fn parse_public_key(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value.trim()).context("friend public key must be hex")?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| anyhow!("friend public key must be 32 bytes, got {}", bytes.len()))
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

fn unix_millis() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_core::PeerIdentity;
    use swarm_protocol::{
        AuthorityPolicyV1, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, RuntimeCompatibilityManifestV1,
        WorldConfigV1, WorldMemberV1, WorldPresentationV1,
    };
    use swarm_storage::{WorldMetadataV1, STORAGE_SCHEMA_VERSION};

    #[test]
    fn friend_store_is_cryptographic_and_label_collisions_are_harmless() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let a = PeerIdentity::from_secret_bytes([1; 32]);
        let b = PeerIdentity::from_secret_bytes([2; 32]);
        add_friend(&paths, &a.peer_id().to_string(), &hex::encode(a.public_key()), "Alex").unwrap();
        add_friend(&paths, &b.peer_id().to_string(), &hex::encode(b.public_key()), "Alex").unwrap();
        assert_eq!(list_friends(&paths).unwrap().len(), 2);

        let err = add_friend(&paths, &a.peer_id().to_string(), &hex::encode(b.public_key()), "collision").unwrap_err();
        assert!(err.to_string().contains("does not match"));
    }

    #[test]
    fn private_and_unlisted_worlds_never_match_public_browse() {
        let identity = PeerIdentity::from_secret_bytes([3; 32]);
        let public = sample_announcement(&identity, WorldVisibilityV1::Public);
        let unlisted = sample_announcement(&identity, WorldVisibilityV1::Unlisted);
        let private = sample_announcement(&identity, WorldVisibilityV1::Private);
        let filter = DiscoveryFilterV1 { limit: 10, ..Default::default() };
        assert!(matches!(public.visibility, WorldVisibilityV1::Public) && announcement_matches(&public, &filter));
        assert!(!matches!(unlisted.visibility, WorldVisibilityV1::Public));
        assert!(!matches!(private.visibility, WorldVisibilityV1::Public));
    }

    #[test]
    fn banned_or_removed_friend_is_not_reported_as_shared_world() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let storage = Storage::open(paths.root.clone()).unwrap();
        let local = PeerIdentity::from_secret_bytes([4; 32]);
        let friend = PeerIdentity::from_secret_bytes([5; 32]);
        let world = WorldId([6; 32]);
        let genesis = swarm_protocol::WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([7; 32]),
            creation_nonce: [8; 32],
            creator_public_key: local.public_key(),
            initial_membership: vec![local.peer_id()],
        };
        storage
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "Shared".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        let mut membership = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 1,
            sequence: 1,
            previous_membership_hash: None,
            members: vec![
                WorldMemberV1 {
                    peer_id: local.peer_id(),
                    public_key: local.public_key(),
                    authority_eligible: true,
                    banned: false,
                },
                WorldMemberV1 {
                    peer_id: friend.peer_id(),
                    public_key: friend.public_key(),
                    authority_eligible: true,
                    banned: true,
                },
            ],
            authority_peer_id: local.peer_id(),
            authority_public_key: local.public_key(),
            signature: Vec::new(),
        };
        local.sign_membership(&mut membership).unwrap();
        storage.save_membership_record(&membership).unwrap();
        assert!(shared_worlds(&storage, local.peer_id(), friend.peer_id(), friend.public_key()).unwrap().is_empty());
    }

    fn sample_announcement(identity: &PeerIdentity, visibility: WorldVisibilityV1) -> WorldAnnouncementV1 {
        let mut value = WorldAnnouncementV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([9; 32]),
            presentation: WorldPresentationV1 {
                name: "Example".into(),
                description: "survival".into(),
                tags: vec!["survival".into()],
                icon_hash: None,
                approximate_region: Some("me".into()),
            },
            compatibility: DiscoveryCompatibilityV1 {
                minecraft_version: "1.21.8".into(),
                loader_id: "fabric".into(),
                loader_version: "0.17.2".into(),
                fabric_adapter_version: "0.4.0".into(),
                compatibility_fingerprint: Hash32([10; 32]),
            },
            visibility,
            membership_policy: MembershipPolicyV1::InviteOnly,
            config_sequence: 1,
            config_hash: Hash32([11; 32]),
            authority_epoch: 1,
            fencing_token: 1,
            announcement_sequence: 1,
            issued_unix_ms: 1,
            expires_unix_ms: 2,
            announcer_peer_id: identity.peer_id(),
            announcer_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        let _ = sign_world_announcement(identity, &mut value);
        value
    }

    #[allow(dead_code)]
    fn canonical_fixture(identity: &PeerIdentity, world: WorldId) -> (WorldConfigV1, EpochRecordV1) {
        (
            WorldConfigV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                sequence: 1,
                previous_config_hash: None,
                compatibility: RuntimeCompatibilityManifestV1 {
                    minecraft_version: "1.21.8".into(),
                    loader_id: "fabric".into(),
                    loader_version: "0.17.2".into(),
                    swarmcraft_protocol_version: PROTOCOL_VERSION,
                    fabric_adapter_version: "0.4.0".into(),
                    required_server_mods: Vec::new(),
                    required_client_mods: Vec::new(),
                    datapacks: Vec::new(),
                },
                visibility: WorldVisibilityV1::Public,
                authority_policy: AuthorityPolicyV1 { allow_solo_advancement: true, preferred_replication_factor: 2 },
                membership_policy: MembershipPolicyV1::InviteOnly,
                presentation: WorldPresentationV1 {
                    name: "Example".into(),
                    description: String::new(),
                    tags: Vec::new(),
                    icon_hash: None,
                    approximate_region: None,
                },
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            },
            EpochRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                epoch_number: 1,
                previous_epoch_hash: None,
                base_state_hash: Hash32([12; 32]),
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                mode: EpochMode::Quorum,
                fencing_token: 1,
                reason: "fixture".into(),
                signature: Vec::new(),
            },
        )
    }
}
