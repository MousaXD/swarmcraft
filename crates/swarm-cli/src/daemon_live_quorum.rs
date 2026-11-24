//! Bounded, authenticated fresh-quorum collection on the daemon's existing
//! SwarmNode. This never signs a Minecraft route or asserts Fabric readiness.
//!
//! Agent 1's live proof provider may request these signed votes from a
//! blocking verification worker once its real FabricSession has been wired.
//! The daemon event loop retains exclusive ownership of SwarmNode. Until that
//! integration exists, the game-route responder remains deny-only.

use super::{
    request_key, sign_local_live_quorum_vote, unix_millis, validate_membership_certificate_signatures,
    verify_membership_signature, LeaseRuntime, OutboundContext,
};
use anyhow::{anyhow, bail, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::mpsc as sync_mpsc,
    time::{Duration, Instant},
};
use swarm_consensus::{
    membership::validate_discovery_freshness_quorum, membership::validate_discovery_membership_proof_shape, quorum_size,
};
use swarm_core::{discovery::verify_discovery_freshness_vote, PeerIdentity};
use swarm_network::{SwarmNode, TransportPeerId, WireRequest, WireResponse};
use swarm_protocol::{
    peer_id_from_public_key, DiscoveryFreshnessChallengeV1, DiscoveryFreshnessVoteV1, DiscoveryMembershipProofV1,
    PeerId, WorldId, PROTOCOL_VERSION,
};
use swarm_storage::Storage;
use tokio::sync::mpsc;

pub(super) const MAX_CONCURRENT_COLLECTIONS: usize = 4;
const MAX_VOTERS_PER_COLLECTION: usize = 32;
const MAX_RECENT_CHALLENGES: usize = 1024;
const QUORUM_COLLECTION_TIMEOUT: Duration = Duration::from_millis(2_600);
const COLLECTION_QUEUE_CAPACITY: usize = MAX_CONCURRENT_COLLECTIONS;

/// The wire envelope is never identity proof. The responder must be exactly
/// the authenticated application peer targeted by this request, and the vote
/// must be a valid signature over the immutable one-use challenge.
fn verify_targeted_vote(
    expected_peer: PeerId,
    expected_key: [u8; 32],
    challenge: &DiscoveryFreshnessChallengeV1,
    response: WireResponse,
) -> Result<DiscoveryFreshnessVoteV1> {
    let WireResponse::DiscoveryFreshnessVote(Some(vote)) = response else {
        bail!("canonical quorum voter refused or sent an unexpected response");
    };
    if vote.voter_peer_id != expected_peer || vote.voter_public_key != expected_key {
        bail!("quorum voter identity did not match the authenticated recipient");
    }
    verify_discovery_freshness_vote(&vote, challenge)?;
    Ok(*vote)
}

pub(super) struct CollectVotesCommand {
    pub challenge: DiscoveryFreshnessChallengeV1,
    pub response: sync_mpsc::SyncSender<Result<Vec<DiscoveryFreshnessVoteV1>>>,
}

/// The blocking proof provider can use this client from spawn_blocking, not
/// from the daemon's asynchronous network event loop. The queue and deadline
/// are hard bounds; dead, overloaded or unscheduled daemons never return votes.
#[derive(Clone)]
pub(super) struct LiveQuorumClient {
    sender: mpsc::Sender<CollectVotesCommand>,
}

impl LiveQuorumClient {
    pub fn collect_blocking(&self, challenge: DiscoveryFreshnessChallengeV1) -> Result<Vec<DiscoveryFreshnessVoteV1>> {
        let (response, receiver) = sync_mpsc::sync_channel(1);
        self.sender
            .try_send(CollectVotesCommand { challenge, response })
            .map_err(|_| anyhow!("live quorum driver unavailable or its request budget is exhausted"))?;
        receiver
            .recv_timeout(QUORUM_COLLECTION_TIMEOUT + Duration::from_millis(200))
            .map_err(|_| anyhow!("live quorum vote collection exceeded its hard deadline"))?
    }
}

pub(super) fn channel() -> (LiveQuorumClient, mpsc::Receiver<CollectVotesCommand>) {
    let (sender, receiver) = mpsc::channel(COLLECTION_QUEUE_CAPACITY);
    (LiveQuorumClient { sender }, receiver)
}

struct ExpectedVoter {
    key: [u8; 32],
    transport: TransportPeerId,
}

pub(super) struct InboundVoteResult {
    pub id: u64,
    pub peer: PeerId,
    pub transport: TransportPeerId,
    pub response: WireResponse,
}

