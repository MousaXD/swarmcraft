//! Read-only recovery choices and explicitly scoped data/evidence exports.
//! A conflict branch is evidence, never a certified authority transition.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};
use swarm_core::{verify_snapshot_signature, PeerIdentity};
use swarm_protocol::{
    DiscoveryMembershipProofV1, Hash32, SnapshotManifestV1, SoloBranchV1, WorldId, MAX_REPLAY_CHECKPOINTS,
    PROTOCOL_VERSION,
};
use swarm_storage::{Storage, StorageError};

const MAX_CONFLICT_BRANCHES: usize = 256;
const CONFLICT_SET_DOMAIN: &[u8] = b"swarmcraft/local-recovery-conflict-set/v1\0";

#[derive(Debug, Serialize)]
pub struct CheckpointChoice {
    pub checkpoint_hash: String,
    pub snapshot_number: String,
    pub epoch: String,
    pub sequence: String,
    pub state_root: String,
    pub signer: String,
    pub files: usize,
    pub latest: bool,
}

#[derive(Debug, Serialize)]
pub struct ConflictChoice {
    pub branch_hash: String,
    pub base_checkpoint: String,
    pub head_checkpoint: String,
    pub head_epoch: String,
    pub head_sequence: String,
    pub state_root: String,
    pub signer: String,
    // Evidence alone does NOT establish that any peer can supply these files.
    pub local_history: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RecoveryReview {
    pub world_id: String,
    pub evidence: &'static str,
    pub checkpoint_history: &'static str,
    pub checkpoint_error: Option<String>,
    pub checkpoints: Vec<CheckpointChoice>,
    pub conflict_digest: Option<String>,
    pub conflicts: Vec<ConflictChoice>,
}

fn accepted_membership(storage: &Storage, local: &PeerIdentity, world: WorldId) -> Result<DiscoveryMembershipProofV1> {
    let proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: storage.load_world(world)?.genesis,
        initial_membership: storage.load_genesis_membership(world)?,
        current_membership: storage.load_membership_record(world)?,
        membership_certificates: storage.load_membership_certificate_chain(world)?,
        pending_membership: None,
    };
    crate::live_authority::verify_genesis_anchored_membership(&proof)?;
    if !proof
        .current_membership
        .members
        .iter()
        .any(|member| !member.banned && member.peer_id == local.peer_id() && member.public_key == local.public_key())
    {
        bail!("recovery review requires a locally admitted member");
    }
    let descriptor = storage.load_world_descriptor(world)?;
    if descriptor.members != proof.current_membership.members
        || descriptor.compatibility_fingerprint != proof.genesis.compatibility_fingerprint
        || storage.first_contact_replay_pinned(world)?
    {
        bail!("local membership or compatibility is not synchronized");
    }
    Ok(proof)
}

