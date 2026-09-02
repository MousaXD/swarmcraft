use std::collections::BTreeSet;

use super::*;

fn semantic_error(record: &'static str, detail: impl Into<String>) -> ProtocolError {
    ProtocolError::InvalidIdentifier {
        kind: record,
        value: detail.into(),
    }
}

pub fn require_protocol_version(record: &'static str, version: u16) -> Result<(), ProtocolError> {
    if version != PROTOCOL_VERSION {
        return Err(semantic_error(
            record,
            format!("unsupported protocol version {version}; expected {PROTOCOL_VERSION}"),
        ));
    }
    Ok(())
}

fn require_strict_peer_order(record: &'static str, members: &[WorldMemberV1]) -> Result<(), ProtocolError> {
    if members.is_empty() {
        return Err(semantic_error(record, "member set must not be empty"));
    }
    if members.windows(2).any(|pair| pair[0].peer_id >= pair[1].peer_id) {
        return Err(semantic_error(
            record,
            "members must be strictly ordered by peer_id with no duplicates",
        ));
    }
    Ok(())
}

fn require_strict_peer_id_order(record: &'static str, peers: &[PeerId]) -> Result<(), ProtocolError> {
    if peers.is_empty() {
        return Err(semantic_error(record, "initial membership must not be empty"));
    }
    if peers.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(semantic_error(
            record,
            "initial membership must be strictly ordered with no duplicates",
        ));
    }
    Ok(())
}

fn require_snapshot_entry_order(entries: &[SnapshotEntry]) -> Result<(), ProtocolError> {
    if entries.windows(2).any(|pair| pair[0].path >= pair[1].path) {
        return Err(semantic_error(
            "snapshot manifest",
            "entries must be strictly ordered by path with no duplicates",
        ));
    }
    Ok(())
}

fn require_unique_artifact_keys(
    record: &'static str,
    values: &[ArtifactRequirementV1],
) -> Result<(), ProtocolError> {
    let mut seen = BTreeSet::new();
    for value in values {
        let key = (
            value.artifact_id.as_str(),
            value.version.as_str(),
            value.artifact_hash,
            value.side,
        );
        if !seen.insert(key) {
            return Err(semantic_error(
                record,
                format!(
                    "duplicate runtime artifact requirement {} {} {:?}; provider_hint cannot disambiguate canonical identity",
                    value.artifact_id, value.version, value.side
                ),
            ));
        }
    }
    Ok(())
}

impl WorldGenesisV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("world genesis", self.protocol_version)?;
        require_strict_peer_id_order("world genesis", &self.initial_membership)
    }
}

impl SnapshotManifestV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("snapshot manifest", self.protocol_version)?;
        require_snapshot_entry_order(&self.entries)?;
        if snapshot_state_root(&self.entries)? != self.state_root {
            return Err(semantic_error("snapshot manifest", "state_root does not match canonical entries"));
        }
        Ok(())
    }
}

impl EpochRecordV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("epoch record", self.protocol_version)
    }
}

impl WorldDescriptorV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("world descriptor", self.protocol_version)?;
        require_strict_peer_order("world descriptor", &self.members)
    }
}

impl MembershipRecordV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("membership record", self.protocol_version)?;
        require_strict_peer_order("membership record", &self.members)
    }
}

impl InviteV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("invite", self.protocol_version)?;
        self.genesis.validate_semantics()?;
        if self.genesis.world_id()? != self.world_id {
            return Err(semantic_error("invite", "world_id does not match genesis"));
        }
        Ok(())
    }
}

impl AuthorityTransferV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("authority transfer", self.protocol_version)
    }
}

impl AuthorityLeaseGrantV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("authority lease grant", self.protocol_version)
    }
}

impl JoinRequestV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        if !self.validate_shape() {
            return Err(semantic_error("join request", "record shape or protocol version is invalid"));
        }
        self.invite.validate_semantics()
    }
}

impl LeaveRequestV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        if !self.validate_shape() {
            return Err(semantic_error("leave request", "record shape or protocol version is invalid"));
        }
        Ok(())
    }
}

impl SleepRecordV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("sleep record", self.protocol_version)
    }
}

impl RuntimeCompatibilityManifestV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("runtime compatibility manifest", self.swarmcraft_protocol_version)?;
        require_unique_artifact_keys("server mod requirements", &self.required_server_mods)?;
        require_unique_artifact_keys("client mod requirements", &self.required_client_mods)?;
        require_unique_artifact_keys("datapack requirements", &self.datapacks)?;
        Ok(())
    }
}