struct PendingCollection {
    challenge: DiscoveryFreshnessChallengeV1,
    votes: BTreeMap<PeerId, DiscoveryFreshnessVoteV1>,
    expected: HashMap<PeerId, ExpectedVoter>,
    signed_local_vote: DiscoveryFreshnessVoteV1,
    response: sync_mpsc::SyncSender<Result<Vec<DiscoveryFreshnessVoteV1>>>,
    deadline: Instant,
}

#[derive(Default)]
pub(super) struct LiveQuorumDriver {
    sequence: u64,
    pending: HashMap<u64, PendingCollection>,
    recent_nonces: HashMap<(WorldId, [u8; 32]), Instant>,
}

impl LiveQuorumDriver {
    pub fn start(
        &mut self,
        storage: &Storage,
        identity: &PeerIdentity,
        node: &mut SwarmNode,
        outbound: &mut HashMap<String, OutboundContext>,
        runtime: &LeaseRuntime,
        command: CollectVotesCommand,
    ) {
        let response = command.response.clone();
        if let Err(error) = self.begin(storage, identity, node, outbound, runtime, command) {
            let _ = response.try_send(Err(error));
        }
    }

    fn begin(
        &mut self,
        storage: &Storage,
        identity: &PeerIdentity,
        node: &mut SwarmNode,
        outbound: &mut HashMap<String, OutboundContext>,
        runtime: &LeaseRuntime,
        command: CollectVotesCommand,
    ) -> Result<()> {
        let challenge = &command.challenge;
        self.expire(Instant::now());
        if self.pending.len() >= MAX_CONCURRENT_COLLECTIONS || self.recent_nonces.len() >= MAX_RECENT_CHALLENGES {
            bail!("fresh-quorum proof budget exhausted");
        }
        if challenge.verifier_peer_id != identity.peer_id() || challenge.world_id == WorldId::default() {
            bail!("only the locally authenticated canonical verifier can request live quorum votes");
        }
        let replay_key = (challenge.world_id, challenge.nonce);
        if self.recent_nonces.contains_key(&replay_key) {
            bail!("duplicate live quorum challenge nonce");
        }
        // Independent canonical signature/fencing/sleep/membership preflight.
        // The one local signature is valid only for THIS current request.
        let now_unix_ms = unix_millis()?;
        let local_vote = sign_local_live_quorum_vote(storage, identity, identity.peer_id(), challenge, now_unix_ms)?
            .context("local canonical voter refused the fresh quorum challenge")?;
        let membership = storage.load_membership_record(challenge.world_id)?;
        verify_membership_signature(&membership)?;
        if membership.record_hash()? != challenge.membership_hash {
            bail!("canonical membership changed before remote quorum collection");
        }
        let active_members = membership.members.iter().filter(|member| !member.banned).collect::<Vec<_>>();
        if active_members.is_empty() || active_members.len() > MAX_VOTERS_PER_COLLECTION {
            bail!("unsupported or unbounded number of current canonical voters");
        }
        let mut expected = HashMap::new();
        for member in active_members.iter().filter(|member| member.peer_id != identity.peer_id()) {
            let Some((transport, _)) = runtime.authenticated_peers.iter().find(|(transport, peer)| {
                **peer == member.peer_id && node.application_peer(transport) == Some(member.peer_id)
            }) else {
                continue;
            };
            if peer_id_from_public_key(&member.public_key) != member.peer_id {
                bail!("current voter public key mismatches its application identity");
            }
            expected.insert(member.peer_id, ExpectedVoter { key: member.public_key, transport: *transport });
        }
        if expected.len() + 1 < quorum_size(active_members.len()) {
            bail!("not enough currently authenticated voters for a signed majority");
        }
        let mut votes = BTreeMap::new();
        votes.insert(identity.peer_id(), local_vote.clone());
        let now = Instant::now();
        self.sequence = self.sequence.checked_add(1).context("fresh quorum request sequence exhausted")?;
        let id = self.sequence;
        // Preserve the replay fence beyond the challenge's full signed expiry
        // (not merely the local 2.6s RPC window). A late identical challenge
        // must never regain admission after an early collection failure.
        let remaining_ms = challenge.expires_unix_ms.saturating_sub(now_unix_ms);
        let replay_ttl = Duration::from_millis(remaining_ms.saturating_add(500)).max(QUORUM_COLLECTION_TIMEOUT);
        self.recent_nonces.insert(replay_key, now + replay_ttl);
        let pending = PendingCollection {
            challenge: challenge.clone(),
            votes,
            expected,
            signed_local_vote: local_vote,
            response: command.response,
            deadline: now + QUORUM_COLLECTION_TIMEOUT,
        };
        for (peer, expected) in &pending.expected {
            if let Ok(request_id) =
                node.send_request(&expected.transport, WireRequest::DiscoveryFreshnessVote(Box::new(challenge.clone())))
            {
                outbound.insert(request_key(&request_id), OutboundContext::LiveQuorum { id, peer: *peer });
            }
        }
        self.pending.insert(id, pending);
        self.try_finish(storage, identity, node, runtime, id);
        Ok(())
    }

