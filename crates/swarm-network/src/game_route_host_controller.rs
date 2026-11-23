//! Dispatch authenticated host route-control requests without blocking the
//! owner of SwarmNode. This is a transport/event-loop adapter only: the
//! mandatory signer and FreshAuthoritySource independently prove membership,
//! current quorum/fencing, hosting consent and live Fabric readiness.
//!
//! The controller never calls SwarmNode directly, because the daemon must
//! retain exclusive mutable ownership and keep polling next_event(). It
//! returns bounded, typed reply actions for that same daemon event loop.

use crate::{
    game_route_control::{GameRouteControlEvent, GameRouteControlResponseV1, GameRouteRefusalV1},
    game_route_host_service::BoundedGameRouteHostResponder,
};
use libp2p::request_response;
use libp2p::PeerId as TransportPeerId;
use std::sync::Arc;
use swarm_protocol::PeerId;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
use tracing::{debug, warn};

/// Bound both outstanding proof tasks and actions waiting to return to the
/// network driver. A saturated controller drops/refuses, never grants a
/// response from cache or queues game authority without a hard limit.
pub const MAX_PENDING_HOST_ROUTE_ACTIONS: usize = 16;

pub struct GameRouteHostReplyAction {
    /// Must still match the live authenticated connection on send; signing
    /// may take several seconds while a peer disconnects or gets rotated.
    pub transport_peer: TransportPeerId,
    pub application_peer: PeerId,
    pub channel: request_response::ResponseChannel<GameRouteControlResponseV1>,
    pub response: GameRouteControlResponseV1,
}

pub struct GameRouteHostControlService {
    responder: Arc<BoundedGameRouteHostResponder>,
    events: mpsc::Receiver<GameRouteControlEvent>,
}

impl GameRouteHostControlService {
    pub fn new(responder: Arc<BoundedGameRouteHostResponder>, events: mpsc::Receiver<GameRouteControlEvent>) -> Self {
        Self { responder, events }
    }

    /// The daemon must run this concurrently with SwarmNode::next_event().
    ///
    /// Create a bounded replies channel (capacity <= MAX_PENDING_HOST_ROUTE_ACTIONS),
    /// run this service as an owned task, and select on its reply receiver in
    /// the existing SwarmNode event loop. Only the owner invokes
    /// SwarmNode::respond_game_route(action.channel, action.response).
    ///
    /// The owner MUST also check its current shutdown/fencing state before
    /// sending a queued reply: a reply enqueued just before shutdown can
    /// remain in its channel after this service has exited.
    ///
    /// Dropping/cancelling this supervisor or signaling shutdown aborts all
    /// pending async admission tasks. A detached external proof worker may
    /// continue to finish, but it cannot send a route after the service exits.
    pub async fn run(mut self, replies: mpsc::Sender<GameRouteHostReplyAction>, mut shutdown: watch::Receiver<bool>) {
        // Do not depend on callers choosing the right buffer size: a large
        // reply channel could accumulate signed offers across many batches
        // even while concurrent proof workers stay within their limit.
        if replies.max_capacity() > MAX_PENDING_HOST_ROUTE_ACTIONS {
            warn!("refusing game-route host controller with an oversized reply queue");
            return;
        }
        if *shutdown.borrow() {
            return;
        }
        let mut jobs = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
                completed = jobs.join_next(), if !jobs.is_empty() => {
                    if let Some(Err(error)) = completed {
                        warn!(%error, "game route host reply worker failed");
                    }
                }
                incoming = self.events.recv() => {
                    let Some(event) = incoming else { return };
                    let GameRouteControlEvent::Inbound {
                        application_peer,
                        transport_peer,
                        request,
                        channel,
                    } = event else {
                        // Host responder owns the single subscriber. A peer
                        // that also wants outbound route discovery should use
                        // GameRouteRpcClient's independent one-shot mailbox.
                        debug!("host route controller ignored non-inbound event");
                        continue;
                    };
                    if jobs.len() >= MAX_PENDING_HOST_ROUTE_ACTIONS {
                        let _ = replies.try_send(GameRouteHostReplyAction {
                            transport_peer,
                            application_peer,
                            channel,
                            response: GameRouteControlResponseV1::Refused(GameRouteRefusalV1::RateLimited),
                        });
                        continue;
                    }
                    let responder = Arc::clone(&self.responder);
                    let replies = replies.clone();
                    let stopped = shutdown.clone();
                    jobs.spawn(async move {
                        let result = responder.answer(application_peer, transport_peer, request).await;
                        if *stopped.borrow() {
                            // Never newly enqueue a signed offer after the
                            // service has observed withdrawn hosting consent.
                            return;
                        }
                        let response = match result {
                            Ok(response) => response,
                            Err(error) => {
                                warn!(%error, "game route host signer refused request");
                                GameRouteControlResponseV1::Refused(GameRouteRefusalV1::Unavailable)
                            }
                        };
                        // No blocking send; a stalled daemon must cause a
                        // bounded denial/time-out, not an unbounded backlog.
                        let _ = replies.try_send(GameRouteHostReplyAction {
                            transport_peer, application_peer, channel, response,
                        });
                    });
                }
            }
        }
    }
}
