from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one replacement target, found {count}")
    path.write_text(text.replace(old, new, 1))


wire = Path("crates/swarm-network/src/wire.rs")
replace_once(
    wire,
    "impl WireRequest {\n    pub fn validate_limits(&self) -> Result<(), WireLimitError> {",
    """impl WireRequest {
    /// Return the canonical world whose current membership is required before
    /// this request may be dispatched by the replication daemon.
    ///
    /// This match is intentionally exhaustive. Adding a new wire request must
    /// make an explicit authorization decision instead of silently inheriting
    /// an unsafe default.
    pub fn membership_world_id(&self) -> Option<WorldId> {
        match self {
            Self::Hello(_)
            | Self::Ping { .. }
            | Self::JoinRequest(_)
            | Self::DiscoveryPublic { .. }
            | Self::DiscoveryResolve { .. }
            | Self::FriendPresence { .. } => None,
            Self::WorldStatus { world_id }
            | Self::WorldDescriptor { world_id }
            | Self::MissingBlobs { world_id, .. }
            | Self::BlobChunk { world_id, .. }
            | Self::HostCapability { world_id } => Some(*world_id),
            Self::LeaveRequest(request) => Some(request.world_id),
            Self::SnapshotManifest(manifest) => Some(manifest.world_id),
            Self::ReplicaAck(ack) => Some(ack.world_id),
            Self::Membership(record) => Some(record.world_id),
            Self::Epoch(record) => Some(record.world_id),
            Self::AuthorityTransfer(transfer) => Some(transfer.world_id),
            Self::LeaseGrant(lease) => Some(lease.world_id),
            Self::Sleep(record) => Some(record.world_id),
            Self::RecoveryBallot(ballot) => Some(ballot.world_id),
            Self::RecoveryEpoch { record, .. } => Some(record.world_id),
            Self::WorldConfig(config) => Some(config.world_id),
            Self::SoloBranch(branch) => Some(branch.world_id),
        }
    }

    pub fn validate_limits(&self) -> Result<(), WireLimitError> {""",
)

daemon = Path("crates/swarm-cli/src/daemon.rs")
replace_once(
    daemon,
    """    let identity = context.identity;
    let storage = context.storage;
    match request {
""",
    """    let identity = context.identity;
    let storage = context.storage;
    if let Some(world_id) = request.membership_world_id() {
        authorize_member(storage, world_id, application_peer)?;
    }
    match request {
""",
)

replace_once(
    daemon,
    """fn authorize_member(storage: &Storage, world: WorldId, peer: PeerId) -> Result<()> {
    let descriptor = storage.load_world_descriptor(world)?;
    let member = descriptor.member(peer).context(\"peer is not an authorized member of this world\")?;
    if member.banned {
""",
    """fn authorize_member(storage: &Storage, world: WorldId, peer: PeerId) -> Result<()> {
    let descriptor = storage.load_world_descriptor(world)?;
    let member = descriptor.member(peer).context(\"peer is not an authorized member of this world\")?;
    if swarm_protocol::peer_id_from_public_key(&member.public_key) != peer {
        return Err(anyhow!(\"world membership public key does not match peer identity\"));
    }
    if member.banned {
""",
)