    pub fn on_response(
        &mut self,
        storage: &Storage,
        identity: &PeerIdentity,
        node: &SwarmNode,
        runtime: &LeaseRuntime,
        event: InboundVoteResult,
    ) {
        let InboundVoteResult { id, peer, transport, response } = event;
        let Some(pending) = self.pending.get_mut(&id) else { return };
        let correct_sender = pending.expected.get(&peer).is_some_and(|expected| {
            expected.transport == transport
                && runtime.authenticated_peers.get(&transport) == Some(&peer)
                && node.application_peer(&transport) == Some(peer)
        });
        if !correct_sender {
            self.fail(id, anyhow!("quorum vote response was not bound to its authenticated connection"));
            return;
        }
        let Some(expected) = pending.expected.get(&peer) else {
            self.fail(id, anyhow!("unrequested quorum signer"));
            return;
        };
        let vote = match verify_targeted_vote(peer, expected.key, &pending.challenge, response) {
            Ok(vote) => vote,
            Err(error) => {
                self.fail(id, error);
                return;
            }
        };
        if pending.votes.insert(peer, vote).is_some() {
            self.fail(id, anyhow!("duplicate signed quorum vote"));
            return;
        }
        self.try_finish(storage, identity, node, runtime, id);
    }

    fn try_finish(
        &mut self,
        storage: &Storage,
        identity: &PeerIdentity,
        node: &SwarmNode,
        runtime: &LeaseRuntime,
        id: u64,
    ) {
        let Some(pending) = self.pending.get(&id) else { return };
        let membership = match storage.load_membership_record(pending.challenge.world_id) {
            Ok(membership) => membership,
            Err(error) => {
                self.fail(id, error.into());
                return;
            }
        };
        let voter_count = membership.members.iter().filter(|member| !member.banned).count();
        if pending.votes.len() < quorum_size(voter_count) {
            return;
        }
        let result = Self::verify_completed(storage, identity, node, runtime, pending);
        if let Some(pending) = self.pending.remove(&id) {
            let _ = pending.response.try_send(result);
        }
    }

