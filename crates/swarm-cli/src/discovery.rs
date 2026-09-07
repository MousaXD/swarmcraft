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
use swarm_consensus::{validate_discovery_freshness_quorum, validate_discovery_membership_proof_shape};
use swarm_core::{
    random_nonce, sign_discovery_freshness_vote, sign_friend_presence, sign_world_announcement,
    verify_discovery_freshness_challenge, verify_discovery_freshness_vote, verify_discovery_membership_proof,
    verify_friend_presence, verify_membership_signature, verify_world_announcement, verify_world_config_signature,
    AnnouncementReplayGuard, DataPaths, DiscoveryFreshnessReplayGuard, DiscoveryRecordError, PeerIdentity,
    DISCOVERY_FRESHNESS_MAX_LIFETIME_MS,
};
use swarm_network::{
    generate_transport_key, load_or_create_transport_key, DiscoveryNetworkEvent, DiscoveryNode, TransportPeerId,
    WireRequest, WireResponse, MAX_DISCOVERY_RESULTS,
};
use swarm_protocol::{
    peer_id_from_public_key, DiscoveryCanonicalHeadV1, DiscoveryCompatibilityV1, DiscoveryFilterV1,
    DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1, FriendPresenceV1,
    MembershipPolicyV1, PeerId, WorldAnnouncementV1, WorldId, WorldVisibilityV1, PROTOCOL_VERSION,
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
const DISCOVERY_FRESHNESS_TIMEOUT: Duration = Duration::from_secs(5);
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
    presence_requesters: HashSet<PeerId>,
    signed_freshness_challenges: HashMap<(PeerId, [u8; 32]), u64>,
}

