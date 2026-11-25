use crate::{
    transaction::{canonical_file_exists, durable_atomic_write, durable_remove, read_canonical_file},
    Storage, StorageError,
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_protocol::{
    Hash32, JoinRequestV1, LeaveRequestV1, MembershipRecordV1, PeerId, WorldDescriptorV1, WorldId,
    MAX_INVITE_LIFETIME_MS,
};

/// Local diagnostic only. Never a membership or authorization record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinDenialReason {
    Revoked,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingJoinDenial {
    world: WorldId,
    issuer: PeerId,
    invite_hash: Hash32,
    request_nonce: [u8; 32],
    reason: JoinDenialReason,
}

impl Storage {
    pub fn save_world_descriptor(&self, descriptor: &WorldDescriptorV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(descriptor.world_id)?;
        let original_member_count = descriptor.members.len();
        let mut descriptor = descriptor.clone();
        descriptor.normalize();
        if descriptor.members.len() != original_member_count {
            return Err(StorageError::WorldMetadataMismatch);
        }
        descriptor.validate_semantics()?;
        if self.first_contact_replay_pinned_locked(descriptor.world_id)? {
            return if self.load_world_descriptor(descriptor.world_id)? == descriptor {
                Ok(())
            } else {
                Err(StorageError::WorldMetadataMismatch)
            };
        }
        let bytes = serde_json::to_vec_pretty(&descriptor)?;
        durable_atomic_write(&self.world_protocol_path(descriptor.world_id, "descriptor.json"), &bytes)
    }

    pub fn load_world_descriptor(&self, world: WorldId) -> Result<WorldDescriptorV1, StorageError> {
        self.ensure_stale_publication_clear(world)?;
        let path = self.world_protocol_path(world, "descriptor.json");
        let bytes = read_canonical_file(&path)?;
        let descriptor: WorldDescriptorV1 = serde_json::from_slice(&bytes)?;
        if descriptor.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        descriptor.validate_semantics()?;
        self.ensure_stale_publication_clear(world)?;
        Ok(descriptor)
    }

    pub fn save_membership_record(&self, record: &MembershipRecordV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(record.world_id)?;
        record.validate_semantics()?;
        let membership_path = self.world_protocol_path(record.world_id, "membership.postcard");
        if canonical_file_exists(&membership_path)? {
            let current = self.load_membership_record(record.world_id)?;
            if record == &current {
                return self.archive_membership_locked(&current);
            }
            if self.first_contact_replay_pinned_locked(record.world_id)? {
                return Err(StorageError::WorldMetadataMismatch);
            }
            if !record.preserves_banned_members(&current)
                || self
                    .pending_device_retirement_locked(record.world_id)?
                    .is_some_and(|promise| promise.proposal.previous != current || promise.proposal.proposed != *record)
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
            let transition =
                swarm_protocol::MembershipProposalV1 { previous: current.clone(), proposed: record.clone() };
            // Do not install the retirement staged by an operator without
            // the exact durable certificate. The daemon verifies all
            // certificate signatures and both quorums before persisting it.
            if transition.retired_device().is_some()
                && self.load_membership_certificate(record.world_id)?.proposal != transition
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
            let expected_sequence =
                current.sequence.checked_add(1).ok_or(StorageError::CounterExhausted("membership sequence"))?;
            if record.sequence != expected_sequence
                || record.previous_membership_hash != Some(current.record_hash()?)
                || record.epoch < current.epoch
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
            let accepted_epoch = match self.load_epoch_record(record.world_id) {
                Ok(epoch) => Some(epoch),
                Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            if let Some(epoch) = accepted_epoch.as_ref() {
                if record.epoch != epoch.epoch_number
                    || record.authority_peer_id != epoch.authority_peer_id
                    || record.authority_public_key != epoch.authority_public_key
                {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            } else if record.epoch != current.epoch
                || record.authority_peer_id != current.authority_peer_id
                || record.authority_public_key != current.authority_public_key
            {
                return Err(StorageError::WorldMetadataMismatch);
            }
            // Preserve the exact signed predecessor before replacing it.
            self.archive_membership_locked(&current)?;
        } else {
            if self.first_contact_replay_pinned_locked(record.world_id)? {
                return Err(StorageError::WorldMetadataMismatch);
            }
            let certified_bootstrap = self.certified_membership_bootstrap(record)?;
            if !certified_bootstrap && (record.sequence != 0 || record.previous_membership_hash.is_some()) {
                return Err(StorageError::WorldMetadataMismatch);
            }
            let accepted_epoch = match self.load_epoch_record(record.world_id) {
                Ok(epoch) => Some(epoch),
                Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            if let Some(epoch) = accepted_epoch.as_ref() {
                if record.epoch != epoch.epoch_number
                    || record.authority_peer_id != epoch.authority_peer_id
                    || record.authority_public_key != epoch.authority_public_key
                {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            } else if record.epoch != 0 && !certified_bootstrap {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        let bytes = postcard::to_allocvec(record)?;
        self.archive_membership_locked(record)?;
        durable_atomic_write(&membership_path, &bytes)
    }

    pub(crate) fn archive_membership_locked(&self, record: &MembershipRecordV1) -> Result<(), StorageError> {
        let path = self
            .world_protocol_path(record.world_id, &format!("membership-history/{}.postcard", record.record_hash()?));
        let bytes = postcard::to_allocvec(record)?;
        if canonical_file_exists(&path)? {
            if read_canonical_file(&path)? != bytes {
                return Err(StorageError::WorldMetadataMismatch);
            }
        } else {
            durable_atomic_write(&path, &bytes)?;
        }
        // Keep the original signed record across same-voter epoch changes.
        if record.sequence == 0 && record.previous_membership_hash.is_none() {
            let genesis_path = self.world_protocol_path(record.world_id, "membership-genesis.postcard");
            if canonical_file_exists(&genesis_path)? {
                if read_canonical_file(&genesis_path)? != bytes {
                    return Err(StorageError::WorldMetadataMismatch);
                }
            } else {
                durable_atomic_write(&genesis_path, &bytes)?;
            }
        }
        Ok(())
    }

    /// Read exact historical membership evidence. Never replace a corrupt
    /// record with the current voter list or infer one from an unsigned file.
    pub fn load_membership_by_hash(&self, world: WorldId, hash: Hash32) -> Result<MembershipRecordV1, StorageError> {
        let path = self.world_protocol_path(world, &format!("membership-history/{hash}.postcard"));
        let record: MembershipRecordV1 = match read_canonical_file(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes)?,
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                // Compatibility for older data: the current record and
                // certificate endpoints are already durable signed evidence.
                let current = self.load_membership_record(world)?;
                if current.record_hash()? == hash {
                    return Ok(current);
                }
                for certificate in self.load_membership_certificate_chain(world)? {
                    for record in [certificate.proposal.previous, certificate.proposal.proposed] {
                        if record.record_hash()? == hash {
                            return Ok(record);
                        }
                    }
                }
                return Err(StorageError::Io { path, source });
            }
            Err(error) => return Err(error),
        };
        if record.world_id != world || record.record_hash()? != hash {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    pub fn load_genesis_membership(&self, world: WorldId) -> Result<MembershipRecordV1, StorageError> {
        let path = self.world_protocol_path(world, "membership-genesis.postcard");
        let record: MembershipRecordV1 = match read_canonical_file(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes)?,
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => self
                .load_membership_certificate_chain(world)?
                .first()
                .map(|certificate| certificate.proposal.previous.clone())
                .unwrap_or(self.load_membership_record(world)?),
            Err(error) => return Err(error),
        };
        if record.world_id != world || record.sequence != 0 || record.previous_membership_hash.is_some() {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        Ok(record)
    }

    fn certified_membership_bootstrap(&self, record: &MembershipRecordV1) -> Result<bool, StorageError> {
        let certificate_path = self.world_protocol_path(record.world_id, "membership-certificate.postcard");
        let pending_join_path = self.world_protocol_path(record.world_id, "pending-join.postcard");
        let certificate_present = canonical_file_exists(&certificate_path)?;
        let pending_join_present = canonical_file_exists(&pending_join_path)?;
        if !certificate_present || !pending_join_present {
            return Ok(false);
        }

        let certificate = self.load_membership_certificate(record.world_id)?;
        let pending_join = self.load_pending_join(record.world_id)?;
        let metadata = self.load_world(record.world_id)?;
        if !certificate.proposal.validate_shape()? {
            return Ok(false);
        }

        Ok(certificate.proposal.proposed == *record
            && pending_join.world_id == record.world_id
            && pending_join.invite.world_id == record.world_id
            && pending_join.invite.genesis == metadata.genesis
            && pending_join.invite.inviter_peer_id == certificate.proposal.previous.authority_peer_id
            && pending_join.invite.inviter_public_key == certificate.proposal.previous.authority_public_key
            && certificate.proposal.proposed.members.iter().any(|member| member == &pending_join.joining_member))
    }

    pub fn load_membership_record(&self, world: WorldId) -> Result<MembershipRecordV1, StorageError> {
        self.ensure_stale_publication_clear(world)?;
        let path = self.world_protocol_path(world, "membership.postcard");
        let bytes = read_canonical_file(&path)?;
        let record: MembershipRecordV1 = postcard::from_bytes(&bytes)?;
        if record.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        record.validate_semantics()?;
        self.ensure_stale_publication_clear(world)?;
        Ok(record)
    }

    pub fn save_pending_join(&self, request: &JoinRequestV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(request.world_id)?;
        request.validate_semantics()?;
        let bytes = postcard::to_allocvec(request)?;
        durable_atomic_write(&self.world_protocol_path(request.world_id, "pending-join.postcard"), &bytes)
    }

    /// A fresh signed token can replace an expired/revoked pending token
    /// only for the exact same local member role and issuer. This is not
    /// cancellation of a membership vote, certificate, or replay anchor.
    /// The application verifies both complete signed requests independently.
    pub fn replace_denied_pending_join(
        &self,
        previous: &JoinRequestV1,
        replacement: &JoinRequestV1,
    ) -> Result<(), StorageError> {
        let world = previous.world_id;
        let _guard = self.lock_world_transaction(world)?;
        replacement.validate_semantics()?;
        if self.load_pending_join(world)? != *previous
            || replacement.world_id != world
            || replacement.joining_member != previous.joining_member
            || replacement.invite.genesis != previous.invite.genesis
            || replacement.invite.inviter_peer_id != previous.invite.inviter_peer_id
            || replacement.invite.inviter_public_key != previous.invite.inviter_public_key
            || replacement.invite == previous.invite
            || replacement.nonce == previous.nonce
            || self.load_world(world)?.genesis != previous.invite.genesis
            || self.first_contact_replay_pinned_locked(world)?
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        for name in [
            "membership.postcard",
            "membership-promise.postcard",
            "membership-certificate.postcard",
            "epoch.postcard",
            "completed-join.postcard",
            "pending-leave.postcard",
        ] {
            if canonical_file_exists(&self.world_protocol_path(world, name))? {
                return Err(StorageError::WorldMetadataMismatch);
            }
        }
        let head = self.load_or_initialize_canonical_head_locked(world)?;
        self.validate_no_orphan_manifests_locked(world, head.head)?;
        if head.head.is_some() {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let now: u64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| StorageError::InviteExpired)?
            .as_millis()
            .try_into()
            .map_err(|_| StorageError::InviteExpired)?;
        if replacement.invite.expires_unix_ms <= now
            || replacement.invite.expires_unix_ms - now > MAX_INVITE_LIFETIME_MS
        {
            return Err(StorageError::InviteExpired);
        }
        if previous.invite.expires_unix_ms > now && self.pending_join_denial(previous)?.is_none() {
            return Err(StorageError::WorldMetadataMismatch);
        }
        let descriptor = self.load_world_descriptor(world)?;
        if descriptor.members.len() != 1
            || !descriptor.members.iter().any(|member| {
                !member.banned
                    && member.peer_id == previous.invite.inviter_peer_id
                    && member.public_key == previous.invite.inviter_public_key
            })
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        // A crash after this write leaves the old diagnostic inert because
        // its request nonce and signed invitation hash no longer match.
        durable_atomic_write(
            &self.world_protocol_path(world, "pending-join.postcard"),
            &postcard::to_allocvec(replacement)?,
        )?;
        remove_protocol_file(self, world, "pending-join-denial.postcard")?;
        Ok(())
    }

    pub fn load_pending_join(&self, world: WorldId) -> Result<JoinRequestV1, StorageError> {
        let path = self.world_protocol_path(world, "pending-join.postcard");
        let bytes = read_canonical_file(&path)?;
        let request: JoinRequestV1 = postcard::from_bytes(&bytes)?;
        if request.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        request.validate_semantics()?;
        Ok(request)
    }

    /// Only the caller's connection-authenticated, request-correlated reply
    /// may be recorded. A concurrent replacement or completion is inert.
    pub fn record_pending_join_denial(
        &self,
        world: WorldId,
        issuer: PeerId,
        invite_hash: Hash32,
        request_nonce: [u8; 32],
        reason: JoinDenialReason,
    ) -> Result<bool, StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let pending = match self.load_pending_join(world) {
            Ok(pending) => pending,
            Err(StorageError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if pending.invite.inviter_peer_id != issuer
            || pending.invite.record_hash()? != invite_hash
            || pending.nonce != request_nonce
        {
            return Ok(false);
        }
        let record = PendingJoinDenial { world, issuer, invite_hash, request_nonce, reason };
        let path = self.world_protocol_path(world, "pending-join-denial.postcard");
        let bytes = postcard::to_allocvec(&record)?;
        if canonical_file_exists(&path)? && read_canonical_file(&path)? == bytes {
            return Ok(true);
        }
        durable_atomic_write(&path, &bytes)?;
        Ok(true)
    }

    pub fn pending_join_denial(&self, request: &JoinRequestV1) -> Result<Option<JoinDenialReason>, StorageError> {
        let path = self.world_protocol_path(request.world_id, "pending-join-denial.postcard");
        if !canonical_file_exists(&path)? {
            return Ok(None);
        }
        let bytes = read_canonical_file(&path)?;
        let record: PendingJoinDenial = postcard::from_bytes(&bytes)?;
        if postcard::to_allocvec(&record)? != bytes {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok((record.world == request.world_id
            && record.issuer == request.invite.inviter_peer_id
            && record.invite_hash == request.invite.record_hash()?
            && record.request_nonce == request.nonce)
            .then_some(record.reason))
    }

    pub fn clear_pending_join(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        remove_protocol_file(self, world, "pending-join-denial.postcard")?;
        remove_protocol_file(self, world, "pending-join.postcard")
    }

    /// Preserve a committed invitation as a reconnect hint, not a pending
    /// admission request. Write the signed hint durably before removing the
    /// pending request so a restart cannot lose the only bootstrap address.
    pub fn complete_pending_join(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        let request = self.load_pending_join(world)?;
        let membership = self.load_membership_record(world)?;
        let descriptor = self.load_world_descriptor(world)?;
        if request.invite.genesis != self.load_world(world)?.genesis
            || descriptor.members != membership.members
            || !membership.members.iter().any(|member| {
                !member.banned
                    && member.peer_id == request.joining_member.peer_id
                    && member.public_key == request.joining_member.public_key
            })
        {
            return Err(StorageError::WorldMetadataMismatch);
        }
        durable_atomic_write(
            &self.world_protocol_path(world, "completed-join.postcard"),
            &postcard::to_allocvec(&request)?,
        )?;
        remove_protocol_file(self, world, "pending-join-denial.postcard")?;
        remove_protocol_file(self, world, "pending-join.postcard")
    }

    /// The caller must verify both signatures and current membership before
    /// dialing. This file is never read by membership admission.
    pub fn load_completed_join(&self, world: WorldId) -> Result<JoinRequestV1, StorageError> {
        let path = self.world_protocol_path(world, "completed-join.postcard");
        let request: JoinRequestV1 = postcard::from_bytes(&read_canonical_file(&path)?)?;
        request.validate_semantics()?;
        if request.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        Ok(request)
    }

    pub fn save_pending_leave(&self, request: &LeaveRequestV1) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(request.world_id)?;
        request.validate_semantics()?;
        let bytes = postcard::to_allocvec(request)?;
        durable_atomic_write(&self.world_protocol_path(request.world_id, "pending-leave.postcard"), &bytes)
    }

    pub fn load_pending_leave(&self, world: WorldId) -> Result<LeaveRequestV1, StorageError> {
        let path = self.world_protocol_path(world, "pending-leave.postcard");
        let bytes = read_canonical_file(&path)?;
        let request: LeaveRequestV1 = postcard::from_bytes(&bytes)?;
        if request.world_id != world {
            return Err(StorageError::WorldMetadataMismatch);
        }
        request.validate_semantics()?;
        Ok(request)
    }

    pub fn clear_pending_leave(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        remove_protocol_file(self, world, "pending-leave.postcard")
    }

    pub fn remove_local_membership(&self, world: WorldId) -> Result<(), StorageError> {
        let _guard = self.lock_world_transaction(world)?;
        for name in [
            "descriptor.json",
            "membership.postcard",
            "membership-promise.postcard",
            "membership-certificate.postcard",
            "pending-join.postcard",
            "pending-join-denial.postcard",
            "completed-join.postcard",
            "pending-leave.postcard",
        ] {
            remove_protocol_file(self, world, name)?;
        }
        Ok(())
    }

    fn world_protocol_path(&self, world: WorldId, name: &str) -> PathBuf {
        self.world_dir(world).join("metadata").join(name)
    }
}

fn remove_protocol_file(storage: &Storage, world: WorldId, name: &str) -> Result<(), StorageError> {
    durable_remove(&storage.world_protocol_path(world, name)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldMetadataV1;
    use std::fs;
    use swarm_protocol::{
        EpochMode, EpochRecordV1, Hash32, PeerId, WorldGenesisV1, WorldMemberV1, PROTOCOL_VERSION,
        STORAGE_SCHEMA_VERSION,
    };

    fn member(peer: u8) -> WorldMemberV1 {
        WorldMemberV1 { peer_id: PeerId([peer; 32]), public_key: [peer; 32], authority_eligible: true, banned: false }
    }

    fn create_test_world(store: &Storage) -> WorldId {
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([6; 32]),
            creation_nonce: [8; 32],
            creator_public_key: [1; 32],
            initial_membership: vec![PeerId([1; 32])],
        };
        let world = genesis.world_id().unwrap();
        store
            .create_world(&WorldMetadataV1 {
                storage_schema_version: STORAGE_SCHEMA_VERSION,
                display_name: "test".into(),
                world_id: world,
                genesis,
            })
            .unwrap();
        store
            .save_world_descriptor(&WorldDescriptorV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                compatibility_fingerprint: Hash32([6; 32]),
                members: vec![member(1), member(2)],
                preferred_replication_factor: 2,
            })
            .unwrap();
        world
    }

    fn membership(
        world: WorldId,
        authority: u8,
        epoch: u64,
        sequence: u64,
        previous: Option<Hash32>,
    ) -> MembershipRecordV1 {
        MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch,
            sequence,
            previous_membership_hash: previous,
            members: vec![member(1), member(2)],
            authority_peer_id: PeerId([authority; 32]),
            authority_public_key: [authority; 32],
            signature: vec![authority; 64],
        }
    }

    #[test]
    fn descriptor_round_trip_normalizes_members() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = WorldId([5; 32]);
        let descriptor = WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: Hash32([6; 32]),
            members: vec![member(8), member(2)],
            preferred_replication_factor: 2,
        };
        fs::create_dir_all(store.world_dir(world).join("metadata")).unwrap();
        store.save_world_descriptor(&descriptor).unwrap();
        let loaded = store.load_world_descriptor(world).unwrap();
        assert_eq!(loaded.members[0].peer_id, PeerId([2; 32]));
    }

    #[test]
    fn corrupt_epoch_cannot_be_skipped_when_extending_membership() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = create_test_world(&store);
        let initial = membership(world, 1, 0, 0, None);
        store.save_membership_record(&initial).unwrap();
        let epoch_path = store.world_dir(world).join("metadata/epoch.postcard");
        let corrupted = b"corrupted accepted authority";
        fs::write(&epoch_path, corrupted).unwrap();

        let next = membership(world, 1, 0, 1, Some(initial.record_hash().unwrap()));
        assert!(store.save_membership_record(&next).is_err());
        assert_eq!(fs::read(&epoch_path).unwrap(), corrupted);
        assert_eq!(store.load_membership_record(world).unwrap(), initial);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_membership_record_cannot_be_reinitialized() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = create_test_world(&store);
        let path = store.world_dir(world).join("metadata/membership.postcard");
        let missing = path.with_extension("missing");
        symlink(&missing, &path).unwrap();

        assert!(matches!(
            store.save_membership_record(&membership(world, 1, 0, 0, None)),
            Err(StorageError::SymlinkUnsupported(actual)) if actual == path
        ));
        assert_eq!(fs::read_link(&path).unwrap(), missing);
    }

    #[test]
    fn membership_rejects_previous_authority_after_epoch_transition() {
        let temp = tempfile::tempdir().unwrap();
        let store = Storage::open(temp.path()).unwrap();
        let world = create_test_world(&store);
        let initial = membership(world, 1, 0, 0, None);
        store.save_membership_record(&initial).unwrap();
        store
            .save_epoch_record(&EpochRecordV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                epoch_number: 1,
                previous_epoch_hash: None,
                base_state_hash: Hash32([7; 32]),
                authority_peer_id: PeerId([2; 32]),
                authority_public_key: [2; 32],
                mode: EpochMode::Recovery,
                fencing_token: 1,
                reason: "test transition".into(),
                signature: vec![2; 64],
            })
            .unwrap();

        let stale = membership(world, 1, 0, 1, Some(initial.record_hash().unwrap()));
        assert!(store.save_membership_record(&stale).is_err());
        assert_eq!(store.load_membership_record(world).unwrap(), initial);

        let promoted = membership(world, 2, 1, 1, Some(initial.record_hash().unwrap()));
        store.save_membership_record(&promoted).unwrap();
        assert_eq!(store.load_membership_record(world).unwrap(), promoted);
        store.save_membership_record(&promoted).unwrap();

        let mut conflict = promoted.clone();
        conflict.signature.push(9);
        assert!(store.save_membership_record(&conflict).is_err());
        assert_eq!(store.load_membership_record(world).unwrap(), promoted);
    }
}