    fn verify_completed(
        storage: &Storage,
        identity: &PeerIdentity,
        node: &SwarmNode,
        runtime: &LeaseRuntime,
        pending: &PendingCollection,
    ) -> Result<Vec<DiscoveryFreshnessVoteV1>> {
        let challenge = &pending.challenge;
        if Instant::now() >= pending.deadline {
            bail!("fresh quorum response expired before canonical validation");
        }
        let current_local_vote =
            sign_local_live_quorum_vote(storage, identity, identity.peer_id(), challenge, unix_millis()?)?
                .context("canonical authority changed during quorum vote collection")?;
        if current_local_vote != pending.signed_local_vote {
            bail!("canonical local voter changed during quorum vote collection");
        }
        let metadata = storage.load_world(challenge.world_id)?;
        let membership = storage.load_membership_record(challenge.world_id)?;
        let certificates = storage.load_membership_certificate_chain(challenge.world_id)?;
        let initial = storage.load_genesis_membership(challenge.world_id)?;
        let proof = DiscoveryMembershipProofV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: challenge.world_id,
            genesis: metadata.genesis,
            initial_membership: initial,
            membership_certificates: certificates,
            current_membership: membership,
            pending_membership: None,
        };
        validate_discovery_membership_proof_shape(&proof)?;
        let genesis_peers = proof.genesis.initial_membership.iter().copied().collect::<BTreeSet<_>>();
        let initial_peers =
            proof.initial_membership.members.iter().map(|member| member.peer_id).collect::<BTreeSet<_>>();
        if proof.initial_membership.sequence != 0
            || proof.initial_membership.previous_membership_hash.is_some()
            || proof.initial_membership.authority_public_key != proof.genesis.creator_public_key
            || proof.initial_membership.authority_peer_id != peer_id_from_public_key(&proof.genesis.creator_public_key)
            || genesis_peers.len() != proof.genesis.initial_membership.len()
            || initial_peers != genesis_peers
        {
            bail!("fresh quorum voter set is not anchored to the signed world genesis");
        }
        verify_membership_signature(&proof.initial_membership)?;
        for certificate in &proof.membership_certificates {
            validate_membership_certificate_signatures(certificate)?;
        }
        let votes = pending.votes.values().cloned().collect::<Vec<_>>();
        for vote in &votes {
            verify_discovery_freshness_vote(vote, challenge)?;
            let member = proof
                .current_membership
                .members
                .iter()
                .find(|member| member.peer_id == vote.voter_peer_id && !member.banned)
                .context("quorum vote signer was removed from current canonical membership")?;
            if member.public_key != vote.voter_public_key {
                bail!("quorum vote signer key differs from current signed membership");
            }
            if vote.voter_peer_id != identity.peer_id() {
                let expected = pending.expected.get(&vote.voter_peer_id).context("quorum voter was never requested")?;
                if runtime.authenticated_peers.get(&expected.transport) != Some(&vote.voter_peer_id)
                    || node.application_peer(&expected.transport) != Some(vote.voter_peer_id)
                {
                    bail!("quorum voter disconnected or changed identity before proof completion");
                }
            }
        }
        validate_discovery_freshness_quorum(&proof, &votes)?;
        Ok(votes)
    }

    pub fn on_disconnect(&mut self, peer: &TransportPeerId) {
        let affected = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.expected.values().any(|voter| &voter.transport == peer))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in affected {
            self.fail(id, anyhow!("a current quorum voter disconnected before proof completion"));
        }
    }

    pub fn fail(&mut self, id: u64, reason: anyhow::Error) {
        if let Some(pending) = self.pending.remove(&id) {
            let _ = pending.response.try_send(Err(reason));
        }
    }

    pub fn expire(&mut self, now: Instant) {
        self.recent_nonces.retain(|_, expires| *expires > now);
        let expired =
            self.pending.iter().filter(|(_, pending)| now >= pending.deadline).map(|(id, _)| *id).collect::<Vec<_>>();
        for id in expired {
            self.fail(id, anyhow!("fresh signed quorum response exceeded its deadline"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_core::{discovery::sign_discovery_freshness_vote, PeerIdentity};
    use swarm_protocol::{DiscoveryCanonicalHeadV1, Hash32};

    fn challenge(requester: PeerId) -> DiscoveryFreshnessChallengeV1 {
        DiscoveryFreshnessChallengeV1 {
            protocol_version: PROTOCOL_VERSION,
            verifier_peer_id: requester,
            nonce: [11; 32],
            world_id: WorldId([12; 32]),
            announcement_hash: Hash32([13; 32]),
            membership_sequence: 1,
            membership_hash: Hash32([14; 32]),
            pending_membership_proposal_hash: None,
            authority_peer_id: requester,
            authority_epoch: 1,
            fencing_token: 1,
            config_sequence: 1,
            config_hash: Hash32([15; 32]),
            canonical_head: Some(DiscoveryCanonicalHeadV1 {
                snapshot_number: 2,
                manifest_hash: Hash32([16; 32]),
                epoch: 1,
                sequence: 2,
            }),
            issued_unix_ms: 1_000,
            expires_unix_ms: 2_000,
        }
    }

    #[test]
    fn targeted_quorum_vote_requires_current_nonce_signature_and_exact_peer_key() {
        let requester = PeerIdentity::from_secret_bytes([21; 32]);
        let voter = PeerIdentity::from_secret_bytes([22; 32]);
        let outsider = PeerIdentity::from_secret_bytes([23; 32]);
        let challenge = challenge(requester.peer_id());
        let correct = sign_discovery_freshness_vote(&voter, &challenge).unwrap();
        let response = WireResponse::DiscoveryFreshnessVote(Some(Box::new(correct.clone())));
        assert_eq!(verify_targeted_vote(voter.peer_id(), voter.public_key(), &challenge, response).unwrap(), correct);
        assert!(
            verify_targeted_vote(
                outsider.peer_id(),
                outsider.public_key(),
                &challenge,
                WireResponse::DiscoveryFreshnessVote(Some(Box::new(correct.clone()))),
            )
            .is_err(),
            "valid signature from another authenticated peer cannot satisfy the expected request"
        );
        assert!(verify_targeted_vote(
            voter.peer_id(),
            outsider.public_key(),
            &challenge,
            WireResponse::DiscoveryFreshnessVote(Some(Box::new(correct.clone()))),
        )
        .is_err());
        let mut altered = challenge.clone();
        altered.nonce[0] ^= 1;
        assert!(verify_targeted_vote(
            voter.peer_id(),
            voter.public_key(),
            &altered,
            WireResponse::DiscoveryFreshnessVote(Some(Box::new(correct.clone()))),
        )
        .is_err());
        let mut bad_signature = correct;
        bad_signature.signature[0] ^= 1;
        assert!(verify_targeted_vote(
            voter.peer_id(),
            voter.public_key(),
            &challenge,
            WireResponse::DiscoveryFreshnessVote(Some(Box::new(bad_signature))),
        )
        .is_err());
        assert!(verify_targeted_vote(
            voter.peer_id(),
            voter.public_key(),
            &challenge,
            WireResponse::DiscoveryFreshnessVote(None)
        )
        .is_err());
    }

    #[test]
    fn expired_or_disconnected_collections_fail_closed_without_reusing_votes() {
        let local = PeerIdentity::from_secret_bytes([24; 32]);
        let challenge = challenge(local.peer_id());
        let signed_local_vote = sign_discovery_freshness_vote(&local, &challenge).unwrap();
        let (reply, receiver) = sync_mpsc::sync_channel(1);
        let now = Instant::now();
        let mut driver = LiveQuorumDriver::default();
        driver.pending.insert(
            1,
            PendingCollection {
                challenge: challenge.clone(),
                votes: BTreeMap::new(),
                expected: HashMap::new(),
                signed_local_vote,
                response: reply,
                deadline: now,
            },
        );
        driver.recent_nonces.insert((challenge.world_id, challenge.nonce), now + Duration::from_secs(1));
        driver.expire(now);
        assert!(receiver.try_recv().unwrap().is_err());
        assert!(driver.pending.is_empty());
        assert!(driver.recent_nonces.contains_key(&(challenge.world_id, challenge.nonce)));
        driver.expire(now + Duration::from_secs(2));
        assert!(driver.recent_nonces.is_empty());

        let mut driver = LiveQuorumDriver::default();
        let (reply, receiver) = sync_mpsc::sync_channel(1);
        let peer = TransportPeerId::from_bytes(&[0, 32].into_iter().chain([25; 32]).collect::<Vec<_>>())
            .expect("valid ephemeral multihash peer");
        driver.pending.insert(
            2,
            PendingCollection {
                challenge: challenge.clone(),
                votes: BTreeMap::new(),
                expected: HashMap::from([(PeerId([26; 32]), ExpectedVoter { key: [26; 32], transport: peer })]),
                signed_local_vote: sign_discovery_freshness_vote(&local, &challenge).unwrap(),
                response: reply,
                deadline: now + Duration::from_secs(1),
            },
        );
        driver.on_disconnect(&peer);
        assert!(receiver.try_recv().unwrap().is_err());
        assert!(driver.pending.is_empty());
    }

    #[test]
    fn proof_worker_mailbox_is_bounded_and_never_fabricates_a_vote() {
        let requester = PeerIdentity::from_secret_bytes([27; 32]);
        let (client, mut receiver) = channel();
        let (response, result) = sync_mpsc::sync_channel(1);
        let expected = challenge(requester.peer_id());
        client.sender.try_send(CollectVotesCommand { challenge: expected.clone(), response }).unwrap();
        let delivered = receiver.try_recv().unwrap();
        assert_eq!(delivered.challenge, expected);
        // The mailbox delivers the verifier's actual reply; it cannot turn a
        // refused network quorum into a successful cached response.
        delivered.response.try_send(Err(anyhow!("signed majority unavailable"))).unwrap();
        assert!(result.try_recv().unwrap().is_err());

        for _ in 0..COLLECTION_QUEUE_CAPACITY {
            let (response, _) = sync_mpsc::sync_channel(1);
            client.sender.try_send(CollectVotesCommand { challenge: expected.clone(), response }).unwrap();
        }
        let (response, _) = sync_mpsc::sync_channel(1);
        assert!(client.sender.try_send(CollectVotesCommand { challenge: expected, response }).is_err());
        drop(receiver);
        assert!(client.collect_blocking(challenge(requester.peer_id())).is_err());
    }
}