pub async fn serve(paths: DataPaths, listen: String) -> Result<()> {
    let storage = Storage::open(paths.root.clone())?;
    let identity = PeerIdentity::load_or_create(&paths)?;
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()])?;
    let transport_path = paths.identity_dir().join("discovery-transport.key");
    let transport_key = load_or_create_transport_key(&transport_path)?;
    let mut node = DiscoveryNode::new(transport_key, hello, identity.network_signing_key())?;
    node.listen(listen.parse().context("invalid discovery listen multiaddress")?)?;

    let mut published = PublishedDiscoveryState::default();
    let mut refresh = tokio::time::interval(DISCOVERY_REFRESH_INTERVAL);
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    refresh_publications(&storage, &identity, &mut node, &mut published)?;
    refresh_presence_publications(&paths, &identity, &mut node, &mut published)?;

    info!(peer = %identity.peer_id(), "discovery service starting");
    loop {
        tokio::select! {
            _ = refresh.tick() => {
                if let Err(error) = refresh_publications(&storage, &identity, &mut node, &mut published) {
                    warn!(%error, "discovery publication refresh failed");
                }
                if let Err(error) = refresh_presence_publications(&paths, &identity, &mut node, &mut published) {
                    warn!(%error, "friend presence publication refresh failed");
                }
            }
            event = node.next_event() => {
                match event? {
                    DiscoveryNetworkEvent::Listening { address } => {
                        info!(%address, "discovery listening");
                    }
                    DiscoveryNetworkEvent::InboundRequest { application_peer, request, channel, .. } => {
                        if let Err(error) = handle_discovery_request(
                            &paths,
                            &storage,
                            &identity,
                            &mut node,
                            &mut published,
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
        let local_is_current = membership.members.iter().any(|member| {
            member.peer_id == identity.peer_id() && member.public_key == identity.public_key() && !member.banned
        });
        let local_is_pending = storage.load_membership_promise(world).ok().is_some_and(|promise| {
            promise.proposal.proposed.members.iter().any(|member| {
                member.peer_id == identity.peer_id() && member.public_key == identity.public_key() && !member.banned
            })
        });
        if !local_is_current && !local_is_pending {
            continue;
        }
        match config.visibility {
            WorldVisibilityV1::Private => continue,
            WorldVisibilityV1::Unlisted | WorldVisibilityV1::Public => {}
        }
        // The DHT provider identity is only a locator. Publishing all current
        // active members under the exact-world key makes a live quorum
        // discoverable without granting any provider authority.
        next_worlds.insert(world);

        let Ok(epoch) = storage.load_epoch_record(world) else { continue };
        if !local_is_current
            || epoch.authority_peer_id != identity.peer_id()
            || epoch.authority_public_key != identity.public_key()
        {
            continue;
        }

        let previous = state.sequences.get(&world).copied().unwrap_or(0);
        let sequence = next_discovery_sequence(previous, now)?;
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
            membership_sequence: membership.sequence,
            membership_hash: membership.record_hash()?,
            authority_epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            canonical_head: storage.canonical_snapshot_head(world)?.head.map(|head| DiscoveryCanonicalHeadV1 {
                snapshot_number: head.snapshot_number,
                manifest_hash: head.manifest_hash,
                epoch: head.epoch,
                sequence: head.sequence,
            }),
            announcement_sequence: sequence,
            issued_unix_ms: now,
            expires_unix_ms: checked_discovery_expiry(now, WORLD_ANNOUNCEMENT_TTL_MS, "discovery")?,
            announcer_peer_id: identity.peer_id(),
            announcer_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        sign_world_announcement(identity, &mut announcement)?;
        verify_world_announcement(&announcement, now).map_err(|error| anyhow!(error))?;
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

#[allow(clippy::too_many_arguments)]
fn handle_discovery_request(
    paths: &DataPaths,
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut DiscoveryNode,
    state: &mut PublishedDiscoveryState,
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
                .cloned()
                .map(Box::new);
            node.respond(channel, WireResponse::DiscoveryResolved(value))?;
        }
        WireRequest::DiscoveryFreshnessContext {
            world_id,
            announcement_hash,
            verifier_peer_id,
            nonce: _,
            issued_unix_ms,
            expires_unix_ms,
        } => {
            let now = unix_millis()?;
            if application_peer != verifier_peer_id
                || expires_unix_ms <= issued_unix_ms
                || expires_unix_ms.saturating_sub(issued_unix_ms) > DISCOVERY_FRESHNESS_MAX_LIFETIME_MS
                || expires_unix_ms < now
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            }
            let Some(announcement) = state.announcements.get(&world_id) else {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            };
            if announcement.announcement_hash()? != announcement_hash
                || announcement.announcer_peer_id != identity.peer_id()
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            }
            let proof = build_discovery_membership_proof(storage, world_id)?;
            node.respond(channel, WireResponse::DiscoveryFreshnessContext(Some(Box::new(proof))))?;
        }
        WireRequest::DiscoveryFreshnessVote(challenge) => {
            let challenge = *challenge;
            let now = unix_millis()?;
            if application_peer != challenge.verifier_peer_id
                || challenge.protocol_version != PROTOCOL_VERSION
                || challenge.expires_unix_ms <= challenge.issued_unix_ms
                || challenge.expires_unix_ms.saturating_sub(challenge.issued_unix_ms)
                    > DISCOVERY_FRESHNESS_MAX_LIFETIME_MS
                || challenge.expires_unix_ms < now
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessVote(None))?;
                return Ok(());
            }
            state.signed_freshness_challenges.retain(|_, expires| *expires >= now);
            let replay_key = (challenge.verifier_peer_id, challenge.nonce);
            if state.signed_freshness_challenges.contains_key(&replay_key)
                || !local_state_matches_freshness_challenge(storage, identity, &challenge)?
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessVote(None))?;
                return Ok(());
            }
            state.signed_freshness_challenges.insert(replay_key, challenge.expires_unix_ms);
            let vote = sign_discovery_freshness_vote(identity, &challenge)?;
            node.respond(channel, WireResponse::DiscoveryFreshnessVote(Some(Box::new(vote))))?;
        }
        WireRequest::FriendPresence { expected_peer_id, requester_peer_id, nonce } => {
            if application_peer != requester_peer_id {
                bail!("presence requester does not match authenticated peer identity");
            }
            if !accepted_friend_peers(paths)?.contains(&requester_peer_id) {
                node.respond(channel, WireResponse::FriendPresence(None))?;
                return Ok(());
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

fn build_discovery_membership_proof(storage: &Storage, world: WorldId) -> Result<DiscoveryMembershipProofV1> {
    let metadata = storage.load_world(world)?;
    let current = storage.load_membership_record(world)?;
    verify_membership_signature(&current)?;
    let certificates = storage.load_membership_certificate_chain(world)?;
    let initial = certificates
        .first()
        .map(|certificate| certificate.proposal.previous.clone())
        .unwrap_or_else(|| current.clone());
    let pending = storage.load_membership_promise(world).ok().map(|promise| promise.proposal);
    let proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: metadata.genesis,
        initial_membership: initial,
        membership_certificates: certificates,
        current_membership: current,
        pending_membership: pending,
    };
    validate_discovery_membership_proof_shape(&proof).map_err(|error| anyhow!(error))?;
    Ok(proof)
}

fn local_state_matches_freshness_challenge(
    storage: &Storage,
    identity: &PeerIdentity,
    challenge: &DiscoveryFreshnessChallengeV1,
) -> Result<bool> {
    let world = challenge.world_id;
    let membership = match storage.load_membership_record(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    verify_membership_signature(&membership)?;
    if membership.sequence != challenge.membership_sequence || membership.record_hash()? != challenge.membership_hash {
        return Ok(false);
    }
    if storage.load_recovery_promise(world).ok().is_some_and(|promise| {
        (promise.ballot.target_epoch, promise.ballot.target_fencing_token)
            > (challenge.authority_epoch, challenge.fencing_token)
    }) {
        return Ok(false);
    }
    let epoch = match storage.load_epoch_record(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    if epoch.authority_peer_id != challenge.authority_peer_id
        || epoch.epoch_number != challenge.authority_epoch
        || epoch.fencing_token != challenge.fencing_token
    {
        return Ok(false);
    }
    let config = match storage.load_world_config(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    verify_world_config_signature(&config)?;
    if config.authority_peer_id != challenge.authority_peer_id
        || config.sequence != challenge.config_sequence
        || config.config_hash()? != challenge.config_hash
    {
        return Ok(false);
    }
    let head = storage.canonical_snapshot_head(world)?.head.map(|head| DiscoveryCanonicalHeadV1 {
        snapshot_number: head.snapshot_number,
        manifest_hash: head.manifest_hash,
        epoch: head.epoch,
        sequence: head.sequence,
    });
    if head != challenge.canonical_head {
        return Ok(false);
    }
    let pending = storage.load_membership_promise(world).ok().map(|promise| promise.proposal);
    let pending_hash = pending.as_ref().map(|proposal| proposal.proposal_hash()).transpose()?;
    if pending_hash != challenge.pending_membership_proposal_hash {
        return Ok(false);
    }
    let eligible = membership.members.iter().any(|member| {
        member.peer_id == identity.peer_id() && member.public_key == identity.public_key() && !member.banned
    }) || pending.as_ref().is_some_and(|proposal| {
        proposal.proposed.members.iter().any(|member| {
            member.peer_id == identity.peer_id() && member.public_key == identity.public_key() && !member.banned
        })
    });
    Ok(eligible)
}

#[allow(clippy::too_many_arguments)]
pub fn validate_fresh_discovery_candidate(
    announcement: &WorldAnnouncementV1,
    proof: &DiscoveryMembershipProofV1,
    challenge: &DiscoveryFreshnessChallengeV1,
    votes: &[DiscoveryFreshnessVoteV1],
    verifier_peer_id: PeerId,
    nonce: [u8; 32],
    now_unix_ms: u64,
    replay: &mut DiscoveryFreshnessReplayGuard,
) -> Result<()> {
    verify_world_announcement(announcement, now_unix_ms).map_err(|error| anyhow!(error))?;
    verify_discovery_membership_proof(announcement, proof).map_err(|error| anyhow!(error))?;
    validate_discovery_membership_proof_shape(proof).map_err(|error| anyhow!(error))?;
    verify_discovery_freshness_challenge(announcement, proof, challenge, verifier_peer_id, nonce, now_unix_ms)
        .map_err(|error| anyhow!(error))?;
    for vote in votes {
        verify_discovery_freshness_vote(vote, challenge).map_err(|error| anyhow!(error))?;
    }
    validate_discovery_freshness_quorum(proof, votes).map_err(|error| anyhow!(error))?;
    replay.accept(challenge).map_err(|error| anyhow!(error))?;
    Ok(())
}

async fn prove_candidate_freshness(
    node: &mut DiscoveryNode,
    verifier: &PeerIdentity,
    announcement: &WorldAnnouncementV1,
    locator_peers: &HashSet<TransportPeerId>,
) -> Result<bool> {
    verify_world_announcement(announcement, unix_millis()?).map_err(|error| anyhow!(error))?;
    let query = node.find_world_providers(announcement.world_id);
    let nonce = random_nonce();
    let issued_unix_ms = unix_millis()?;
    let expires_unix_ms =
        checked_discovery_expiry(issued_unix_ms, DISCOVERY_FRESHNESS_MAX_LIFETIME_MS, "freshness challenge")?;
    let announcement_hash = announcement.announcement_hash()?;
    let mut providers = locator_peers.clone();
    let mut applications = HashMap::new();
    let mut context_requested = HashSet::new();
    let mut vote_requested = HashSet::new();
    let mut proof: Option<DiscoveryMembershipProofV1> = None;
    let mut challenge: Option<DiscoveryFreshnessChallengeV1> = None;
    let mut votes = Vec::<DiscoveryFreshnessVoteV1>::new();
    let mut replay = DiscoveryFreshnessReplayGuard::default();

    // Reuse already-authenticated explicit locators from browse/resolve while
    // still accepting additional DHT-discovered world providers. Locator
    // identity never grants authority; proof and quorum verification below do.
    for transport_peer in providers.iter().copied().collect::<Vec<_>>() {
        if let Some(application_peer) = node.application_peer(&transport_peer) {
            applications.insert(transport_peer, application_peer);
        } else {
            let _ = node.dial_peer(transport_peer);
        }
    }
    for (transport_peer, application_peer) in applications.clone() {
        if application_peer == announcement.announcer_peer_id && context_requested.insert(transport_peer) {
            node.send_request(
                &transport_peer,
                WireRequest::DiscoveryFreshnessContext {
                    world_id: announcement.world_id,
                    announcement_hash,
                    verifier_peer_id: verifier.peer_id(),
                    nonce,
                    issued_unix_ms,
                    expires_unix_ms,
                },
            )?;
        }
    }

    let run = timeout(DISCOVERY_FRESHNESS_TIMEOUT, async {
        loop {
            match node.next_event().await? {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for peer in found {
                        if providers.len() >= swarm_core::MAX_DISCOVERY_FRESHNESS_VOTES {
                            break;
                        }
                        if providers.insert(peer) {
                            let _ = node.dial_peer(peer);
                        }
                    }
                }
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query && providers.is_empty() => {
                    break;
                }
                DiscoveryNetworkEvent::ProvidersFailed { query_id, .. }
                    if query_id == query && providers.is_empty() =>
                {
                    break;
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, application_peer }
                    if providers.contains(&transport_peer) =>
                {
                    applications.insert(transport_peer, application_peer);
                    if application_peer == announcement.announcer_peer_id && context_requested.insert(transport_peer) {
                        node.send_request(
                            &transport_peer,
                            WireRequest::DiscoveryFreshnessContext {
                                world_id: announcement.world_id,
                                announcement_hash,
                                verifier_peer_id: verifier.peer_id(),
                                nonce,
                                issued_unix_ms,
                                expires_unix_ms,
                            },
                        )?;
                    }
                    if let Some(active) = &challenge {
                        if vote_requested.insert(transport_peer) {
                            node.send_request(
                                &transport_peer,
                                WireRequest::DiscoveryFreshnessVote(Box::new(active.clone())),
                            )?;
                        }
                    }
                }
                DiscoveryNetworkEvent::Response {
                    transport_peer,
                    response: WireResponse::DiscoveryFreshnessContext(Some(value)),
                    ..
                } => {
                    if applications.get(&transport_peer) != Some(&announcement.announcer_peer_id) {
                        continue;
                    }
                    let candidate = *value;
                    verify_discovery_membership_proof(announcement, &candidate).map_err(|error| anyhow!(error))?;
                    validate_discovery_membership_proof_shape(&candidate).map_err(|error| anyhow!(error))?;
                    let pending_membership_proposal_hash =
                        candidate.pending_membership.as_ref().map(|proposal| proposal.proposal_hash()).transpose()?;
                    let active = DiscoveryFreshnessChallengeV1 {
                        protocol_version: PROTOCOL_VERSION,
                        verifier_peer_id: verifier.peer_id(),
                        nonce,
                        world_id: announcement.world_id,
                        announcement_hash,
                        membership_sequence: announcement.membership_sequence,
                        membership_hash: announcement.membership_hash,
                        pending_membership_proposal_hash,
                        authority_peer_id: announcement.announcer_peer_id,
                        authority_epoch: announcement.authority_epoch,
                        fencing_token: announcement.fencing_token,
                        config_sequence: announcement.config_sequence,
                        config_hash: announcement.config_hash,
                        canonical_head: announcement.canonical_head,
                        issued_unix_ms,
                        expires_unix_ms,
                    };
                    verify_discovery_freshness_challenge(
                        announcement,
                        &candidate,
                        &active,
                        verifier.peer_id(),
                        nonce,
                        unix_millis()?,
                    )
                    .map_err(|error| anyhow!(error))?;
                    proof = Some(candidate);
                    challenge = Some(active.clone());
                    for transport_peer in providers.iter().copied().collect::<Vec<_>>() {
                        if applications.contains_key(&transport_peer) && vote_requested.insert(transport_peer) {
                            let _ = node.send_request(
                                &transport_peer,
                                WireRequest::DiscoveryFreshnessVote(Box::new(active.clone())),
                            );
                        }
                    }
                }
                DiscoveryNetworkEvent::Response {
                    response: WireResponse::DiscoveryFreshnessVote(Some(value)), ..
                } => {
                    let Some(active) = &challenge else { continue };
                    let vote = *value;
                    if verify_discovery_freshness_vote(&vote, active).is_err() {
                        continue;
                    }
                    if votes.len() < swarm_core::MAX_DISCOVERY_FRESHNESS_VOTES
                        && votes.iter().all(|existing| existing.voter_peer_id != vote.voter_peer_id)
                    {
                        votes.push(vote);
                        votes.sort_by_key(|value| value.voter_peer_id);
                    }
                    if let Some(candidate) = &proof {
                        if validate_discovery_freshness_quorum(candidate, &votes).is_ok() {
                            validate_fresh_discovery_candidate(
                                announcement,
                                candidate,
                                active,
                                &votes,
                                verifier.peer_id(),
                                nonce,
                                unix_millis()?,
                                &mut replay,
                            )?;
                            return Ok::<bool, anyhow::Error>(true);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok::<bool, anyhow::Error>(false)
    })
    .await;
    match run {
        Ok(value) => value,
        Err(_) => Ok(false),
    }
}

pub fn add_friend(paths: &DataPaths, peer: &str, public_key_hex: &str, label: &str) -> Result<FriendContactV1> {
    let peer_id = PeerId::from_str(peer).context("invalid friend peer ID")?;
    let public_key = parse_public_key(public_key_hex)?;
    if peer_id_from_public_key(&public_key) != peer_id {
        bail!("friend peer ID does not match the supplied public key");
    }
    let label = label.trim();
    if label.len() > MAX_FRIEND_LABEL_BYTES {
        bail!("friend label is too long");
    }
    let mut store = load_friend_store(paths)?;
    if let Some(existing) = store.friends.iter_mut().find(|friend| friend.peer_id == peer_id.to_string()) {
        if existing.public_key_hex != hex::encode(public_key) {
            bail!("friend identity collision: the peer ID is already stored with another public key");
        }
        existing.label = label.to_owned();
        let existing = existing.clone();
        save_friend_store(paths, &store)?;
        return Ok(existing);
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
    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key())?;
    node.listen("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())?;
    // Warm the actual browse node's direct locators before starting a DHT
    // query. This avoids racing bootstrap/query-created connections against the
    // explicit transport dials and preserves caller order for adversarial first
    // contact. Locator order and identity still grant zero authority.
    let bootstrap_order = add_explicit_locator_addresses(&mut node, bootstrap_addrs)?;
    warm_explicit_locators(&mut node, &bootstrap_order).await?;
    let bootstrap_peers = bootstrap_order.iter().copied().collect::<HashSet<_>>();

    // Explicit bootstrap nodes are bounded untrusted locator candidates. DHT
    // provider discovery augments this set; neither source conveys authority.
    let mut providers = bootstrap_peers.clone();
    let mut requested = HashSet::new();
    let mut pending = HashSet::new();
    for transport_peer in bootstrap_order.iter().copied() {
        if node.application_peer(&transport_peer).is_some() && requested.insert(transport_peer) {
            let request_id =
                node.send_request(&transport_peer, WireRequest::DiscoveryPublic { filter: filter.clone() })?;
            pending.insert(format!("{request_id:?}"));
        }
    }
    if !bootstrap_order.is_empty() {
        node.bootstrap()?;
    }
    let query = node.find_public_providers();
    let mut candidates = Vec::<WorldAnnouncementV1>::new();
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
                        let request_id = node
                            .send_request(&transport_peer, WireRequest::DiscoveryPublic { filter: filter.clone() })?;
                        pending.insert(format!("{request_id:?}"));
                    }
                }
                DiscoveryNetworkEvent::Response { request_id, response, .. } => {
                    pending.remove(&format!("{request_id:?}"));
                    if let WireResponse::DiscoveryWorlds(values) = response {
                        for value in values {
                            if !matches!(value.visibility, WorldVisibilityV1::Public)
                                || !announcement_matches(&value, &filter)
                            {
                                rejected_invalid += 1;
                                continue;
                            }
                            match verify_world_announcement(&value, unix_millis()?) {
                                Ok(()) => {
                                    let hash = value.announcement_hash()?;
                                    if candidates.iter().all(|existing| existing.announcement_hash().ok() != Some(hash))
                                    {
                                        candidates.push(value);
                                    }
                                }
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

    for candidate in candidates {
        match prove_candidate_freshness(&mut node, &identity, &candidate, &bootstrap_peers).await {
            Ok(true) => match replay.accept(&candidate) {
                Ok(()) => {
                    results.insert(candidate.world_id, candidate);
                }
                Err(DiscoveryRecordError::Replay) => rejected_stale += 1,
                Err(_) => rejected_invalid += 1,
            },
            Ok(false) => rejected_stale += 1,
            Err(_) => rejected_invalid += 1,
        }
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
        if network_error.is_some() {
            "network_unavailable"
        } else {
            "provider_unavailable"
        }
    } else if cards.is_empty() {
        if rejected_invalid > 0 || rejected_stale > 0 {
            "partial"
        } else {
            "empty"
        }
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
    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key())?;
    node.listen("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())?;
    // Exact resolve uses the same actual-node readiness discipline as
    // browse. Direct locators are contacted in caller order before DHT
    // augmentation, but every returned candidate still needs the canonical
    // proof and fresh Agent 1 quorum below.
    let bootstrap_order = add_explicit_locator_addresses(&mut node, bootstrap_addrs)?;
    warm_explicit_locators(&mut node, &bootstrap_order).await?;
    let bootstrap_peers = bootstrap_order.iter().copied().collect::<HashSet<_>>();
    // Exact resolve also treats explicit bootstraps only as untrusted locators.
    let mut providers = bootstrap_peers.clone();
    let mut requested = HashSet::new();
    for transport_peer in bootstrap_order.iter().copied() {
        if node.application_peer(&transport_peer).is_some() && requested.insert(transport_peer) {
            node.send_request(&transport_peer, WireRequest::DiscoveryResolve { world_id: world })?;
        }
    }
    if !bootstrap_order.is_empty() {
        node.bootstrap()?;
    }
    let query = node.find_world_providers(world);
    let mut candidates = Vec::<WorldAnnouncementV1>::new();
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
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query && providers.is_empty() => {
                    break
                }
                DiscoveryNetworkEvent::ProvidersFailed { query_id, error } if query_id == query => {
                    detail = Some(error);
                    if providers.is_empty() {
                        break;
                    }
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
                            let value = *value;
                            let hash = value.announcement_hash()?;
                            if candidates.iter().all(|existing| existing.announcement_hash().ok() != Some(hash)) {
                                candidates.push(value);
                            }
                        }
                        Err(DiscoveryRecordError::Expired) => stale = true,
                        Err(_) => invalid = true,
                    }
                }
                DiscoveryNetworkEvent::OutboundFailure { error, .. } => {
                    detail.get_or_insert(error);
                }
                _ => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;
    if let Ok(value) = run {
        value?;
    }
    for candidate in candidates {
        match prove_candidate_freshness(&mut node, &identity, &candidate, &bootstrap_peers).await {
            Ok(true) => {
                result = Some(candidate);
                break;
            }
            Ok(false) => stale = true,
            Err(_) => invalid = true,
        }
    }

    let state = if result.is_some() {
        "found"
    } else if stale {
        "stale"
    } else if invalid {
        "invalid"
    } else if providers.is_empty() {
        if detail.is_some() {
            "network_unavailable"
        } else {
            "provider_unavailable"
        }
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
    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key())?;
    node.listen("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())?;
    add_explicit_bootstraps(&mut node, bootstrap_addrs)?;
    let query = node.find_friend_providers(peer, requester);
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
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query && providers.is_empty() => {
                    break
                }
                DiscoveryNetworkEvent::ProvidersFailed { query_id, error } if query_id == query => {
                    detail = Some(error);
                    if providers.is_empty() {
                        break;
                    }
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, application_peer }
                    if providers.contains(&transport_peer) =>
                {
                    if application_peer != peer {
                        continue;
                    }
                    if requested.insert(transport_peer) {
                        node.send_request(
                            &transport_peer,
                            WireRequest::FriendPresence { expected_peer_id: peer, requester_peer_id: requester, nonce },
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
                DiscoveryNetworkEvent::OutboundFailure { error, .. } => {
                    detail.get_or_insert(error);
                }
                _ => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;
    if let Ok(value) = run {
        value?;
    }

    Ok(report.unwrap_or_else(|| FriendPresenceReportV1 {
        peer_id: peer.to_string(),
        state: if providers.is_empty() && detail.is_some() { "network_unavailable".into() } else { "offline".into() },
        observed_unix_ms: None,
        expires_unix_ms: None,
        detail,
    }))
}

fn accepted_friend_peers(paths: &DataPaths) -> Result<HashSet<PeerId>> {
    load_friend_store(paths)?
        .friends
        .into_iter()
        .map(|friend| PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid"))
        .collect()
}

fn refresh_presence_publications(
    paths: &DataPaths,
    identity: &PeerIdentity,
    node: &mut DiscoveryNode,
    state: &mut PublishedDiscoveryState,
) -> Result<()> {
    let next = accepted_friend_peers(paths)?;
    for requester in state.presence_requesters.difference(&next).copied().collect::<Vec<_>>() {
        node.stop_providing_friend_presence(identity.peer_id(), requester);
    }
    for requester in next.difference(&state.presence_requesters).copied().collect::<Vec<_>>() {
        let _ = node.start_providing_friend_presence(identity.peer_id(), requester)?;
    }
    state.presence_requesters = next;
    Ok(())
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
        if !value.presentation.approximate_region.as_deref().is_some_and(|region| region.eq_ignore_ascii_case(expected))
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

fn add_explicit_locator_addresses(node: &mut DiscoveryNode, values: &[String]) -> Result<Vec<TransportPeerId>> {
    let mut peers = Vec::new();
    let mut seen = HashSet::new();
    for value in values.iter().map(|value| value.trim()).filter(|value| !value.is_empty()) {
        let peer = node.add_bootstrap_address(
            value.parse().with_context(|| format!("invalid discovery bootstrap address: {value}"))?,
        )?;
        if seen.insert(peer) {
            peers.push(peer);
        }
    }
    Ok(peers)
}

async fn warm_explicit_locators(node: &mut DiscoveryNode, peers: &[TransportPeerId]) -> Result<()> {
    if peers.is_empty() {
        return Ok(());
    }
    let expected = peers.iter().copied().collect::<HashSet<_>>();
    let warmup = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        loop {
            if expected.iter().all(|peer| node.application_peer(peer).is_some()) {
                return Ok::<(), anyhow::Error>(());
            }
            match node.next_event().await? {
                DiscoveryNetworkEvent::Disconnected { transport_peer, .. } if expected.contains(&transport_peer) => {
                    let _ = node.dial_peer(transport_peer);
                }
                _ => {}
            }
        }
    })
    .await;
    if let Ok(result) = warmup {
        result?;
    }
    Ok(())
}

fn add_explicit_bootstraps(node: &mut DiscoveryNode, values: &[String]) -> Result<HashSet<TransportPeerId>> {
    let peers = add_explicit_locator_addresses(node, values)?;
    if !peers.is_empty() {
        node.bootstrap()?;
    }
    Ok(peers.into_iter().collect())
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
    bytes.try_into().map_err(|bytes: Vec<u8>| anyhow!("friend public key must be 32 bytes, got {}", bytes.len()))
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

fn next_discovery_sequence(previous: u64, now: u64) -> Result<u64> {
    if now > previous {
        Ok(now)
    } else {
        previous.checked_add(1).context("discovery announcement sequence exhausted")
    }
}

fn checked_discovery_expiry(issued: u64, ttl: u64, label: &str) -> Result<u64> {
    issued.checked_add(ttl).with_context(|| format!("{label} expiry overflow"))
}

fn unix_millis() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_core::{sign_recovery_ballot, sign_recovery_vote, sign_world_config, PeerIdentity};
    use swarm_protocol::{
        AuthorityPolicyV1, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, RecoveryBallotV1, RecoveryVoteV1,
        RuntimeCompatibilityManifestV1, WorldConfigV1, WorldMemberV1, WorldPresentationV1, STORAGE_SCHEMA_VERSION,
    };
    use swarm_storage::WorldMetadataV1;

    #[test]
    fn friend_store_is_cryptographic_and_label_collisions_are_harmless() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let a = PeerIdentity::from_secret_bytes([1; 32]);
        let b = PeerIdentity::from_secret_bytes([2; 32]);
        add_friend(&paths, &a.peer_id().to_string(), &hex::encode(a.public_key()), "Alex").unwrap();
        add_friend(&paths, &b.peer_id().to_string(), &hex::encode(b.public_key()), "Alex").unwrap();
        assert_eq!(list_friends(&paths).unwrap().len(), 2);
        let accepted = accepted_friend_peers(&paths).unwrap();
        assert_eq!(accepted.len(), 2);
        assert!(accepted.contains(&a.peer_id()));
        assert!(accepted.contains(&b.peer_id()));

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
        let mut initial_membership = vec![local.peer_id(), friend.peer_id()];
        initial_membership.sort();
        let genesis = swarm_protocol::WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([7; 32]),
            creation_nonce: [8; 32],
            creator_public_key: local.public_key(),
            initial_membership,
        };
        let world = genesis.world_id().unwrap();
        storage
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "Shared".into(),
                world_id: world,
                genesis,
            })
            .unwrap();

        let mut members = vec![
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
                banned: false,
            },
        ];
        members.sort_by_key(|member| member.peer_id);
        let mut initial = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 0,
            sequence: 0,
            previous_membership_hash: None,
            members,
            authority_peer_id: local.peer_id(),
            authority_public_key: local.public_key(),
            signature: Vec::new(),
        };
        local.sign_membership(&mut initial).unwrap();
        storage.save_membership_record(&initial).unwrap();
        assert_eq!(shared_worlds(&storage, local.peer_id(), friend.peer_id(), friend.public_key()).unwrap().len(), 1);

        let mut banned = initial.clone();
        banned.sequence = 1;
        banned.previous_membership_hash = Some(initial.record_hash().unwrap());
        banned.members.iter_mut().find(|member| member.peer_id == friend.peer_id()).unwrap().banned = true;
        banned.signature.clear();
        local.sign_membership(&mut banned).unwrap();
        storage.save_membership_record(&banned).unwrap();
        assert!(shared_worlds(&storage, local.peer_id(), friend.peer_id(), friend.public_key()).unwrap().is_empty());

        let mut removed = banned.clone();
        removed.sequence = 2;
        removed.previous_membership_hash = Some(banned.record_hash().unwrap());
        removed.members.retain(|member| member.peer_id != friend.peer_id());
        removed.signature.clear();
        local.sign_membership(&mut removed).unwrap();
        storage.save_membership_record(&removed).unwrap();
        assert!(shared_worlds(&storage, local.peer_id(), friend.peer_id(), friend.public_key()).unwrap().is_empty());
    }

    #[test]
    fn durable_recovery_promise_fences_stale_freshness_and_current_majority_recovers() {
        let a = PeerIdentity::from_secret_bytes([31; 32]);
        let b = PeerIdentity::from_secret_bytes([32; 32]);
        let c = PeerIdentity::from_secret_bytes([33; 32]);
        let mut members = [&a, &b, &c]
            .into_iter()
            .map(|identity| WorldMemberV1 {
                peer_id: identity.peer_id(),
                public_key: identity.public_key(),
                authority_eligible: true,
                banned: false,
            })
            .collect::<Vec<_>>();
        members.sort_by_key(|member| member.peer_id);
        let genesis = swarm_protocol::WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([34; 32]),
            creation_nonce: [35; 32],
            creator_public_key: a.public_key(),
            initial_membership: members.iter().map(|member| member.peer_id).collect(),
        };
        let world = genesis.world_id().unwrap();
        let metadata = WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "durable-freshness".into(),
            world_id: world,
            genesis,
        };
        let roots = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
        let stores = roots.iter().map(|root| Storage::open(root.path()).unwrap()).collect::<Vec<_>>();

        let mut initial = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 0,
            sequence: 0,
            previous_membership_hash: None,
            members: members.clone(),
            authority_peer_id: a.peer_id(),
            authority_public_key: a.public_key(),
            signature: Vec::new(),
        };
        a.sign_membership(&mut initial).unwrap();
        let mut config1 = canonical_fixture(&a, world).0;
        sign_world_config(&a, &mut config1).unwrap();
        let mut epoch1 = canonical_fixture(&a, world).1;
        epoch1.signature = a.sign(&epoch1.signing_bytes().unwrap());
        let mut membership1 = initial.clone();
        membership1.epoch = epoch1.epoch_number;
        membership1.sequence = 1;
        membership1.previous_membership_hash = Some(initial.record_hash().unwrap());
        membership1.signature.clear();
        a.sign_membership(&mut membership1).unwrap();

        for store in &stores {
            store.create_world(&metadata).unwrap();
            store.save_membership_record(&initial).unwrap();
            store.save_world_config(&config1).unwrap();
            store.save_epoch_record(&epoch1).unwrap();
            store.save_membership_record(&membership1).unwrap();
        }

        let stale = DiscoveryFreshnessChallengeV1 {
            protocol_version: PROTOCOL_VERSION,
            verifier_peer_id: PeerIdentity::from_secret_bytes([36; 32]).peer_id(),
            nonce: [37; 32],
            world_id: world,
            announcement_hash: Hash32([38; 32]),
            membership_sequence: membership1.sequence,
            membership_hash: membership1.record_hash().unwrap(),
            pending_membership_proposal_hash: None,
            authority_peer_id: a.peer_id(),
            authority_epoch: epoch1.epoch_number,
            fencing_token: epoch1.fencing_token,
            config_sequence: config1.sequence,
            config_hash: config1.config_hash().unwrap(),
            canonical_head: None,
            issued_unix_ms: 1,
            expires_unix_ms: 2,
        };
        assert!(local_state_matches_freshness_challenge(&stores[0], &a, &stale).unwrap());

        let mut ballot = RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            base_epoch: epoch1.epoch_number,
            base_fencing_token: epoch1.fencing_token,
            target_epoch: epoch1.epoch_number.checked_add(1).unwrap(),
            target_fencing_token: epoch1.fencing_token.checked_add(1).unwrap(),
            round: 1,
            candidate_peer_id: b.peer_id(),
            candidate_public_key: b.public_key(),
            base_snapshot_hash: Hash32([39; 32]),
            base_state_hash: epoch1.base_state_hash,
            membership_hash: membership1.record_hash().unwrap(),
            signature: Vec::new(),
        };
        sign_recovery_ballot(&b, &mut ballot).unwrap();
        for (store, voter) in [(&stores[1], &b), (&stores[2], &c)] {
            let mut vote = RecoveryVoteV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                ballot_hash: ballot.ballot_hash().unwrap(),
                base_epoch: ballot.base_epoch,
                target_epoch: ballot.target_epoch,
                round: ballot.round,
                candidate_peer_id: ballot.candidate_peer_id,
                voter_peer_id: voter.peer_id(),
                voter_public_key: voter.public_key(),
                signature: Vec::new(),
            };
            sign_recovery_vote(voter, &mut vote).unwrap();
            assert_eq!(
                store.promise_recovery_ballot(&ballot, &vote).unwrap(),
                swarm_storage::RecoveryPromiseResult::Accepted
            );
        }
        assert!(!local_state_matches_freshness_challenge(&stores[1], &b, &stale).unwrap());
        assert!(!local_state_matches_freshness_challenge(&stores[2], &c, &stale).unwrap());

        let mut epoch2 = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: ballot.target_epoch,
            previous_epoch_hash: None,
            base_state_hash: epoch1.base_state_hash,
            authority_peer_id: b.peer_id(),
            authority_public_key: b.public_key(),
            mode: EpochMode::Quorum,
            fencing_token: ballot.target_fencing_token,
            reason: "durable freshness recovery".into(),
            signature: Vec::new(),
        };
        epoch2.signature = b.sign(&epoch2.signing_bytes().unwrap());
        let mut membership2 = membership1.clone();
        membership2.epoch = epoch2.epoch_number;
        membership2.sequence = membership1.sequence.checked_add(1).unwrap();
        membership2.previous_membership_hash = Some(membership1.record_hash().unwrap());
        membership2.signature.clear();
        b.sign_membership(&mut membership2).unwrap();
        let mut config2 = config1.clone();
        config2.sequence = config1.sequence.checked_add(1).unwrap();
        config2.previous_config_hash = Some(config1.config_hash().unwrap());
        config2.signature.clear();
        sign_world_config(&b, &mut config2).unwrap();
        for store in [&stores[1], &stores[2]] {
            store.save_epoch_record(&epoch2).unwrap();
            store.save_membership_record(&membership2).unwrap();
            store.save_world_config(&config2).unwrap();
            assert!(store.clear_recovery_promise_after_epoch_advance(world, epoch2.epoch_number).unwrap());
        }
        let current = DiscoveryFreshnessChallengeV1 {
            protocol_version: PROTOCOL_VERSION,
            verifier_peer_id: stale.verifier_peer_id,
            nonce: [40; 32],
            world_id: world,
            announcement_hash: Hash32([41; 32]),
            membership_sequence: membership2.sequence,
            membership_hash: membership2.record_hash().unwrap(),
            pending_membership_proposal_hash: None,
            authority_peer_id: b.peer_id(),
            authority_epoch: epoch2.epoch_number,
            fencing_token: epoch2.fencing_token,
            config_sequence: config2.sequence,
            config_hash: config2.config_hash().unwrap(),
            canonical_head: None,
            issued_unix_ms: 3,
            expires_unix_ms: 4,
        };
        assert!(local_state_matches_freshness_challenge(&stores[1], &b, &current).unwrap());
        assert!(local_state_matches_freshness_challenge(&stores[2], &c, &current).unwrap());
        assert!(!local_state_matches_freshness_challenge(&stores[0], &a, &current).unwrap());
        assert!(!local_state_matches_freshness_challenge(&stores[1], &b, &stale).unwrap());
    }

    #[test]
    fn discovery_counter_exhaustion_fails_closed() {
        assert!(next_discovery_sequence(u64::MAX, u64::MAX).is_err());
        assert!(checked_discovery_expiry(u64::MAX, 1, "test").is_err());
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
            membership_sequence: 0,
            membership_hash: Hash32([13; 32]),
            authority_epoch: 1,
            fencing_token: 1,
            canonical_head: None,
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
