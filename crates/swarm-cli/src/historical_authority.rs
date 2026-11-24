//! Read-only collection of signed historical records for a future verified
//! checkpoint replay protocol. A signed chain is NOT a certified authority
//! transition: membership, transfer/recovery/sleep proofs and exact
//! checkpoint boundaries still have to be validated before replay.
use anyhow::{bail, Context, Result};
use swarm_core::{protocol_v2::verify_world_config_signature, verify_signature, verify_snapshot_signature};
use swarm_protocol::{peer_id_from_public_key, EpochMode, EpochRecordV1, WorldConfigV1, WorldId};
use swarm_storage::Storage;

// Bounded independently of peer-supplied record counts. Large or legacy
// histories are explicitly unsupported, never silently truncated.
pub const MAX_HISTORICAL_EPOCHS: usize = 256;
pub const MAX_HISTORICAL_CONFIGS: usize = 256;

/// Follow exact signed predecessor hashes back to a new-world genesis
/// quorum. This neither writes canonical state nor verifies authority
/// transitions; callers must not use its result as a live quorum proof.
pub fn collect_signed_epoch_history(storage: &Storage, world: WorldId) -> Result<Vec<EpochRecordV1>> {
    let metadata = storage.load_world(world)?;
    let creator = peer_id_from_public_key(&metadata.genesis.creator_public_key);
    let accepted = storage.load_epoch_record(world)?;
    let mut record = accepted.clone();
    let mut history = Vec::new();
    loop {
        if history.len() >= MAX_HISTORICAL_EPOCHS {
            bail!("signed epoch history exceeds the supported replay limit");
        }
        record.validate_semantics()?;
        if record.world_id != world {
            bail!("signed epoch belongs to a different world");
        }
        verify_signature(
            record.authority_peer_id,
            record.authority_public_key,
            &record.signing_bytes()?,
            &record.signature,
        )?;
        let previous_hash = record.previous_epoch_hash;
        history.push(record);
        let Some(hash) = previous_hash else { break };
        let previous =
            storage.load_epoch_by_hash(world, hash).context("required signed predecessor epoch is unavailable")?;
        let next = history.last().context("missing successor epoch")?;
        if previous.epoch_number.checked_add(1) != Some(next.epoch_number)
            || previous.fencing_token.checked_add(1) != Some(next.fencing_token)
        {
            bail!("historical epoch does not advance the predecessor exactly once");
        }
        record = previous;
    }
    history.reverse();
    let first = history.first().context("empty signed epoch history")?;
    if first.epoch_number != crate::world_genesis::GENESIS_EPOCH
        || first.fencing_token != 1
        || first.mode != EpochMode::Quorum
        || first.authority_peer_id != creator
        || first.authority_public_key != metadata.genesis.creator_public_key
        || metadata.genesis.initial_membership != [creator]
    {
        bail!("history does not begin at the supported creator-signed genesis quorum");
    }
    let base = storage.load_snapshot(world, 1).context("original signed checkpoint is unavailable")?;
    verify_snapshot_signature(&base)?;
    storage.verify_snapshot(&base)?;
    if base.epoch != first.epoch_number
        || base.sequence != 1
        || base.previous_snapshot_hash.is_some()
        || base.authority_peer_id != creator
        || base.authority_public_key != first.authority_public_key
        || base.state_root != first.base_state_hash
    {
        bail!("initial epoch is not bound to its original verified checkpoint");
    }
    if storage.load_epoch_record(world)? != accepted {
        bail!("accepted epoch changed while collecting signed history");
    }
    Ok(history)
}

