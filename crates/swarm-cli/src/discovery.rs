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
    verify_friend_presence, verify_membership_signature, verify_signature, verify_world_announcement,
    verify_world_config_signature, AnnouncementReplayGuard, DataPaths, DiscoveryFreshnessReplayGuard,
    DiscoveryRecordError, PeerIdentity, DISCOVERY_FRESHNESS_MAX_LIFETIME_MS,
};
use swarm_network::{
    generate_transport_key, load_or_create_transport_key, DiscoveryNetworkEvent, DiscoveryNode, TransportPeerId,
    WireRequest, WireResponse, MAX_DISCOVERY_RESULTS,
};
use swarm_protocol::{
    peer_id_from_public_key, DiscoveryCanonicalHeadV1, DiscoveryCompatibilityV1, DiscoveryFilterV1,
    DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1, EpochMode, EpochRecordV1,
    FriendPresenceV1, MembershipPolicyV1, MembershipRecordV1, PeerId, WorldAnnouncementV1, WorldConfigV1, WorldId,
    WorldVisibilityV1, PROTOCOL_VERSION,
};
use swarm_storage::{Storage, StorageError};
use tokio::time::{timeout, timeout_at, Instant, MissedTickBehavior};
use tracing::{debug, info, warn};

pub const DISCOVERY_CAPABILITY: &str = "discovery-v1";
pub const DISCOVERY_LISTEN_DEFAULT: &str = "/ip4/0.0.0.0/udp/0/quic-v1";
pub const WORLD_ANNOUNCEMENT_TTL_MS: u64 = 5 * 60 * 1_000;
pub const FRIEND_PRESENCE_TTL_MS: u64 = 30 * 1_000;
// Reconcile visibility and roster changes promptly. Keep an unchanged
// announcement stable so a client's in-flight nonce challenge can complete.
const DISCOVERY_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const ANNOUNCEMENT_RENEW_BEFORE_MS: u64 = 60_000;
const DISCOVERY_QUERY_TIMEOUT: Duration = Duration::from_secs(8);
const DISCOVERY_FRESHNESS_TIMEOUT: Duration = Duration::from_secs(5);
// Bound the whole verification pass, not just each candidate. Warmup and
// collection are separately bounded at eight seconds apiece. An exhausted
// budget produces a partial directory report, never an unverified listing.
const DISCOVERY_VERIFICATION_BUDGET: Duration = Duration::from_secs(12);
const MAX_DIRECTORY_PROVIDERS: usize = 64;
const MAX_DIRECTORY_CANDIDATES: usize = MAX_DISCOVERY_RESULTS;
// A transient QUIC stream loss must not permanently hide a signed canonical
// provider. Limit retries to the caller's explicit locators and the existing
// overall discovery deadline; retries never bypass authentication or quorum.
const DISCOVERY_EXPLICIT_PROVIDER_RETRIES: u8 = 2;
const DISCOVERY_RETRY_DELAY: Duration = Duration::from_millis(200);
// DiscoveryNode can legitimately need its next two-second connection-proof
// challenge before the other endpoint has authenticated us. Do not burn the
// independent stream-loss retry budget on pre-authentication refusals.
const DISCOVERY_HANDSHAKE_RETRY_DELAY: Duration = Duration::from_millis(2250);
const FRIENDS_FILE_VERSION: u16 = 2;
const MAX_FRIEND_CONTACTS: usize = 256;
const MAX_PRESENCE_PROVIDERS: usize = 64;
const MAX_FRIEND_LABEL_BYTES: usize = 96;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendContactV1 {
    pub peer_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub accepted_unix_ms: u64,
    /// Acceptance alone never grants presence access.
    #[serde(default)]
    pub share_presence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FriendStoreV1 {
    version: u16,
    friends: Vec<FriendContactV1>,
    #[serde(default)]
    presence_enabled: bool,
    #[serde(default)]
    revision: u64,
}

impl Default for FriendStoreV1 {
    fn default() -> Self {
        Self { version: FRIENDS_FILE_VERSION, friends: Vec::new(), presence_enabled: false, revision: 0 }
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
    pub share_presence: bool,
    pub shared_worlds: Vec<SharedWorldV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendSettingsV1 {
    pub own_peer_id: String,
    pub own_public_key_hex: String,
    pub presence_enabled: bool,
    pub contacts: Vec<FriendCardV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendPresenceReportV1 {
    pub peer_id: String,
    /// online, not_observed, network_unavailable, stale, or invalid.
    /// Silence is never proof of offline status.
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
    /// The exact signed configuration which the listing projects.
    pub config_hash: String,
    pub visibility: String,
    pub membership_policy: String,
    /// invite_required for supported V1 worlds. Unsupported membership
    /// policies are not presented as working admission actions.
    pub join_action: String,
    pub announcer_peer_id: String,
    pub issued_unix_ms: u64,
    pub expires_unix_ms: u64,
    /// Local time at successful nonce/quorum verification. This is a
    /// point-in-time directory observation, never a live hosting lease.
    pub verified_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicWorldSearchReportV1 {
    /// ok, empty, provider_unavailable, network_unavailable, or partial.
    pub state: String,
    pub results: Vec<WorldDiscoveryCardV1>,
    pub providers_seen: usize,
    pub providers_responded: usize,
    /// True if any provider, candidate, result or verification budget may
    /// have limited this snapshot. V1 has no remote continuation cursor.
    pub truncated: bool,
    pub rejected_invalid: usize,
    pub rejected_stale: usize,
    /// Number of verified results omitted by this client's local hide list.
    pub hidden_results: usize,
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

pub async fn serve(paths: DataPaths, listen: String, presence_only: bool, bootstrap_addrs: &[String]) -> Result<()> {
    if bootstrap_addrs.len() > MAX_DIRECTORY_PROVIDERS {
        bail!("too many configured discovery locators");
    }
    if presence_only && consented_friend_peers(&paths)?.is_empty() {
        bail!("presence-only service requires global sharing and at least one explicit contact grant");
    }
    let storage = Storage::open(paths.root.clone())?;
    let identity = PeerIdentity::load_or_create(&paths)?;
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()])?;
    let lock_path = paths.identity_dir().join("discovery-provider.lock");
    let mut lock_options = fs::OpenOptions::new();
    lock_options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600);
    }
    let provider_lock = lock_options.open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&provider_lock).context("another discovery publisher owns this data directory")?;
    let transport_path = paths.identity_dir().join("discovery-transport.key");
    let transport_key = load_or_create_transport_key(&transport_path)?;
    let mut node = DiscoveryNode::new(transport_key, hello, identity.network_signing_key())?;
    node.listen(listen.parse().context("invalid discovery listen multiaddress")?)?;
    let locators = add_explicit_locator_addresses(&mut node, bootstrap_addrs)?;
    if !locators.is_empty() {
        node.bootstrap()?;
    }

    // Provider records need a real bound address before DHT publication.
    let first_address = timeout(Duration::from_secs(10), async {
        loop {
            if let DiscoveryNetworkEvent::Listening { address } = node.next_event().await? {
                return Ok::<_, anyhow::Error>(address);
            }
        }
    })
    .await
    .context("discovery publisher listener did not start")??;
    let first_address = node.advertise_local_listener(first_address)?;
    println!("Discovery provider address: {first_address}");

    let mut published = PublishedDiscoveryState::default();
    let mut refresh = tokio::time::interval(DISCOVERY_REFRESH_INTERVAL);
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    if !presence_only {
        let started = std::time::Instant::now();
        let result = refresh_publications(&storage, &identity, &mut node, &mut published);
        crate::support_metrics::observe_result(crate::support_metrics::Metric::DiscoveryRefresh, started, &result);
        result?;
    }
    refresh_presence_publications(&paths, &identity, &mut node, &mut published)?;

    info!(peer = %identity.peer_id(), "discovery service starting");
    loop {
        tokio::select! {
            _ = refresh.tick() => {
                if !presence_only {
                    let started = std::time::Instant::now();
                    let result = refresh_publications(&storage, &identity, &mut node, &mut published);
                    crate::support_metrics::observe_result(crate::support_metrics::Metric::DiscoveryRefresh, started, &result);
                    if let Err(error) = result {
                        warn!(%error, "discovery publication refresh failed");
                    }
                }
                if let Err(error) = refresh_presence_publications(&paths, &identity, &mut node, &mut published) {
                    warn!(%error, "friend presence publication refresh failed");
                    if presence_only { return Err(error); }
                }
                if presence_only && published.presence_requesters.is_empty() {
                    // Exit rather than leave an unnecessary listener alive
                    // after the final grant or global visibility is withdrawn.
                    return Ok(());
                }
            }
            event = node.next_event() => {
                match event? {
                    DiscoveryNetworkEvent::Listening { address } => {
                        if let Ok(address) = node.advertise_local_listener(address) {
                            println!("Discovery provider address: {address}");
                        }
                    }
                    DiscoveryNetworkEvent::InboundRequest { application_peer, request, channel, .. } => {
                        if presence_only && !matches!(request, WireRequest::FriendPresence { .. }) {
                            // No world announcements, membership proof or
                            // freshness votes from a presence-only service.
                            let _ = node.respond(channel, WireResponse::Error {
                                code: "PRESENCE_ONLY".into(), message: "presence-only service".into(),
                            });
                            continue;
                        }
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
        // Never create discovery records for private worlds, even while
        // their membership or other control metadata is inconsistent.
        if !supported_discovery_config(&config) {
            continue;
        }
        let Ok(membership) = storage.load_membership_record(world) else { continue };
        let Ok(epoch) = storage.load_epoch_record(world) else { continue };
        if !discovery_state_matches(storage, &config, &membership, &epoch).unwrap_or(false) {
            continue;
        }
        let local_is_current = membership.members.iter().any(|member| {
            member.peer_id == identity.peer_id() && member.public_key == identity.public_key() && !member.banned
        });
        let pending_membership = match storage.load_membership_promise(world) {
            Ok(promise) => Some(promise.proposal),
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => continue,
        };
        let local_is_pending = pending_membership.as_ref().is_some_and(|proposal| {
            proposal.proposed.members.iter().any(|member| {
                member.peer_id == identity.peer_id() && member.public_key == identity.public_key() && !member.banned
            })
        });
        if !local_is_current && !local_is_pending {
            continue;
        }
        if !discovery_state_matches(storage, &config, &membership, &epoch).unwrap_or(false) {
            continue;
        }
        // DHT hints do not grant authority. Only eligible signed members
        // advertise exact-world provider keys, never private worlds.
        next_worlds.insert(world);
        if !local_is_current
            || epoch.authority_peer_id != identity.peer_id()
            || epoch.authority_public_key != identity.public_key()
        {
            continue;
        }
        if let Some(cached) = state.announcements.get(&world) {
            if cached.expires_unix_ms > now.saturating_add(ANNOUNCEMENT_RENEW_BEFORE_MS)
                && published_announcement_is_current(storage, cached)
            {
                next_announcements.insert(world, cached.clone());
                any_public |= config.visibility == WorldVisibilityV1::Public;
                continue;
            }
        }

        let previous = state.sequences.get(&world).copied().unwrap_or(0);
        let sequence = next_discovery_sequence(previous, now)?;
        state.sequences.insert(world, sequence);
        let mut announcement = WorldAnnouncementV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            presentation: config.presentation.clone(),
            compatibility: discovery_compatibility(&config)?,
            visibility: config.visibility,
            membership_policy: config.membership_policy,
            config_sequence: config.sequence,
            config_hash: config.config_hash()?,
            membership_sequence: membership.sequence,
            membership_hash: membership.record_hash()?,
            authority_epoch: epoch.epoch_number,
            fencing_token: epoch.fencing_token,
            canonical_head: discovery_head(storage, world)?,
            announcement_sequence: sequence,
            issued_unix_ms: now,
            expires_unix_ms: checked_discovery_expiry(now, WORLD_ANNOUNCEMENT_TTL_MS, "discovery")?,
            announcer_peer_id: identity.peer_id(),
            announcer_public_key: identity.public_key(),
            signature: Vec::new(),
        };
        sign_world_announcement(identity, &mut announcement)?;
        verify_world_announcement(&announcement, now).map_err(|error| anyhow!(error))?;
        if !published_announcement_is_current(storage, &announcement) {
            continue;
        }
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

// V1 supports signed, invite-mediated admission only. Neither a public
// listing nor an unsupported future membership policy grants access.
fn supported_discovery_config(config: &WorldConfigV1) -> bool {
    config.visibility != WorldVisibilityV1::Private && config.membership_policy == MembershipPolicyV1::InviteOnly
}

fn discovery_compatibility(config: &WorldConfigV1) -> Result<DiscoveryCompatibilityV1> {
    Ok(DiscoveryCompatibilityV1 {
        minecraft_version: config.compatibility.minecraft_version.clone(),
        loader_id: config.compatibility.loader_id.clone(),
        loader_version: config.compatibility.loader_version.clone(),
        fabric_adapter_version: config.compatibility.fabric_adapter_version.clone(),
        compatibility_fingerprint: config.compatibility.fingerprint()?,
    })
}

fn discovery_head(storage: &Storage, world: WorldId) -> Result<Option<DiscoveryCanonicalHeadV1>> {
    Ok(storage.canonical_snapshot_head(world)?.head.map(|head| DiscoveryCanonicalHeadV1 {
        snapshot_number: head.snapshot_number,
        manifest_hash: head.manifest_hash,
        epoch: head.epoch,
        sequence: head.sequence,
    }))
}

fn discovery_state_matches(
    storage: &Storage,
    config: &WorldConfigV1,
    membership: &MembershipRecordV1,
    epoch: &EpochRecordV1,
) -> Result<bool> {
    let world = config.world_id;
    if !supported_discovery_config(config)
        || storage.discovery_listing_paused(world)?
        || epoch.mode == EpochMode::Solo
        || membership.world_id != world
        || epoch.world_id != world
        || verify_world_config_signature(config).is_err()
        || verify_membership_signature(membership).is_err()
        || verify_signature(
            epoch.authority_peer_id,
            epoch.authority_public_key,
            &epoch.signing_bytes()?,
            &epoch.signature,
        )
        .is_err()
        || membership.epoch != epoch.epoch_number
        || membership.authority_peer_id != epoch.authority_peer_id
        || membership.authority_public_key != epoch.authority_public_key
        || config.authority_peer_id != epoch.authority_peer_id
        || config.authority_public_key != epoch.authority_public_key
        || !membership.members.iter().any(|member| {
            !member.banned
                && member.authority_eligible
                && member.peer_id == epoch.authority_peer_id
                && member.public_key == epoch.authority_public_key
        })
        || !canonical_recovery_promise_allows_freshness(storage, world, epoch.epoch_number, epoch.fencing_token)
    {
        return Ok(false);
    }
    let metadata = storage.load_world(world)?;
    let descriptor = storage.load_world_descriptor(world)?;
    if descriptor.members != membership.members
        || descriptor.compatibility_fingerprint != metadata.genesis.compatibility_fingerprint
        || config.compatibility_fingerprint()? != metadata.genesis.compatibility_fingerprint
        || crate::live_authority::verify_playable_epoch(storage, epoch).is_err()
    {
        return Ok(false);
    }
    // Refuse stale accepted state while a competing authority transition
    // has been durably recorded. A completed predecessor transfer is fine.
    match storage.load_transfer_record(world) {
        Ok(transfer) => {
            swarm_core::verify_transfer_signature(&transfer)?;
            let target = (transfer.next_epoch, transfer.next_fencing_token);
            let current = (epoch.epoch_number, epoch.fencing_token);
            if transfer.phase != swarm_protocol::TransferPhase::Committed
                || target > current
                || (target == current && transfer.to_peer_id != epoch.authority_peer_id)
            {
                return Ok(false);
            }
        }
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(!storage.discovery_listing_paused(world)?
        && storage.load_world_config(world)? == *config
        && storage.load_membership_record(world)? == *membership
        && storage.load_epoch_record(world)? == *epoch
        && storage.load_world_descriptor(world)? == descriptor)
}

// Cached announcements must not outlive signed config, roster, authority
// or checkpoint changes, even before the next DHT refresh.
fn published_announcement_is_current(storage: &Storage, value: &WorldAnnouncementV1) -> bool {
    fn check(storage: &Storage, value: &WorldAnnouncementV1) -> Result<bool> {
        let config = storage.load_world_config(value.world_id)?;
        let membership = storage.load_membership_record(value.world_id)?;
        let epoch = storage.load_epoch_record(value.world_id)?;
        if !discovery_state_matches(storage, &config, &membership, &epoch)? {
            return Ok(false);
        }
        Ok(config.visibility == value.visibility
            && config.membership_policy == value.membership_policy
            && config.presentation == value.presentation
            && discovery_compatibility(&config)? == value.compatibility
            && config.sequence == value.config_sequence
            && config.config_hash()? == value.config_hash
            && membership.sequence == value.membership_sequence
            && membership.record_hash()? == value.membership_hash
            && epoch.epoch_number == value.authority_epoch
            && epoch.fencing_token == value.fencing_token
            && epoch.authority_peer_id == value.announcer_peer_id
            && epoch.authority_public_key == value.announcer_public_key
            && discovery_head(storage, value.world_id)? == value.canonical_head
            && discovery_state_matches(storage, &config, &membership, &epoch)?)
    }
    check(storage, value).unwrap_or(false)
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
            // Optional directory moderation is a local browse decision only.
            // Hold the policy lock through enqueue so completed suppression
            // cannot be overtaken by a new response using stale policy.
            let policy = crate::directory_moderation::lock(paths)
                .and_then(|guard| crate::directory_moderation::read_locked(paths).map(|state| (guard, state)));
            let Ok((_policy_guard, policy)) = policy else {
                node.respond(
                    channel,
                    WireResponse::Error {
                        code: "DIRECTORY_POLICY_UNAVAILABLE".into(),
                        message: "local directory policy could not be verified".into(),
                    },
                )?;
                return Ok(());
            };
            let now = unix_millis()?;
            let limit = usize::from(filter.limit.clamp(1, MAX_DISCOVERY_RESULTS as u16));
            let mut matching: Vec<_> = state
                .announcements
                .values()
                .filter(|value| matches!(value.visibility, WorldVisibilityV1::Public))
                .filter(|value| !policy.suppressed(value.world_id))
                .filter(|value| value.expires_unix_ms > now)
                .filter(|value| announcement_matches(value, &filter))
                .collect();
            // A capped response must not select an arbitrary HashMap subset
            // on each identical request.
            matching.sort_by(|a, b| announcement_order(a, b));
            let values = matching
                .into_iter()
                .filter(|value| published_announcement_is_current(storage, value))
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
                .filter(|value| published_announcement_is_current(storage, value))
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
            if announcement.expires_unix_ms < now
                || !published_announcement_is_current(storage, announcement)
                || announcement.announcement_hash()? != announcement_hash
                || announcement.announcer_peer_id != identity.peer_id()
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            }
            let proof = build_discovery_membership_proof(storage, world_id)?;
            if verify_discovery_membership_proof(announcement, &proof).is_err()
                || !published_announcement_is_current(storage, announcement)
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessContext(None))?;
                return Ok(());
            }
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
            if identity.peer_id() == challenge.authority_peer_id
                && !state.announcements.get(&challenge.world_id).is_some_and(|announcement| {
                    announcement.expires_unix_ms >= now
                        && announcement.announcement_hash().ok() == Some(challenge.announcement_hash)
                        && published_announcement_is_current(storage, announcement)
                })
            {
                node.respond(channel, WireResponse::DiscoveryFreshnessVote(None))?;
                return Ok(());
            }
            let vote = sign_discovery_freshness_vote(identity, &challenge)?;
            if !local_state_matches_freshness_challenge(storage, identity, &challenge)? {
                node.respond(channel, WireResponse::DiscoveryFreshnessVote(None))?;
                return Ok(());
            }
            state.signed_freshness_challenges.insert(replay_key, challenge.expires_unix_ms);
            node.respond(channel, WireResponse::DiscoveryFreshnessVote(Some(Box::new(vote))))?;
        }
        WireRequest::FriendPresence { expected_peer_id, requester_peer_id, nonce } => {
            if application_peer != requester_peer_id {
                bail!("presence requester does not match authenticated peer identity");
            }
            // Serialize a fresh presence response with contact removal.
            // A completed removal must not authorize new signatures, even
            // if the DHT still advertises a stale provider record.
            let _contacts = lock_friend_store(paths)?;
            if !consented_friend_peers(paths)?.contains(&requester_peer_id) {
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
                expires_unix_ms: checked_discovery_expiry(now, FRIEND_PRESENCE_TTL_MS, "presence")?,
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
    let initial = storage.load_genesis_membership(world)?;
    let pending = match storage.load_membership_promise(world) {
        Ok(promise) => Some(promise.proposal),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
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

/// A corrupt or unreadable durable ballot cannot certify a fresh discovery
/// response. A genuinely missing promise has no higher-round constraint.
fn canonical_recovery_promise_allows_freshness(
    storage: &Storage,
    world: WorldId,
    authority_epoch: u64,
    fencing_token: u64,
) -> bool {
    match storage.load_recovery_promise(world) {
        Ok(promise) => {
            (promise.ballot.target_epoch, promise.ballot.target_fencing_token) <= (authority_epoch, fencing_token)
        }
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
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
    if !canonical_recovery_promise_allows_freshness(storage, world, challenge.authority_epoch, challenge.fencing_token)
    {
        return Ok(false);
    }
    let epoch = match storage.load_epoch_record(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let config = match storage.load_world_config(world) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    if epoch.authority_peer_id != challenge.authority_peer_id
        || epoch.epoch_number != challenge.authority_epoch
        || epoch.fencing_token != challenge.fencing_token
        || !discovery_state_matches(storage, &config, &membership, &epoch).unwrap_or(false)
        || config.sequence != challenge.config_sequence
        || config.config_hash()? != challenge.config_hash
    {
        return Ok(false);
    }
    let head = discovery_head(storage, world)?;
    if head != challenge.canonical_head {
        return Ok(false);
    }
    let pending = match storage.load_membership_promise(world) {
        Ok(promise) => Some(promise.proposal),
        Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Ok(false),
    };
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
    if announcement.membership_policy != MembershipPolicyV1::InviteOnly
        || !votes.iter().any(|vote| {
            vote.voter_peer_id == announcement.announcer_peer_id
                && vote.voter_public_key == announcement.announcer_public_key
        })
    {
        bail!("discovery requires a supported invite policy and a fresh vote from the listing authority");
    }
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
    // Include previously discovered providers: their authentication event may
    // already have been consumed by the browse/resolve collection phase.
    // Prefer the announcer, then bound the number of untrusted locators.
    let mut known: Vec<_> = locator_peers.iter().copied().collect();
    known.sort_by_key(|peer| (node.application_peer(peer) != Some(announcement.announcer_peer_id), *peer));
    known.truncate(swarm_core::MAX_DISCOVERY_FRESHNESS_VOTES);
    let mut providers: HashSet<_> = known.into_iter().collect();
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
                            if let Some(application_peer) = node.application_peer(&peer) {
                                applications.insert(peer, application_peer);
                                if application_peer == announcement.announcer_peer_id && context_requested.insert(peer)
                                {
                                    node.send_request(
                                        &peer,
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
                                    if vote_requested.insert(peer) {
                                        node.send_request(
                                            &peer,
                                            WireRequest::DiscoveryFreshnessVote(Box::new(active.clone())),
                                        )?;
                                    }
                                }
                            } else {
                                let _ = node.dial_peer(peer);
                            }
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
                        if votes.iter().any(|vote| {
                            vote.voter_peer_id == announcement.announcer_peer_id
                                && vote.voter_public_key == announcement.announcer_public_key
                        }) && validate_discovery_freshness_quorum(candidate, &votes).is_ok()
                        {
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
    if label.is_empty() || label.len() > MAX_FRIEND_LABEL_BYTES || label.chars().any(char::is_control) {
        bail!("friend label must contain 1 to 96 UTF-8 bytes without control characters");
    }
    let identity = PeerIdentity::load_or_create(paths)?;
    if peer_id == identity.peer_id() {
        bail!("cannot add your own peer identity as a friend");
    }
    let _contacts = lock_friend_store(paths)?;
    let mut store = load_friend_store(paths)?;
    if let Some(existing) = store.friends.iter_mut().find(|friend| friend.peer_id == peer_id.to_string()) {
        if existing.public_key_hex != hex::encode(public_key) {
            bail!("friend identity collision: the peer ID is already stored with another public key");
        }
        existing.label = label.to_owned();
        let existing = existing.clone();
        persist_friend_mutation(paths, &mut store)?;
        return Ok(existing);
    }
    if store.friends.len() >= MAX_FRIEND_CONTACTS {
        bail!("local friend contact limit reached");
    }
    let contact = FriendContactV1 {
        peer_id: peer_id.to_string(),
        public_key_hex: hex::encode(public_key),
        label: label.to_owned(),
        accepted_unix_ms: unix_millis()?,
        share_presence: false,
    };
    store.friends.push(contact.clone());
    store.friends.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    persist_friend_mutation(paths, &mut store)?;
    Ok(contact)
}

pub fn remove_friend(paths: &DataPaths, peer: &str) -> Result<bool> {
    let peer_id = PeerId::from_str(peer).context("invalid friend peer ID")?;
    let _contacts = lock_friend_store(paths)?;
    let mut store = load_friend_store(paths)?;
    let before = store.friends.len();
    store.friends.retain(|friend| friend.peer_id != peer_id.to_string());
    let changed = store.friends.len() != before;
    if changed {
        persist_friend_mutation(paths, &mut store)?;
    }
    Ok(changed)
}

/// Explicit global hide/show switch. Hidden retains per-contact preferences
/// but authorizes no fresh proof or provider publication.
pub fn set_friend_presence_enabled(paths: &DataPaths, enabled: bool) -> Result<()> {
    let _contacts = lock_friend_store(paths)?;
    let mut store = load_friend_store(paths)?;
    store.presence_enabled = enabled;
    persist_friend_mutation(paths, &mut store)
}

/// A separate deliberate grant for an already verified local contact.
pub fn set_friend_presence_sharing(paths: &DataPaths, peer: &str, enabled: bool) -> Result<()> {
    let peer_id = PeerId::from_str(peer).context("invalid friend peer ID")?;
    let _contacts = lock_friend_store(paths)?;
    let mut store = load_friend_store(paths)?;
    let contact = store
        .friends
        .iter_mut()
        .find(|friend| friend.peer_id == peer_id.to_string())
        .context("peer is not an accepted local friend")?;
    contact.share_presence = enabled;
    persist_friend_mutation(paths, &mut store)
}

pub fn friend_settings(paths: &DataPaths) -> Result<FriendSettingsV1> {
    let _contacts = lock_friend_store(paths)?;
    let (mut store, legacy) = load_friend_store_with_migration(paths)?;
    if legacy {
        // Persist the safe migration while holding the same lock as old
        // publishers. They reject the unsupported new store version rather
        // than continuing the old implicit-sharing policy.
        persist_friend_mutation(paths, &mut store)?;
    }
    let identity = PeerIdentity::load_or_create(paths)?;
    let storage = Storage::open(paths.root.clone())?;
    let contacts = friend_cards(&storage, &identity, store.friends)?;
    Ok(FriendSettingsV1 {
        own_peer_id: identity.peer_id().to_string(),
        own_public_key_hex: hex::encode(identity.public_key()),
        presence_enabled: store.presence_enabled,
        contacts,
    })
}

pub fn list_friends(paths: &DataPaths) -> Result<Vec<FriendCardV1>> {
    Ok(friend_settings(paths)?.contacts)
}

fn friend_cards(
    storage: &Storage,
    identity: &PeerIdentity,
    friends: Vec<FriendContactV1>,
) -> Result<Vec<FriendCardV1>> {
    friends
        .into_iter()
        .map(|friend| {
            let peer_id = PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid")?;
            let public_key = parse_public_key(&friend.public_key_hex)?;
            if peer_id_from_public_key(&public_key) != peer_id {
                bail!("stored friend identity is inconsistent");
            }
            Ok(FriendCardV1 {
                shared_worlds: shared_worlds(storage, identity.peer_id(), peer_id, public_key)?,
                peer_id: friend.peer_id,
                public_key_hex: friend.public_key_hex,
                label: if friend.label.trim().is_empty() { "Unnamed contact".into() } else { friend.label },
                accepted_unix_ms: friend.accepted_unix_ms,
                presence: "unknown".into(),
                share_presence: friend.share_presence,
            })
        })
        .collect()
}

fn reserve_explicit_provider_retry(attempts: &mut u8) -> bool {
    if *attempts >= DISCOVERY_EXPLICIT_PROVIDER_RETRIES {
        return false;
    }
    *attempts += 1;
    true
}

fn reserve_matching_explicit_provider_retry(
    pending: &mut HashMap<String, TransportPeerId>,
    retry_attempts: &mut HashMap<TransportPeerId, u8>,
    transport_peer: TransportPeerId,
    request_id: impl std::fmt::Debug,
    explicit_peers: &HashSet<TransportPeerId>,
) -> bool {
    let key = format!("{request_id:?}");
    if pending.get(&key) != Some(&transport_peer) {
        return false;
    }
    pending.remove(&key);
    explicit_peers.contains(&transport_peer)
        && reserve_explicit_provider_retry(retry_attempts.entry(transport_peer).or_default())
}

fn retry_due_providers(
    node: &mut DiscoveryNode,
    queued: &mut HashMap<TransportPeerId, Instant>,
    requested: &mut HashSet<TransportPeerId>,
    pending: &mut HashMap<String, TransportPeerId>,
    request: &WireRequest,
) -> Result<()> {
    let now = Instant::now();
    let due: Vec<_> = queued.iter().filter_map(|(peer, at)| (*at <= now).then_some(*peer)).collect();
    for peer in due {
        if node.is_authenticated(&peer) {
            queued.remove(&peer);
            let id = node.send_request(&peer, request.clone())?;
            pending.insert(format!("{id:?}"), peer);
            requested.insert(peer);
        } else {
            // A dial may fail before it produces an application event. Keep
            // driving the same reserved attempt within the overall deadline.
            // DiscoveryNode suppresses duplicate pending connections; no
            // application request budget is consumed until one is sent.
            queued.insert(peer, now + DISCOVERY_RETRY_DELAY);
            if let Err(error) = node.dial_peer(peer) {
                debug!(transport_peer = %peer, %error, "explicit discovery retry dial failed");
            }
        }
    }
    Ok(())
}

pub async fn search_public_worlds(
    paths: &DataPaths,
    input: DiscoverySearchInputV1,
    bootstrap_addrs: &[String],
) -> Result<PublicWorldSearchReportV1> {
    let filter: DiscoveryFilterV1 = input.into();
    let request = WireRequest::DiscoveryPublic { filter: filter.clone() };
    request.validate_limits()?;
    if bootstrap_addrs.len() > MAX_DIRECTORY_PROVIDERS {
        bail!("too many configured discovery locators");
    }
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
    let mut pending = HashMap::<String, TransportPeerId>::new();
    let mut retry_attempts = HashMap::<TransportPeerId, u8>::new();
    let mut handshake_retry_attempts = HashMap::<TransportPeerId, u8>::new();
    let mut queued_retries = HashMap::new();
    for transport_peer in bootstrap_order.iter().copied() {
        if node.application_peer(&transport_peer).is_some() && requested.insert(transport_peer) {
            let request_id =
                node.send_request(&transport_peer, WireRequest::DiscoveryPublic { filter: filter.clone() })?;
            pending.insert(format!("{request_id:?}"), transport_peer);
        } else if !requested.contains(&transport_peer) {
            queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_RETRY_DELAY);
        }
    }
    if !bootstrap_order.is_empty() {
        node.bootstrap()?;
    }
    let query = node.find_public_providers();
    let mut candidates = Vec::<WorldAnnouncementV1>::new();
    let mut results = HashMap::<WorldId, (WorldAnnouncementV1, u64)>::new();
    let mut candidate_hashes = HashSet::new();
    let mut responded = HashSet::new();
    let mut truncated = false;
    let mut replay = AnnouncementReplayGuard::default();
    let mut rejected_invalid = 0usize;
    let mut rejected_stale = 0usize;
    let mut query_finished = false;
    let mut network_error = None;

    let run = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        let mut retry_tick = tokio::time::interval(DISCOVERY_RETRY_DELAY);
        retry_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            let event = tokio::select! {
                event = node.next_event() => event?,
                _ = retry_tick.tick(), if !queued_retries.is_empty() => {
                    retry_due_providers(&mut node, &mut queued_retries, &mut requested, &mut pending, &request)?;
                    continue;
                }
            };
            match event {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for peer in found {
                        if providers.contains(&peer) {
                            continue;
                        }
                        if providers.len() >= MAX_DIRECTORY_PROVIDERS {
                            truncated = true;
                            break;
                        }
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
                    if !queued_retries.contains_key(&transport_peer) && requested.insert(transport_peer) {
                        let request_id = node
                            .send_request(&transport_peer, WireRequest::DiscoveryPublic { filter: filter.clone() })?;
                        pending.insert(format!("{request_id:?}"), transport_peer);
                    }
                }
                DiscoveryNetworkEvent::Response { transport_peer, request_id, response } => {
                    let key = format!("{request_id:?}");
                    if pending.get(&key) != Some(&transport_peer) {
                        continue;
                    }
                    if let WireResponse::Error { code, message } = &response {
                        network_error.get_or_insert_with(|| format!("{code}: {message}"));
                        // Our receiver can finish verifying the provider's
                        // proof just before the provider finishes verifying
                        // ours. Never interpret that refusal as an empty
                        // directory or skip the existing bounded retry path.
                        if matches!(code.as_str(), "HANDSHAKE_REQUIRED" | "AUTH_CONNECTION_RETRY")
                            && reserve_matching_explicit_provider_retry(
                                &mut pending,
                                &mut handshake_retry_attempts,
                                transport_peer,
                                request_id,
                                &bootstrap_peers,
                            )
                        {
                            requested.remove(&transport_peer);
                            queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_HANDSHAKE_RETRY_DELAY);
                        }
                    }
                    pending.remove(&key);
                    if let WireResponse::DiscoveryWorlds(values) = response {
                        responded.insert(transport_peer);
                        if values.len() >= usize::from(filter.limit) {
                            truncated = true;
                        }
                        for value in values {
                            if candidates.len() >= MAX_DIRECTORY_CANDIDATES {
                                truncated = true;
                                break;
                            }
                            if !matches!(value.visibility, WorldVisibilityV1::Public)
                                || value.membership_policy != MembershipPolicyV1::InviteOnly
                                || !announcement_matches(&value, &filter)
                            {
                                rejected_invalid += 1;
                                continue;
                            }
                            match verify_world_announcement(&value, unix_millis()?) {
                                Ok(()) => {
                                    let hash = value.announcement_hash()?;
                                    if candidate_hashes.insert(hash) {
                                        candidates.push(value);
                                    }
                                }
                                Err(DiscoveryRecordError::Expired) => rejected_stale += 1,
                                Err(_) => rejected_invalid += 1,
                            }
                        }
                    } else if !matches!(response, WireResponse::Error { .. }) {
                        rejected_invalid += 1;
                    }
                }
                DiscoveryNetworkEvent::OutboundFailure { transport_peer, request_id, error } => {
                    network_error.get_or_insert(error);
                    // Only retry a request we actually issued to a caller-
                    // specified locator. DHT-discovered peers and unsolicited
                    // failures receive no retry budget. Signature, membership
                    // and freshness quorum verification below are unchanged.
                    if reserve_matching_explicit_provider_retry(
                        &mut pending,
                        &mut retry_attempts,
                        transport_peer,
                        request_id,
                        &bootstrap_peers,
                    ) {
                        requested.remove(&transport_peer);
                        // Request failure can arrive before the swarm processes
                        // the closing connection. Let it advance before sending
                        // again, so a retry does not attach to that old stream.
                        queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_RETRY_DELAY);
                    }
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

    match run {
        Ok(result) => result?,
        Err(_) => {
            // An unresponsive but known locator is not proof of an empty
            // directory. The request/collection phase has a hard deadline;
            // distinguish expiry from a completed empty response.
            network_error.get_or_insert_with(|| "public discovery query deadline exceeded".into());
        }
    }

    let deadline = Instant::now() + DISCOVERY_VERIFICATION_BUDGET;
    for candidate in candidates {
        if Instant::now() >= deadline {
            truncated = true;
            break;
        }
        match timeout_at(deadline, prove_candidate_freshness(&mut node, &identity, &candidate, &providers)).await {
            Ok(Ok(true)) => match replay.accept(&candidate) {
                Ok(()) => {
                    results.insert(candidate.world_id, (candidate, unix_millis()?));
                }
                Err(DiscoveryRecordError::Replay) => rejected_stale += 1,
                Err(_) => rejected_invalid += 1,
            },
            Ok(Ok(false)) => rejected_stale += 1,
            Ok(Err(_)) => rejected_invalid += 1,
            Err(_) => {
                truncated = true;
                break;
            }
        }
    }

    let now = unix_millis()?;
    let mut values: Vec<_> = results
        .into_values()
        .filter(|(value, _)| {
            if value.expires_unix_ms <= now {
                rejected_stale += 1;
                false
            } else {
                true
            }
        })
        .collect();
    values.sort_by(|(a, _), (b, _)| announcement_order(a, b));
    if values.len() > usize::from(filter.limit) {
        truncated = true;
    }
    values.truncate(usize::from(filter.limit));
    let _policy_guard = crate::directory_moderation::lock(paths)?;
    let policy = crate::directory_moderation::read_locked(paths)?;
    let before_filter = values.len();
    values.retain(|(value, _)| !policy.hidden(value.world_id));
    let hidden_results = before_filter - values.len();
    let cards = values
        .into_iter()
        .map(|(value, verified)| WorldDiscoveryCardV1::from_verified(value, verified))
        .collect::<Vec<_>>();
    let incomplete = truncated
        || responded.len() < providers.len()
        || rejected_invalid > 0
        || rejected_stale > 0
        || network_error.is_some();
    let state = if providers.is_empty() {
        if network_error.is_some() {
            "network_unavailable"
        } else {
            "provider_unavailable"
        }
    } else if cards.is_empty() && hidden_results == 0 {
        // Failed explicit providers are not proof that no public worlds
        // exist. Report incomplete discovery rather than a false empty set.
        if incomplete {
            "partial"
        } else {
            "empty"
        }
    } else if incomplete {
        "partial"
    } else {
        "ok"
    };
    Ok(PublicWorldSearchReportV1 {
        state: state.into(),
        results: cards,
        providers_seen: providers.len(),
        providers_responded: responded.len(),
        truncated,
        rejected_invalid,
        rejected_stale,
        hidden_results,
        detail: network_error,
    })
}

pub async fn resolve_world(
    paths: &DataPaths,
    world: WorldId,
    bootstrap_addrs: &[String],
) -> Result<ResolveWorldReportV1> {
    if bootstrap_addrs.len() > MAX_DIRECTORY_PROVIDERS {
        bail!("too many configured discovery locators");
    }
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
    // Keep the exact outbound request/peer pairing so a late or unsolicited
    // response cannot consume a legitimate retry slot.
    let mut pending = HashMap::<String, TransportPeerId>::new();
    let mut retry_attempts = HashMap::<TransportPeerId, u8>::new();
    let mut handshake_retry_attempts = HashMap::<TransportPeerId, u8>::new();
    let mut queued_retries = HashMap::new();
    let request = WireRequest::DiscoveryResolve { world_id: world };
    for transport_peer in bootstrap_order.iter().copied() {
        if node.application_peer(&transport_peer).is_some() && requested.insert(transport_peer) {
            let request_id = node.send_request(&transport_peer, WireRequest::DiscoveryResolve { world_id: world })?;
            pending.insert(format!("{request_id:?}"), transport_peer);
        } else if !requested.contains(&transport_peer) {
            queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_RETRY_DELAY);
        }
    }
    if !bootstrap_order.is_empty() {
        node.bootstrap()?;
    }
    let query = node.find_world_providers(world);
    let mut candidates = Vec::<WorldAnnouncementV1>::new();
    let mut result = None;
    let mut candidate_hashes = HashSet::new();
    let mut responded = HashSet::new();
    let mut invalid = false;
    let mut stale = false;
    let mut detail = None;
    let mut query_finished = false;

    let run = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        let mut retry_tick = tokio::time::interval(DISCOVERY_RETRY_DELAY);
        retry_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            let event = tokio::select! {
                event = node.next_event() => event?,
                _ = retry_tick.tick(), if !queued_retries.is_empty() => {
                    retry_due_providers(&mut node, &mut queued_retries, &mut requested, &mut pending, &request)?;
                    continue;
                }
            };
            match event {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for peer in found {
                        providers.insert(peer);
                        let _ = node.dial_peer(peer);
                    }
                }
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query => {
                    query_finished = true;
                    if providers.is_empty() {
                        break;
                    }
                }
                DiscoveryNetworkEvent::ProvidersFailed { query_id, error } if query_id == query => {
                    detail = Some(error);
                    query_finished = true;
                    if providers.is_empty() {
                        break;
                    }
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, .. } if providers.contains(&transport_peer) => {
                    if !queued_retries.contains_key(&transport_peer) && requested.insert(transport_peer) {
                        let request_id =
                            node.send_request(&transport_peer, WireRequest::DiscoveryResolve { world_id: world })?;
                        pending.insert(format!("{request_id:?}"), transport_peer);
                    }
                }
                DiscoveryNetworkEvent::Response { transport_peer, request_id, response } => {
                    let key = format!("{request_id:?}");
                    // Do not accept unsolicited, wrong-peer, duplicated or
                    // delayed results from a previous failed request.
                    if pending.get(&key) != Some(&transport_peer) {
                        continue;
                    }
                    if let WireResponse::Error { code, message } = &response {
                        detail.get_or_insert_with(|| format!("{code}: {message}"));
                        if matches!(code.as_str(), "HANDSHAKE_REQUIRED" | "AUTH_CONNECTION_RETRY")
                            && reserve_matching_explicit_provider_retry(
                                &mut pending,
                                &mut handshake_retry_attempts,
                                transport_peer,
                                request_id,
                                &bootstrap_peers,
                            )
                        {
                            requested.remove(&transport_peer);
                            queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_HANDSHAKE_RETRY_DELAY);
                        }
                    }
                    pending.remove(&key);
                    if matches!(&response, WireResponse::DiscoveryResolved(_)) {
                        responded.insert(transport_peer);
                    }
                    if let WireResponse::DiscoveryResolved(Some(value)) = response {
                        if candidates.len() >= MAX_DIRECTORY_CANDIDATES {
                            detail.get_or_insert_with(|| "exact discovery candidate limit reached".into());
                            continue;
                        }
                        if value.world_id != world
                            || matches!(value.visibility, WorldVisibilityV1::Private)
                            || value.membership_policy != MembershipPolicyV1::InviteOnly
                        {
                            invalid = true;
                            continue;
                        }
                        match verify_world_announcement(&value, unix_millis()?) {
                            Ok(()) => {
                                let value = *value;
                                let hash = value.announcement_hash()?;
                                if candidate_hashes.insert(hash) {
                                    candidates.push(value);
                                }
                            }
                            Err(DiscoveryRecordError::Expired) => stale = true,
                            Err(_) => invalid = true,
                        }
                    } else if !matches!(response, WireResponse::DiscoveryResolved(None) | WireResponse::Error { .. }) {
                        invalid = true;
                    }
                }
                DiscoveryNetworkEvent::OutboundFailure { transport_peer, request_id, error } => {
                    detail.get_or_insert(error);
                    // Retry only an actual pending request to an explicit
                    // locator. DHT-only results and duplicate failures cannot
                    // trigger redial amplification.
                    if reserve_matching_explicit_provider_retry(
                        &mut pending,
                        &mut retry_attempts,
                        transport_peer,
                        request_id,
                        &bootstrap_peers,
                    ) {
                        requested.remove(&transport_peer);
                        queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_RETRY_DELAY);
                    }
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
    match run {
        Ok(value) => value?,
        Err(_) => {
            detail.get_or_insert_with(|| "exact discovery query deadline exceeded".into());
        }
    }
    let deadline = Instant::now() + DISCOVERY_VERIFICATION_BUDGET;
    for candidate in candidates {
        if Instant::now() >= deadline {
            detail.get_or_insert_with(|| "exact discovery verification budget exhausted".into());
            break;
        }
        match timeout_at(deadline, prove_candidate_freshness(&mut node, &identity, &candidate, &providers)).await {
            Ok(Ok(true)) => {
                let verified = unix_millis()?;
                if candidate.expires_unix_ms > verified {
                    result = Some(WorldDiscoveryCardV1::from_verified(candidate, verified));
                    break;
                }
                stale = true;
            }
            Ok(Ok(false)) => stale = true,
            Ok(Err(_)) => invalid = true,
            Err(_) => {
                detail.get_or_insert_with(|| "exact discovery verification budget exhausted".into());
                break;
            }
        }
    }

    let state = if result.is_some() {
        "found"
    } else if stale {
        "stale"
    } else if invalid {
        "invalid"
    } else if detail.is_some() || responded.len() < providers.len() {
        // A failed explicit request or provider lookup is not evidence that
        // this world does not exist, even when DHT peers were discovered.
        "network_unavailable"
    } else if providers.is_empty() {
        "provider_unavailable"
    } else {
        "not_found"
    };
    Ok(ResolveWorldReportV1 { state: state.into(), world: result, detail })
}

pub async fn friend_presence(
    paths: &DataPaths,
    peer: PeerId,
    bootstrap_addrs: &[String],
) -> Result<FriendPresenceReportV1> {
    if bootstrap_addrs.len() > MAX_PRESENCE_PROVIDERS {
        bail!("too many configured presence locators");
    }
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
    let locators = add_explicit_locator_addresses(&mut node, bootstrap_addrs)?;
    warm_explicit_locators(&mut node, &locators).await?;
    let mut providers: HashSet<_> = locators.iter().copied().collect();
    if !locators.is_empty() {
        node.bootstrap()?;
    }
    let query = node.find_friend_providers(peer, requester);
    let request = WireRequest::FriendPresence { expected_peer_id: peer, requester_peer_id: requester, nonce };
    let mut requested = HashSet::new();
    let mut pending = HashMap::new();
    let mut network_error = false;
    let mut report = None;
    let mut query_finished = false;
    let mut retry_attempts = HashMap::<TransportPeerId, u8>::new();
    let mut queued_retries = HashMap::new();
    let mut tick = tokio::time::interval(DISCOVERY_RETRY_DELAY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // Only the exact authenticated application peer may be asked for its
    // presence. Locator peers, even when reachable, cannot answer for it.
    for transport in locators.iter().copied() {
        if node.application_peer(&transport) == Some(peer) {
            requested.insert(transport);
            let id = node.send_request(&transport, request.clone())?;
            pending.insert(format!("{id:?}"), transport);
        }
    }
    let run = timeout(DISCOVERY_QUERY_TIMEOUT, async {
        loop {
            let event = tokio::select! {
                event = node.next_event() => event?,
                _ = tick.tick(), if !queued_retries.is_empty() => {
                    let now = Instant::now();
                    let due: Vec<_> = queued_retries.iter().filter(|(_, time)| **time <= now).map(|(peer, _)| *peer).collect();
                    for transport in due {
                        if node.application_peer(&transport) == Some(peer) {
                            queued_retries.remove(&transport);
                            let id = node.send_request(&transport, request.clone())?;
                            pending.insert(format!("{id:?}"), transport);
                        } else {
                            let _ = node.dial_peer(transport);
                        }
                    }
                    continue;
                }
            };
            match event {
                DiscoveryNetworkEvent::ProvidersFound { query_id, providers: found } if query_id == query => {
                    for provider in found {
                        if !providers.contains(&provider) && providers.len() >= MAX_PRESENCE_PROVIDERS {
                            network_error = true;
                            break;
                        }
                        providers.insert(provider);
                        if node.application_peer(&provider) == Some(peer) {
                            if requested.insert(provider) {
                                let id = node.send_request(&provider, request.clone())?;
                                pending.insert(format!("{id:?}"), provider);
                            }
                        } else { let _ = node.dial_peer(provider); }
                    }
                }
                DiscoveryNetworkEvent::ProvidersFinished { query_id } if query_id == query => {
                    query_finished = true;
                }
                DiscoveryNetworkEvent::ProvidersFailed { query_id, .. } if query_id == query => {
                    query_finished = true;
                    network_error = true;
                }
                DiscoveryNetworkEvent::Authenticated { transport_peer, application_peer } if providers.contains(&transport_peer) => {
                    if application_peer == peer && requested.insert(transport_peer) {
                        let id = node.send_request(&transport_peer, request.clone())?;
                        pending.insert(format!("{id:?}"), transport_peer);
                    }
                }
                DiscoveryNetworkEvent::Response { transport_peer, request_id, response } => {
                    let key = format!("{request_id:?}");
                    if pending.get(&key) != Some(&transport_peer)
                        || node.application_peer(&transport_peer) != Some(peer) {
                        continue;
                    }
                    pending.remove(&key);
                    match response {
                        WireResponse::FriendPresence(Some(presence)) => {
                            let now = unix_millis()?;
                            let state = match verify_friend_presence(&presence, peer, requester, nonce, now) {
                                Ok(()) if presence.public_key == public_key && presence.expires_unix_ms > now => "online",
                                Ok(()) if presence.expires_unix_ms <= now => "stale",
                                Ok(()) => "invalid",
                                Err(DiscoveryRecordError::Expired) => "stale",
                                Err(_) => "invalid",
                            };
                            let online = state == "online";
                            report = Some(FriendPresenceReportV1 {
                                peer_id: peer.to_string(), state: state.into(),
                                observed_unix_ms: online.then_some(now),
                                expires_unix_ms: online.then_some(presence.expires_unix_ms.min(now.saturating_add(FRIEND_PRESENCE_TTL_MS))),
                                detail: None,
                            });
                            if online { break; }
                        }
                        WireResponse::FriendPresence(None) => {
                            // A refusal cannot distinguish hidden from not
                            // sharing with this requester. No signed offline.
                            report = Some(FriendPresenceReportV1 {
                                peer_id: peer.to_string(), state: "not_observed".into(),
                                observed_unix_ms: None, expires_unix_ms: None, detail: None,
                            });
                            break;
                        }
                        WireResponse::Error { code, .. }
                            if matches!(code.as_str(), "HANDSHAKE_REQUIRED" | "AUTH_CONNECTION_RETRY")
                                && reserve_explicit_provider_retry(retry_attempts.entry(transport_peer).or_default()) =>
                        {
                            queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_HANDSHAKE_RETRY_DELAY);
                        }
                        _ => { network_error = true; }
                    }
                }
                DiscoveryNetworkEvent::OutboundFailure { transport_peer, request_id, .. } => {
                    let key = format!("{request_id:?}");
                    if pending.get(&key) == Some(&transport_peer) {
                        pending.remove(&key);
                        network_error = true;
                        if reserve_explicit_provider_retry(retry_attempts.entry(transport_peer).or_default()) {
                            queued_retries.insert(transport_peer, Instant::now() + DISCOVERY_RETRY_DELAY);
                        }
                    }
                }
                _ => {}
            }
            if query_finished && pending.is_empty() && queued_retries.is_empty()
                && providers.iter().all(|p| node.application_peer(p).is_some() || providers.is_empty()) {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    }).await;
    match run {
        Ok(result) => result?,
        Err(_) => network_error = true,
    }

    let _contacts = lock_friend_store(paths)?;
    let current = load_friend_store(paths)?;
    // A remove/re-add (even with the same timestamp/label), consent change,
    // or other contact mutation retires every outstanding observation.
    if current.revision != contacts.revision || !current.friends.iter().any(|current| current == contact) {
        bail!("friend contact changed while checking presence");
    }
    Ok(report.unwrap_or_else(|| FriendPresenceReportV1 {
        peer_id: peer.to_string(),
        state: if network_error { "network_unavailable".into() } else { "not_observed".into() },
        observed_unix_ms: None,
        expires_unix_ms: None,
        detail: None,
    }))
}

#[cfg(test)]
fn accepted_friend_peers(paths: &DataPaths) -> Result<HashSet<PeerId>> {
    load_friend_store(paths)?
        .friends
        .into_iter()
        .map(|friend| PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid"))
        .collect()
}

fn consented_friend_peers(paths: &DataPaths) -> Result<HashSet<PeerId>> {
    let store = load_friend_store(paths)?;
    if !store.presence_enabled {
        return Ok(HashSet::new());
    }
    store
        .friends
        .into_iter()
        .filter(|friend| friend.share_presence)
        .map(|friend| PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid"))
        .collect()
}

fn refresh_presence_publications(
    paths: &DataPaths,
    identity: &PeerIdentity,
    node: &mut DiscoveryNode,
    state: &mut PublishedDiscoveryState,
) -> Result<()> {
    let _contacts = lock_friend_store(paths)?;
    let consent = consented_friend_peers(paths);
    // On invalid/unreadable local state withdraw every hint we previously
    // published. A stale remote DHT record can never authorize a proof.
    let next = consent.as_ref().cloned().unwrap_or_default();
    for requester in state.presence_requesters.difference(&next).copied().collect::<Vec<_>>() {
        node.stop_providing_friend_presence(identity.peer_id(), requester);
        state.presence_requesters.remove(&requester);
    }
    consent?;
    for requester in next.difference(&state.presence_requesters).copied().collect::<Vec<_>>() {
        let _ = node.start_providing_friend_presence(identity.peer_id(), requester)?;
        state.presence_requesters.insert(requester);
    }
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

fn announcement_order(a: &WorldAnnouncementV1, b: &WorldAnnouncementV1) -> std::cmp::Ordering {
    a.presentation
        .name
        .to_lowercase()
        .cmp(&b.presentation.name.to_lowercase())
        .then_with(|| a.world_id.cmp(&b.world_id))
}

impl WorldDiscoveryCardV1 {
    fn from_verified(value: WorldAnnouncementV1, verified_unix_ms: u64) -> Self {
        let (membership_policy, join_action) = match value.membership_policy {
            MembershipPolicyV1::Open => ("open", "unsupported"),
            MembershipPolicyV1::ApprovalRequired => ("approval_required", "unsupported"),
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
            config_hash: value.config_hash.to_string(),
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
            verified_unix_ms,
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
            // `DiscoveryNode::next_event` consumes raw outbound dial errors
            // internally. If an explicit locator's first dial loses a race
            // during startup, waiting only for a surfaced network event can
            // therefore strand that locator until this whole warmup times out.
            // Periodically re-drive every unauthenticated explicit peer. The
            // node suppresses duplicate pending dials, and the outer timeout
            // keeps unreachable locators non-fatal and bounded.
            for peer in expected.iter().copied() {
                if node.application_peer(&peer).is_none() {
                    let _ = node.dial_peer(peer);
                }
            }
            if let Ok(event) = timeout(Duration::from_millis(250), node.next_event()).await {
                if let DiscoveryNetworkEvent::Disconnected { transport_peer, .. } = event? {
                    if expected.contains(&transport_peer) {
                        let _ = node.dial_peer(transport_peer);
                    }
                }
            }
        }
    })
    .await;
    if let Ok(result) = warmup {
        result?;
    }
    Ok(())
}

fn load_friend_store(paths: &DataPaths) -> Result<FriendStoreV1> {
    load_friend_store_with_migration(paths).map(|(store, _)| store)
}

fn load_friend_store_with_migration(paths: &DataPaths) -> Result<(FriendStoreV1, bool)> {
    let path = friends_path(paths);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((FriendStoreV1::default(), false)),
        Err(error) => return Err(error).with_context(|| format!("could not read {}", path.display())),
    };
    let mut store: FriendStoreV1 = serde_json::from_slice(&bytes).context("friend store is malformed")?;
    // V1 acceptance previously implied sharing. Migrate conservatively:
    // never infer the new explicit grant from old contact membership.
    let legacy = store.version == 1;
    if legacy {
        store.version = FRIENDS_FILE_VERSION;
        store.presence_enabled = false;
        store.revision = 0;
        for friend in &mut store.friends {
            friend.share_presence = false;
        }
    } else if store.version != FRIENDS_FILE_VERSION {
        bail!("unsupported friend store version {}", store.version);
    }
    store.friends.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    if store.friends.windows(2).any(|pair| pair[0].peer_id == pair[1].peer_id) {
        bail!("duplicate peer in friend store");
    }
    for friend in &store.friends {
        let peer = PeerId::from_str(&friend.peer_id).context("stored friend peer ID is invalid")?;
        let public_key = parse_public_key(&friend.public_key_hex)?;
        if peer_id_from_public_key(&public_key) != peer
            || friend.peer_id != peer.to_string()
            || friend.public_key_hex != hex::encode(public_key)
        {
            bail!("stored friend peer ID does not match its public key");
        }
    }
    Ok((store, legacy))
}

// All writers hold the same advisory lock across their read-modify-write.
// A concurrent label update cannot restore a removed contact from a stale
// copy. The discovery provider also holds it for a fresh presence response.
fn lock_friend_store(paths: &DataPaths) -> Result<fs::File> {
    paths.ensure()?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(paths.root.join("friends.lock"))?;
    fs2::FileExt::lock_exclusive(&file).context("cannot serialize contact store access")?;
    Ok(file)
}

fn persist_friend_mutation(paths: &DataPaths, store: &mut FriendStoreV1) -> Result<()> {
    store.revision = store.revision.checked_add(1).context("friend store revision exhausted")?;
    save_friend_store(paths, store)
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

    #[test]
    fn transient_public_provider_retry_budget_is_bounded() {
        let mut retries = 0;
        assert!(reserve_explicit_provider_retry(&mut retries));
        assert!(reserve_explicit_provider_retry(&mut retries));
        assert!(!reserve_explicit_provider_retry(&mut retries));
        assert_eq!(retries, DISCOVERY_EXPLICIT_PROVIDER_RETRIES);
    }

    #[test]
    fn discovery_retry_fault_injection_rejects_duplicate_wrong_peer_and_dht_failures() {
        // This is the same state transition invoked by both real event loops.
        // Synthetic fault injection complements the authenticated QUIC
        // connection-loss test in discovery_network_freshness.rs.
        let canonical = generate_transport_key().public().to_peer_id();
        let impostor = generate_transport_key().public().to_peer_id();
        let dht_only = generate_transport_key().public().to_peer_id();
        let explicit_peers = HashSet::from([canonical, impostor]);
        let mut pending = HashMap::new();
        let mut attempts = HashMap::new();

        pending.insert(format!("{:?}", "req-1"), canonical);
        assert!(!reserve_matching_explicit_provider_retry(
            &mut pending,
            &mut attempts,
            impostor,
            "req-1",
            &explicit_peers,
        ));
        assert!(reserve_matching_explicit_provider_retry(
            &mut pending,
            &mut attempts,
            canonical,
            "req-1",
            &explicit_peers,
        ));
        assert!(!reserve_matching_explicit_provider_retry(
            &mut pending,
            &mut attempts,
            canonical,
            "req-1",
            &explicit_peers,
        ));
        pending.insert(format!("{:?}", "req-2"), canonical);
        assert!(reserve_matching_explicit_provider_retry(
            &mut pending,
            &mut attempts,
            canonical,
            "req-2",
            &explicit_peers,
        ));
        pending.insert(format!("{:?}", "req-3"), canonical);
        assert!(!reserve_matching_explicit_provider_retry(
            &mut pending,
            &mut attempts,
            canonical,
            "req-3",
            &explicit_peers,
        ));
        assert_eq!(attempts[&canonical], DISCOVERY_EXPLICIT_PROVIDER_RETRIES);

        pending.insert(format!("{:?}", "dht-1"), dht_only);
        assert!(!reserve_matching_explicit_provider_retry(
            &mut pending,
            &mut attempts,
            dht_only,
            "dht-1",
            &explicit_peers,
        ));
        assert!(!attempts.contains_key(&dht_only));
        assert!(pending.is_empty());
    }
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
    fn legacy_contacts_migrate_hidden_and_require_two_explicit_grants() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let peer = PeerIdentity::from_secret_bytes([71; 32]);
        fs::write(
            friends_path(&paths),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "friends": [{
                    "peer_id": peer.peer_id().to_string(),
                    "public_key_hex": hex::encode(peer.public_key()),
                    "label": "Existing contact",
                    "accepted_unix_ms": 1,
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        let id = peer.peer_id().to_string();
        let initial = friend_settings(&paths).unwrap();
        assert!(!initial.presence_enabled);
        assert!(!initial.contacts[0].share_presence);
        let persisted: serde_json::Value = serde_json::from_slice(&fs::read(friends_path(&paths)).unwrap()).unwrap();
        assert_eq!(persisted["version"], 2);
        assert_eq!(persisted["presence_enabled"], false);
        assert_eq!(persisted["friends"][0]["share_presence"], false);
        assert!(consented_friend_peers(&paths).unwrap().is_empty());
        set_friend_presence_sharing(&paths, &id, true).unwrap();
        assert!(consented_friend_peers(&paths).unwrap().is_empty());
        set_friend_presence_enabled(&paths, true).unwrap();
        assert_eq!(consented_friend_peers(&paths).unwrap(), HashSet::from([peer.peer_id()]));
        assert_eq!(load_friend_store(&paths).unwrap().version, 2);
        set_friend_presence_enabled(&paths, false).unwrap();
        assert!(consented_friend_peers(&paths).unwrap().is_empty());
        assert!(friend_settings(&paths).unwrap().contacts[0].share_presence);
        set_friend_presence_enabled(&paths, true).unwrap();
        set_friend_presence_sharing(&paths, &id, false).unwrap();
        assert!(consented_friend_peers(&paths).unwrap().is_empty());
    }

    #[test]
    fn contact_removal_and_readdition_never_restore_old_consent_or_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let peer = PeerIdentity::from_secret_bytes([72; 32]);
        let id = peer.peer_id().to_string();
        let key = hex::encode(peer.public_key());
        add_friend(&paths, &id, &key, "Friend").unwrap();
        set_friend_presence_enabled(&paths, true).unwrap();
        set_friend_presence_sharing(&paths, &id, true).unwrap();
        let old = load_friend_store(&paths).unwrap();
        remove_friend(&paths, &id).unwrap();
        add_friend(&paths, &id, &key, "Friend").unwrap();
        let new = load_friend_store(&paths).unwrap();
        assert!(new.revision > old.revision);
        assert!(!new.friends[0].share_presence);
        assert!(consented_friend_peers(&paths).unwrap().is_empty());
        assert!(set_friend_presence_sharing(
            &paths,
            &PeerIdentity::from_secret_bytes([73; 32]).peer_id().to_string(),
            true
        )
        .is_err());
    }

    #[test]
    fn duplicate_consent_and_invalid_friend_store_fail_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let peer = PeerIdentity::from_secret_bytes([74; 32]);
        add_friend(&paths, &peer.peer_id().to_string(), &hex::encode(peer.public_key()), "Friend").unwrap();
        set_friend_presence_enabled(&paths, true).unwrap();
        set_friend_presence_sharing(&paths, &peer.peer_id().to_string(), true).unwrap();
        let mut store = load_friend_store(&paths).unwrap();
        let mut duplicate = store.friends[0].clone();
        duplicate.share_presence = false;
        store.friends.push(duplicate);
        save_friend_store(&paths, &store).unwrap();
        assert!(consented_friend_peers(&paths).is_err());
    }

    #[tokio::test]
    async fn removed_contact_cannot_be_probed_and_does_not_remove_same_label_contact() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let a = PeerIdentity::from_secret_bytes([1; 32]);
        let b = PeerIdentity::from_secret_bytes([2; 32]);
        for peer in [&a, &b] {
            add_friend(&paths, &peer.peer_id().to_string(), &hex::encode(peer.public_key()), "Alex").unwrap();
        }
        assert!(remove_friend(&paths, &a.peer_id().to_string()).unwrap());
        assert!(!remove_friend(&paths, &a.peer_id().to_string()).unwrap());
        let reopened = DataPaths::from_root(tmp.path());
        assert_eq!(accepted_friend_peers(&reopened).unwrap(), HashSet::from([b.peer_id()]));
        let error = friend_presence(&reopened, a.peer_id(), &[]).await.unwrap_err();
        assert!(error.to_string().contains("not an accepted local friend"));
        assert_eq!(list_friends(&reopened).unwrap()[0].peer_id, b.peer_id().to_string());
    }

    #[test]
    fn concurrent_contact_mutations_preserve_other_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(tmp.path());
        let old = PeerIdentity::from_secret_bytes([50; 32]);
        add_friend(&paths, &old.peer_id().to_string(), &hex::encode(old.public_key()), "Old").unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| assert!(remove_friend(&paths, &old.peer_id().to_string()).unwrap()));
            for seed in 1..=16 {
                let paths = &paths;
                scope.spawn(move || {
                    let peer = PeerIdentity::from_secret_bytes([seed; 32]);
                    add_friend(paths, &peer.peer_id().to_string(), &hex::encode(peer.public_key()), "Same label")
                        .unwrap();
                });
            }
        });
        let contacts = accepted_friend_peers(&paths).unwrap();
        assert_eq!(contacts.len(), 16);
        assert!(!contacts.contains(&old.peer_id()));
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
        assert!(storage.save_membership_record(&removed).is_err(), "a committed ban cannot be erased in V1");
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
            compatibility_fingerprint: canonical_fixture(&a, WorldId::default()).0.compatibility_fingerprint().unwrap(),
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
            store
                .save_world_descriptor(&swarm_protocol::WorldDescriptorV1 {
                    protocol_version: PROTOCOL_VERSION,
                    world_id: world,
                    compatibility_fingerprint: metadata.genesis.compatibility_fingerprint,
                    members: members.clone(),
                    preferred_replication_factor: 2,
                })
                .unwrap();
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

        let mut cached_public = sample_announcement(&b, WorldVisibilityV1::Public);
        cached_public.world_id = world;
        cached_public.presentation = config2.presentation.clone();
        cached_public.compatibility = discovery_compatibility(&config2).unwrap();
        cached_public.config_sequence = config2.sequence;
        cached_public.config_hash = config2.config_hash().unwrap();
        cached_public.membership_sequence = membership2.sequence;
        cached_public.membership_hash = membership2.record_hash().unwrap();
        cached_public.authority_epoch = epoch2.epoch_number;
        cached_public.fencing_token = epoch2.fencing_token;
        sign_world_announcement(&b, &mut cached_public).unwrap();
        assert!(published_announcement_is_current(&stores[1], &cached_public));
        for field in 0..5 {
            let mut stale_cache = cached_public.clone();
            match field {
                0 => stale_cache.membership_hash = Hash32([89; 32]),
                1 => stale_cache.authority_epoch -= 1,
                2 => stale_cache.fencing_token -= 1,
                3 => stale_cache.presentation.description = "not signed in config".into(),
                _ => {
                    stale_cache.canonical_head = Some(DiscoveryCanonicalHeadV1 {
                        snapshot_number: 1,
                        manifest_hash: Hash32([90; 32]),
                        epoch: 1,
                        sequence: 1,
                    })
                }
            }
            sign_world_announcement(&b, &mut stale_cache).unwrap();
            assert!(!published_announcement_is_current(&stores[1], &stale_cache));
        }

        // Local unlisting takes effect before a publisher refreshes its
        // DHT hints or replaces the cached announcement.
        stores[1].set_discovery_listing_paused(world, true).unwrap();
        assert!(stores[1].discovery_listing_paused(world).unwrap());
        assert!(!published_announcement_is_current(&stores[1], &cached_public));
        assert!(!local_state_matches_freshness_challenge(&stores[1], &b, &current).unwrap());
        assert_eq!(stores[1].load_world_config(world).unwrap(), config2);
        stores[1].set_discovery_listing_paused(world, false).unwrap();
        assert!(published_announcement_is_current(&stores[1], &cached_public));

        let pause_path = stores[1].world_dir(world).join("metadata/discovery-listing.paused");
        fs::write(&pause_path, b"invalid marker").unwrap();
        assert!(!published_announcement_is_current(&stores[1], &cached_public));
        assert!(!local_state_matches_freshness_challenge(&stores[1], &b, &current).unwrap());
        assert!(stores[1].set_discovery_listing_paused(world, false).is_err());
        stores[1].set_discovery_listing_paused(world, true).unwrap();
        stores[1].set_discovery_listing_paused(world, false).unwrap();

        let mut private = config2.clone();
        private.sequence += 1;
        private.previous_config_hash = Some(config2.config_hash().unwrap());
        private.visibility = WorldVisibilityV1::Private;
        sign_world_config(&b, &mut private).unwrap();
        stores[1].save_world_config(&private).unwrap();
        assert!(!published_announcement_is_current(&stores[1], &cached_public));
        let mut private_challenge = current;
        private_challenge.config_sequence = private.sequence;
        private_challenge.config_hash = private.config_hash().unwrap();
        assert!(!local_state_matches_freshness_challenge(&stores[1], &b, &private_challenge).unwrap());
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

#[cfg(test)]
mod recovery_freshness_fail_closed_tests {
    use super::*;

    #[test]
    fn unreadable_promise_cannot_prove_discovery_freshness() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path()).unwrap();
        let world = WorldId([0xb1; 32]);
        assert!(canonical_recovery_promise_allows_freshness(&storage, world, 1, 1));

        let metadata = storage.world_dir(world).join("metadata");
        fs::create_dir_all(&metadata).unwrap();
        let path = metadata.join("recovery-promise.postcard");
        let corrupt = b"unreadable higher-round durable promise";
        fs::write(&path, corrupt).unwrap();

        assert!(!canonical_recovery_promise_allows_freshness(&storage, world, 1, 1));
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_promise_symlink_cannot_prove_discovery_freshness() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path()).unwrap();
        let world = WorldId([0xb2; 32]);
        let metadata = storage.world_dir(world).join("metadata");
        fs::create_dir_all(&metadata).unwrap();
        let path = metadata.join("recovery-promise.postcard");
        symlink(temp.path().join("missing-durable-vote"), &path).unwrap();

        assert!(!canonical_recovery_promise_allows_freshness(&storage, world, 1, 1));
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    }
}
