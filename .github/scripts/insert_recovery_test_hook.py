from pathlib import Path

path = Path("crates/swarm-cli/src/daemon.rs")
text = path.read_text()
old = '''                    NetworkEvent::Response { transport_peer, request_id, response } => {
                        let context = outbound.remove(&request_key(&request_id));
                        handle_response(
                            storage,
                            &mut node,
                            &transport_peer,
                            context,
                            response,
                            &mut leases,
                            Instant::now(),
                        )?;
                    }
'''
new = '''                    NetworkEvent::Response { transport_peer, request_id, response } => {
                        // Receiving HelloAccepted means the remote has authenticated our
                        // signed PeerHello. If we have also authenticated theirs, both sides
                        // can now safely accept canonical world synchronization requests.
                        // Re-push here because the earlier Authenticated event can race the
                        // remote handshake and be rejected with HANDSHAKE_REQUIRED.
                        if matches!(response, WireResponse::HelloAccepted { .. }) {
                            if let Some(application_peer) = node.application_peer(&transport_peer) {
                                leases.authenticated_peers.insert(transport_peer, application_peer);
                                push_pending_membership_requests(
                                    storage,
                                    &mut node,
                                    &transport_peer,
                                    application_peer,
                                )?;
                                push_known_worlds(
                                    storage,
                                    &mut node,
                                    &transport_peer,
                                    application_peer,
                                    identity.peer_id(),
                                    &mut outbound,
                                )?;
                            }
                        }
                        let context = outbound.remove(&request_key(&request_id));
                        handle_response(
                            storage,
                            &mut node,
                            &transport_peer,
                            context,
                            response,
                            &mut leases,
                            Instant::now(),
                        )?;
                    }
'''
if old not in text:
    if new in text:
        raise SystemExit(0)
    raise SystemExit("target response block not found")
path.write_text(text.replace(old, new, 1))