/// Collect the signed config hash chain independently. Full replay must
/// additionally bind each config signer to its exact authorized epoch.
pub fn collect_signed_config_history(storage: &Storage, world: WorldId) -> Result<Vec<WorldConfigV1>> {
    let metadata = storage.load_world(world)?;
    let creator = peer_id_from_public_key(&metadata.genesis.creator_public_key);
    let accepted = storage.load_world_config(world)?;
    let mut record = accepted.clone();
    let mut history = Vec::new();
    loop {
        if history.len() >= MAX_HISTORICAL_CONFIGS {
            bail!("signed config history exceeds the supported replay limit");
        }
        verify_world_config_signature(&record)?;
        if record.world_id != world || record.compatibility_fingerprint()? != metadata.genesis.compatibility_fingerprint
        {
            bail!("historical config does not match the original world compatibility");
        }
        let previous_hash = record.previous_config_hash;
        history.push(record);
        let Some(hash) = previous_hash else { break };
        let previous = storage
            .load_world_config_by_hash(world, hash)
            .context("required signed predecessor config is unavailable")?;
        if previous.sequence.checked_add(1) != Some(history.last().context("missing successor config")?.sequence) {
            bail!("historical config skips a predecessor sequence");
        }
        record = previous;
    }
    history.reverse();
    let first = history.first().context("empty signed config history")?;
    if first.sequence != 1
        || first.authority_peer_id != creator
        || first.authority_public_key != metadata.genesis.creator_public_key
    {
        bail!("config history does not begin at the creator-signed configuration");
    }
    if storage.load_world_config(world)? != accepted {
        bail!("accepted config changed while collecting signed history");
    }
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_core::{protocol_v2::sign_world_config, PeerIdentity};
    use swarm_protocol::{
        AuthorityPolicyV1, Hash32, MembershipPolicyV1, RuntimeCompatibilityManifestV1, WorldGenesisV1,
        WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
    };
    use swarm_storage::{SnapshotContext, WorldMetadataV1};

    fn compatibility() -> RuntimeCompatibilityManifestV1 {
        RuntimeCompatibilityManifestV1 {
            minecraft_version: "26.1.2".into(),
            loader_id: "fabric".into(),
            loader_version: "0.19.3".into(),
            swarmcraft_protocol_version: PROTOCOL_VERSION,
            fabric_adapter_version: "test".into(),
            required_server_mods: Vec::new(),
            required_client_mods: Vec::new(),
            datapacks: Vec::new(),
        }
    }

    fn fixture() -> (tempfile::TempDir, Storage, PeerIdentity, EpochRecordV1) {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("data")).unwrap();
        let creator = PeerIdentity::from_secret_bytes([21; 32]);
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "26.1.2".into(),
            fabric_loader_version: "0.19.3".into(),
            compatibility_fingerprint: compatibility().fingerprint().unwrap(),
            creation_nonce: [8; 32],
            creator_public_key: creator.public_key(),
            initial_membership: vec![creator.peer_id()],
        };
        let world = genesis.world_id().unwrap();
        storage
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "historical replay".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        let source = temp.path().join("snapshot");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("level.dat"), b"first checkpoint").unwrap();
        let mut checkpoint = storage
            .snapshot_directory(
                &source,
                SnapshotContext {
                    world,
                    snapshot_number: 1,
                    epoch: 1,
                    sequence: 1,
                    previous_snapshot_hash: None,
                    authority_peer_id: creator.peer_id(),
                    authority_public_key: creator.public_key(),
                },
            )
            .unwrap();
        creator.sign_snapshot(&mut checkpoint).unwrap();
        storage.commit_snapshot(&checkpoint).unwrap();
        let mut epoch = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: 1,
            fencing_token: 1,
            previous_epoch_hash: None,
            base_state_hash: checkpoint.state_root,
            authority_peer_id: creator.peer_id(),
            authority_public_key: creator.public_key(),
            mode: EpochMode::Quorum,
            reason: "genesis".into(),
            signature: Vec::new(),
        };
        epoch.signature = creator.sign(&epoch.signing_bytes().unwrap());
        storage.save_epoch_record(&epoch).unwrap();
        (temp, storage, creator, epoch)
    }

    fn successor(first: &EpochRecordV1, identity: &PeerIdentity) -> EpochRecordV1 {
        let mut record = EpochRecordV1 {
            epoch_number: first.epoch_number + 1,
            fencing_token: first.fencing_token + 1,
            previous_epoch_hash: Some(first.record_hash().unwrap()),
            authority_peer_id: identity.peer_id(),
            authority_public_key: identity.public_key(),
            reason: "historical successor".into(),
            signature: Vec::new(),
            ..first.clone()
        };
        record.signature = identity.sign(&record.signing_bytes().unwrap());
        record
    }

    #[test]
    fn signed_history_walks_exact_records_without_granting_authority() {
        let (temp, storage, creator, first) = fixture();
        let second = successor(&first, &creator);
        let third = successor(&second, &PeerIdentity::from_secret_bytes([22; 32]));
        storage.save_epoch_record(&second).unwrap();
        storage.save_epoch_record(&third).unwrap();
        drop(storage);
        let storage = Storage::open(temp.path().join("data")).unwrap();
        assert_eq!(collect_signed_epoch_history(&storage, first.world_id).unwrap(), vec![first, second, third.clone()]);
        assert_eq!(storage.load_epoch_record(third.world_id).unwrap(), third);
        // The collector deliberately does not grant authority to this
        // successor: no transfer/quorum evidence was provided.
    }

    #[test]
    fn missing_predecessor_and_skipped_fence_fail_closed() {
        let (_temp, storage, creator, first) = fixture();
        let mut next = successor(&first, &creator);
        next.fencing_token += 1;
        next.signature = creator.sign(&next.signing_bytes().unwrap());
        storage.save_epoch_record(&next).unwrap();
        assert!(collect_signed_epoch_history(&storage, first.world_id).is_err());

        let mut third = successor(&next, &creator);
        third.previous_epoch_hash = Some(Hash32([99; 32]));
        third.signature = creator.sign(&third.signing_bytes().unwrap());
        storage.save_epoch_record(&third).unwrap();
        assert!(collect_signed_epoch_history(&storage, first.world_id).is_err());
    }

    #[test]
    fn forged_successor_signature_fails_closed() {
        let (_temp, storage, creator, first) = fixture();
        let mut next = successor(&first, &creator);
        next.signature[0] ^= 1;
        storage.save_epoch_record(&next).unwrap();
        assert!(collect_signed_epoch_history(&storage, first.world_id).is_err());
    }

    #[test]
    fn signed_config_chain_survives_authority_change_and_rejects_corrupt_history() {
        let (_temp, storage, creator, epoch) = fixture();
        let world = epoch.world_id;
        let mut first = WorldConfigV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            sequence: 1,
            previous_config_hash: None,
            compatibility: compatibility(),
            visibility: WorldVisibilityV1::Private,
            authority_policy: AuthorityPolicyV1 { allow_solo_advancement: false, preferred_replication_factor: 3 },
            membership_policy: MembershipPolicyV1::InviteOnly,
            presentation: WorldPresentationV1 {
                name: "original".into(),
                description: String::new(),
                tags: Vec::new(),
                icon_hash: None,
                approximate_region: None,
            },
            authority_peer_id: creator.peer_id(),
            authority_public_key: creator.public_key(),
            signature: Vec::new(),
        };
        sign_world_config(&creator, &mut first).unwrap();
        storage.save_world_config(&first).unwrap();
        let next_authority = PeerIdentity::from_secret_bytes([23; 32]);
        storage.save_epoch_record(&successor(&epoch, &next_authority)).unwrap();
        let first_hash = first.config_hash().unwrap();
        let mut second = WorldConfigV1 { sequence: 2, previous_config_hash: Some(first_hash), ..first.clone() };
        second.presentation.name = "successor".into();
        sign_world_config(&next_authority, &mut second).unwrap();
        storage.save_world_config(&second).unwrap();
        assert_eq!(collect_signed_config_history(&storage, world).unwrap(), vec![first.clone(), second.clone()]);
        // The config hash intentionally covers the unsigned canonical data.
        // Replacing only the archived signature must still be rejected.
        let mut corrupt = first;
        corrupt.signature[0] ^= 1;
        std::fs::write(
            storage.world_dir(world).join("metadata/authority-history").join(format!("config-{first_hash}.postcard")),
            postcard::to_allocvec(&corrupt).unwrap(),
        )
        .unwrap();
        assert!(collect_signed_config_history(&storage, world).is_err());
        assert_eq!(storage.load_world_config(world).unwrap(), second);
    }
}
