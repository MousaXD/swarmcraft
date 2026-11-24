//! Bounded proof envelope for data-only historical checkpoint replay.
//! Neither a historical record nor this envelope is a live authority lease.
use crate::{
    AuthorityTransferV1, EpochRecordV1, Hash32, MembershipCertificateV1, MembershipRecordV1, ProtocolError,
    RecoveryCertificateV1, SleepRecordV1, SnapshotManifestV1, WorldConfigV1, WorldGenesisV1, WorldId,
};
use serde::{Deserialize, Serialize};

pub const MAX_REPLAY_EPOCHS: usize = 256;
pub const MAX_REPLAY_CONFIGS: usize = 256;
pub const MAX_REPLAY_MEMBERSHIP_RECORDS: usize = 512;
pub const MAX_REPLAY_CHECKPOINTS: usize = 4096;
pub const MAX_REPLAY_PROOF_BYTES: usize = 512 * 1024;

/// A reference to exact signed manifest bytes. Receivers still verify the
/// actual manifest signature, ancestry and every referenced blob before use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalCheckpointV1 {
    pub snapshot_number: u64,
    pub sequence: u64,
    pub epoch: u64,
    pub manifest_hash: Hash32,
    pub state_root: Hash32,
}

impl HistoricalCheckpointV1 {
    pub fn from_manifest(manifest: &SnapshotManifestV1) -> Result<Self, ProtocolError> {
        Ok(Self {
            snapshot_number: manifest.snapshot_number,
            sequence: manifest.sequence,
            epoch: manifest.epoch,
            manifest_hash: manifest.manifest_hash()?,
            state_root: manifest.state_root,
        })
    }
}

/// Evidence for the boundary between two consecutive signed authority
/// generations. The entire transfer transcript is necessary: a lone
/// committed message is not evidence of the target's signed acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoricalTransitionV1 {
    Transfer {
        prepared: Box<AuthorityTransferV1>,
        accepted: Box<AuthorityTransferV1>,
        committed: Box<AuthorityTransferV1>,
    },
    Recovery(Box<RecoveryCertificateV1>),
    SoleMemberWake(Box<SleepRecordV1>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalReplayProofV1 {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub genesis: WorldGenesisV1,
    pub epochs: Vec<EpochRecordV1>,
    pub membership: Vec<MembershipRecordV1>,
    pub membership_certificates: Vec<MembershipCertificateV1>,
    pub configs: Vec<WorldConfigV1>,
    pub checkpoints: Vec<HistoricalCheckpointV1>,
    /// One entry per epoch successor, in chronological order.
    pub transitions: Vec<HistoricalTransitionV1>,
    /// The final authority signs the exact replay envelope. This is a
    /// historical data commitment, NOT evidence of freshness or quorum.
    pub signature: Vec<u8>,
}

impl HistoricalReplayProofV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let unsigned = (
            self.protocol_version,
            self.world_id,
            &self.genesis,
            &self.epochs,
            &self.membership,
            &self.membership_certificates,
            &self.configs,
            &self.checkpoints,
            &self.transitions,
        );
        let mut bytes = b"swarmcraft/historical-replay-proof/v1\0".to_vec();
        bytes.extend(postcard::to_allocvec(&unsigned)?);
        Ok(bytes)
    }
}
