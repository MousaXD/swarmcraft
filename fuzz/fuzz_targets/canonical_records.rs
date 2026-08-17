#![no_main]

use libfuzzer_sys::fuzz_target;
use swarm_protocol::{
    InviteV1, MembershipRecordV1, RecoveryBallotV1, RecoveryCertificateV1, SnapshotManifestV1, WorldConfigV1,
};

fuzz_target!(|data: &[u8]| {
    if let Ok(manifest) = postcard::from_bytes::<SnapshotManifestV1>(data) {
        let _ = manifest.signing_bytes();
        let _ = manifest.manifest_hash();
    }

    if let Ok(record) = postcard::from_bytes::<MembershipRecordV1>(data) {
        let _ = record.signing_bytes();
        let _ = record.record_hash();
    }

    if let Ok(invite) = postcard::from_bytes::<InviteV1>(data) {
        let _ = invite.signing_bytes();
    }

    if let Ok(ballot) = postcard::from_bytes::<RecoveryBallotV1>(data) {
        let _ = ballot.signing_bytes();
        let _ = ballot.ballot_hash();
        let _ = ballot.generation_is_well_formed();
    }

    if let Ok(certificate) = postcard::from_bytes::<RecoveryCertificateV1>(data) {
        let _ = certificate.ballot.ballot_hash();
        let _ = certificate.unique_voters();
        for vote in &certificate.votes {
            let _ = vote.signing_bytes();
            let _ = vote.matches_ballot(&certificate.ballot);
        }
    }

    if let Ok(config) = postcard::from_bytes::<WorldConfigV1>(data) {
        let _ = config.signing_bytes();
        let _ = config.config_hash();
        let _ = config.compatibility_fingerprint();
    }
});