fn verified_branches(storage: &Storage, world: WorldId) -> Result<BTreeMap<Hash32, SoloBranchV1>> {
    let mut branches = storage.list_solo_conflicts(world)?;
    // An active solo record can be useful recovery evidence even before the
    // peer has observed the competing branch. Never discard it.
    match storage.load_solo_branch(world) {
        Ok(branch) => branches.push(branch),
        Err(StorageError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if branches.len() > MAX_CONFLICT_BRANCHES {
        bail!("too many retained conflict records to review safely");
    }
    let mut exact = BTreeMap::new();
    for branch in branches {
        swarm_core::verify_solo_branch_signature(&branch)?;
        if branch.world_id != world || branch.head_epoch < branch.base_epoch {
            bail!("invalid retained solo branch ancestry");
        }
        let hash = branch.branch_hash()?;
        if let Some(previous) = exact.insert(hash, branch.clone()) {
            if previous != branch {
                bail!("conflicting encodings for one retained branch");
            }
        }
    }
    Ok(exact)
}

fn digest(branches: &BTreeMap<Hash32, SoloBranchV1>) -> Result<Option<Hash32>> {
    if branches.is_empty() {
        return Ok(None);
    }
    Ok(Some(Hash32::from_domain_bytes(CONFLICT_SET_DOMAIN, &postcard::to_allocvec(branches)?)))
}

pub(crate) fn conflict_digest(storage: &Storage, world: WorldId) -> Result<Option<Hash32>> {
    digest(&verified_branches(storage, world)?)
}

/// Only a signed, exact local base-to-head suffix may be exported as files.
/// This is data authentication, NOT a proof that the solo writer obtained
/// quorum, that the base is canonical globally, or that the branch may host.
fn local_branch_history(
    proof: &DiscoveryMembershipProofV1,
    branch: &SoloBranchV1,
    manifests: &[SnapshotManifestV1],
) -> Result<Option<Vec<SnapshotManifestV1>>> {
    let eligible = proof.current_membership.members.iter().any(|member| {
        !member.banned
            && member.authority_eligible
            && member.peer_id == branch.authority_peer_id
            && member.public_key == branch.authority_public_key
    });
    if !eligible {
        // Historical, removed or unproven writer: retain signed evidence but
        // do not claim that this device can authenticate a branch data export.
        return Ok(None);
    }
    let mut base = None;
    let mut head = None;
    for (index, manifest) in manifests.iter().enumerate() {
        let hash = manifest.manifest_hash()?;
        if hash == branch.base_snapshot_hash {
            base = Some(index);
        }
        if hash == branch.head_snapshot_hash {
            head = Some(index);
        }
    }
    let (Some(base), Some(head)) = (base, head) else {
        return Ok(None);
    };
    if base > head {
        bail!("branch head precedes its signed base");
    }
    let chain = &manifests[base..=head];
    let first = &chain[0];
    let last = chain.last().context("branch has no head")?;
    if first.epoch != branch.base_epoch
        || last.epoch != branch.head_epoch
        || last.sequence != branch.head_sequence
        || last.state_root != branch.state_hash
        || last.authority_peer_id != branch.authority_peer_id
        || last.authority_public_key != branch.authority_public_key
    {
        bail!("local manifests disagree with signed branch boundary");
    }
    for (index, manifest) in chain.iter().enumerate() {
        verify_snapshot_signature(manifest)?;
        if manifest.world_id != branch.world_id {
            bail!("branch contains a different world");
        }
        if index != 0 {
            let previous = &chain[index - 1];
            if manifest.previous_snapshot_hash != Some(previous.manifest_hash()?)
                || previous.snapshot_number.checked_add(1) != Some(manifest.snapshot_number)
                || previous.sequence.checked_add(1) != Some(manifest.sequence)
                || manifest.epoch < previous.epoch
                || manifest.epoch > previous.epoch.saturating_add(1)
                || manifest.authority_peer_id != branch.authority_peer_id
                || manifest.authority_public_key != branch.authority_public_key
            {
                bail!("branch suffix does not follow its exact signed writer and base");
            }
        }
    }
    Ok(Some(chain.to_vec()))
}

fn bounded_manifests(storage: &Storage, world: WorldId) -> Result<Vec<SnapshotManifestV1>> {
    let manifests = storage.list_snapshots(world)?;
    if manifests.len() > MAX_REPLAY_CHECKPOINTS {
        bail!("local checkpoint history exceeds supported review bounds");
    }
    Ok(manifests)
}

pub fn review(storage: &Storage, local: &PeerIdentity, world: WorldId) -> Result<RecoveryReview> {
    let membership = accepted_membership(storage, local, world)?;
    let branches = verified_branches(storage, world)?;
    let manifests = bounded_manifests(storage, world)?;
    let mut conflicts = Vec::new();
    for (hash, branch) in &branches {
        let local_history = match local_branch_history(&membership, branch, &manifests) {
            Ok(Some(_)) => "matching_signed_manifests",
            Ok(None) => "evidence_only",
            Err(_) => "unverified_local_history",
        };
        conflicts.push(ConflictChoice {
            branch_hash: hash.to_string(),
            base_checkpoint: branch.base_snapshot_hash.to_string(),
            head_checkpoint: branch.head_snapshot_hash.to_string(),
            head_epoch: branch.head_epoch.to_string(),
            head_sequence: branch.head_sequence.to_string(),
            state_root: branch.state_hash.to_string(),
            signer: branch.authority_peer_id.to_string(),
            local_history,
        });
    }
    let (checkpoints, checkpoint_error) =
        match crate::historical_authority::verified_local_checkpoint_history(storage, local, world) {
            Ok((_, history)) => {
                let choices = history
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(index, manifest)| {
                        Ok(CheckpointChoice {
                            checkpoint_hash: manifest.manifest_hash()?.to_string(),
                            snapshot_number: manifest.snapshot_number.to_string(),
                            epoch: manifest.epoch.to_string(),
                            sequence: manifest.sequence.to_string(),
                            state_root: manifest.state_root.to_string(),
                            signer: manifest.authority_peer_id.to_string(),
                            files: manifest.entries.len(),
                            latest: index + 1 == history.len(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                (choices, None)
            }
            Err(error) => (Vec::new(), Some(format!("{error:#}"))),
        };
    if accepted_membership(storage, local, world)? != membership
        || verified_branches(storage, world)? != branches
        || storage.list_snapshots(world)? != manifests
    {
        bail!("local recovery history changed during inspection; refresh the review");
    }
    Ok(RecoveryReview {
        world_id: world.to_string(),
        evidence: "local_signed_history_not_live_authority",
        checkpoint_history: if checkpoint_error.is_none() {
            "verified_history_files_checked_on_action"
        } else {
            "unavailable"
        },
        checkpoint_error,
        checkpoints,
        conflict_digest: digest(&branches)?.map(|hash| hash.to_string()),
        conflicts,
    })
}

#[derive(Debug, Serialize)]
pub struct RecoveryExportReceipt {
    pub world_id: String,
    pub checkpoint_hash: String,
    pub branch_hash: Option<String>,
    pub destination: PathBuf,
    pub evidence: &'static str,
    pub files_exported: bool,
    pub manifest_count: usize,
}

/// Export to a brand-new directory, never overwrite an existing save, and
/// never accept a path beneath the SwarmCraft data root. The incomplete
/// marker remains on an ambiguous failure; no success receipt is returned.
fn create_export_destination(storage: &Storage, destination: &Path) -> Result<PathBuf> {
    let name = destination.file_name().context("export requires a new directory name")?;
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().context("export parent directory must already exist")?;
    let root = storage.root().canonicalize()?;
    if parent.starts_with(&root) {
        bail!("export destination must be outside the original SwarmCraft data root");
    }
    let destination = parent.join(name);
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&destination).context("export destination must be a new, non-existing directory")?;
    // create_dir above is the exclusive ownership operation. The marker is
    // written before any data, and is not a Minecraft world directory.
    write_new(&destination.join(".swarmcraft-export-incomplete"), b"incomplete recovery export\n")?;
    Ok(destination)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn finish_export(receipt: &RecoveryExportReceipt) -> Result<()> {
    write_new(&receipt.destination.join("recovery-receipt.json"), &serde_json::to_vec_pretty(receipt)?)?;
    // Sync before removing the marker. Keep a visible incomplete marker on
    // failure; no destructive recovery of that user-owned folder is attempted.
    #[cfg(unix)]
    fs::File::open(&receipt.destination)?.sync_all()?;
    fs::remove_file(receipt.destination.join(".swarmcraft-export-incomplete"))?;
    #[cfg(unix)]
    {
        fs::File::open(&receipt.destination)?.sync_all()?;
        fs::File::open(receipt.destination.parent().context("missing export parent")?)?.sync_all()?;
    }
    Ok(())
}

fn write_manifests(destination: &Path, manifests: &[SnapshotManifestV1]) -> Result<()> {
    let dir = destination.join("signed-manifests");
    fs::create_dir(&dir)?;
    for manifest in manifests {
        write_new(&dir.join(format!("{:020}.postcard", manifest.snapshot_number)), &postcard::to_allocvec(manifest)?)?;
    }
    #[cfg(unix)]
    fs::File::open(&dir)?.sync_all()?;
    Ok(())
}

pub fn export_checkpoint(
    storage: &Storage,
    local: &PeerIdentity,
    world: WorldId,
    checkpoint: Hash32,
    acknowledged_conflicts: Option<Hash32>,
    destination: &Path,
) -> Result<RecoveryExportReceipt> {
    let source = crate::historical_authority::verify_selected_recovery_source(
        storage,
        local,
        world,
        checkpoint,
        acknowledged_conflicts,
    )?;
    let destination = create_export_destination(storage, destination)?;
    storage.restore_snapshot(&source.manifest, &destination.join("world"))?;
    // Signed records and manifest chain are portable evidence, not a
    // network replay envelope. Only selected head file bytes are exported.
    let (history, manifests) = crate::historical_authority::verified_local_checkpoint_history(storage, local, world)?;
    write_manifests(&destination, &manifests)?;
    write_new(&destination.join("local-certified-history.postcard"), &postcard::to_allocvec(&history)?)?;
    write_new(&destination.join("signed-config.postcard"), &postcard::to_allocvec(&source.config)?)?;
    source.ensure_unchanged(storage)?;
    let receipt = RecoveryExportReceipt {
        world_id: world.to_string(),
        checkpoint_hash: checkpoint.to_string(),
        branch_hash: None,
        destination,
        evidence: "verified_local_checkpoint_not_authority",
        files_exported: true,
        manifest_count: manifests.len(),
    };
    finish_export(&receipt)?;
    Ok(receipt)
}

pub fn export_conflict(
    storage: &Storage,
    local: &PeerIdentity,
    world: WorldId,
    branch_hash: Hash32,
    acknowledged_conflicts: Hash32,
    evidence_only: bool,
    destination: &Path,
) -> Result<RecoveryExportReceipt> {
    let membership = accepted_membership(storage, local, world)?;
    let branches = verified_branches(storage, world)?;
    if digest(&branches)? != Some(acknowledged_conflicts) {
        bail!("conflict evidence changed; inspect the exact branches before exporting");
    }
    let branch = branches.get(&branch_hash).context("selected signed branch is not retained")?;
    let manifests = bounded_manifests(storage, world)?;
    let chain = if evidence_only {
        None
    } else {
        let chain = local_branch_history(&membership, branch, &manifests)?
            .context("branch files are not available on this device; export evidence only or export files from the branch holder")?;
        storage.verify_snapshot(chain.last().context("missing branch head")?)?;
        Some(chain)
    };
    let destination = create_export_destination(storage, destination)?;
    write_new(&destination.join("signed-branch.postcard"), &postcard::to_allocvec(branch)?)?;
    write_new(&destination.join("local-membership-proof.postcard"), &postcard::to_allocvec(&membership)?)?;
    if let Some(chain) = &chain {
        storage.restore_snapshot(chain.last().context("missing branch head")?, &destination.join("world"))?;
        write_manifests(&destination, chain)?;
    }
    if accepted_membership(storage, local, world)? != membership
        || verified_branches(storage, world)? != branches
        || storage.list_snapshots(world)? != manifests
    {
        bail!("conflict evidence changed during export; no completed export is confirmed");
    }
    let receipt = RecoveryExportReceipt {
        world_id: world.to_string(),
        checkpoint_hash: branch.head_snapshot_hash.to_string(),
        branch_hash: Some(branch_hash.to_string()),
        destination,
        evidence: if evidence_only {
            "signed_branch_evidence_only"
        } else {
            "signed_branch_head_files_not_canonical_authority"
        },
        files_exported: chain.is_some(),
        manifest_count: chain.as_ref().map_or(0, Vec::len),
    };
    finish_export(&receipt)?;
    Ok(receipt)
}