impl WorldConfigV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("world config", self.protocol_version)?;
        self.compatibility.validate_semantics()
    }

    pub fn normalize_canonical(&mut self) {
        self.compatibility.normalize();
        self.presentation.normalize();
    }
}

impl MembershipVoteV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("membership vote", self.protocol_version)
    }
}

impl RecoveryBallotV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("recovery ballot", self.protocol_version)?;
        if !self.generation_is_well_formed() {
            return Err(semantic_error("recovery ballot", "authority generation is not a strict next generation"));
        }
        Ok(())
    }
}

impl RecoveryVoteV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("recovery vote", self.protocol_version)
    }
}

impl SoloBranchV1 {
    pub fn validate_semantics(&self) -> Result<(), ProtocolError> {
        require_protocol_version("solo branch", self.protocol_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(peer: u8) -> WorldMemberV1 {
        WorldMemberV1 {
            peer_id: PeerId([peer; 32]),
            public_key: [peer; 32],
            authority_eligible: true,
            banned: false,
        }
    }

    fn artifact(hint: Option<&str>) -> ArtifactRequirementV1 {
        ArtifactRequirementV1 {
            artifact_id: "example".into(),
            version: "1.0.0".into(),
            artifact_hash: Hash32([7; 32]),
            side: ArtifactSideV1::Server,
            provider_hint: hint.map(str::to_owned),
        }
    }

    #[test]
    fn membership_requires_canonical_order_and_uniqueness() {
        let base = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            epoch: 1,
            sequence: 1,
            previous_membership_hash: None,
            members: vec![member(1), member(2)],
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: Vec::new(),
        };
        assert!(base.validate_semantics().is_ok());

        let mut permuted = base.clone();
        permuted.members.reverse();
        assert!(permuted.validate_semantics().is_err());

        let mut duplicate = base;
        duplicate.members.push(member(2));
        assert!(duplicate.validate_semantics().is_err());
    }

    #[test]
    fn snapshot_requires_canonical_path_order() {
        let entry = |path: &str, byte| SnapshotEntry {
            path: path.into(),
            blob: BlobDescriptor {
                hash: Hash32([byte; 32]),
                uncompressed_size: 1,
                encoded_size: 1,
                encoding: BlobEncoding::Raw,
            },
        };
        let entries = vec![entry("a.dat", 1), entry("b.dat", 2)];
        let mut manifest = SnapshotManifestV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([1; 32]),
            snapshot_number: 1,
            epoch: 1,
            sequence: 0,
            previous_snapshot_hash: None,
            state_root: snapshot_state_root(&entries).unwrap(),
            entries,
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: Vec::new(),
        };
        assert!(manifest.validate_semantics().is_ok());
        manifest.entries.reverse();
        manifest.state_root = snapshot_state_root(&manifest.entries).unwrap();
        assert!(manifest.validate_semantics().is_err());
    }

    #[test]
    fn unsupported_versions_fail_closed_per_record_family() {
        let membership = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION + 1,
            world_id: WorldId([1; 32]),
            epoch: 1,
            sequence: 1,
            previous_membership_hash: None,
            members: vec![member(1)],
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            signature: Vec::new(),
        };
        assert!(membership.validate_semantics().is_err());

        let epoch = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION + 1,
            world_id: WorldId([1; 32]),
            epoch_number: 1,
            previous_epoch_hash: None,
            base_state_hash: Hash32([2; 32]),
            authority_peer_id: PeerId([1; 32]),
            authority_public_key: [1; 32],
            mode: EpochMode::Quorum,
            fencing_token: 1,
            reason: "test".into(),
            signature: Vec::new(),
        };
        assert!(epoch.validate_semantics().is_err());
    }

    #[test]
    fn provider_hint_cannot_disambiguate_duplicate_runtime_identity() {
        let manifest = RuntimeCompatibilityManifestV1 {
            minecraft_version: "1.21.8".into(),
            loader_id: "fabric".into(),
            loader_version: "0.17.2".into(),
            swarmcraft_protocol_version: PROTOCOL_VERSION,
            fabric_adapter_version: "0.2.0".into(),
            required_server_mods: vec![artifact(Some("provider-a")), artifact(Some("provider-b"))],
            required_client_mods: Vec::new(),
            datapacks: Vec::new(),
        };
        assert!(manifest.validate_semantics().is_err());
    }
}
