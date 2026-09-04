from pathlib import Path


path = Path("crates/swarm-network/src/discovery.rs")
text = path.read_text()

challenge_old = '''                                    self.respond(channel, WireResponse::HelloChallengeAccepted)?;
                                    self.swarm
                                        .behaviour_mut()
                                        .request_response
                                        .send_request(&peer, WireRequest::HelloProof(Box::new(proof)));
'''
challenge_new = '''                                    if let Err(error) = self.respond(channel, WireResponse::HelloChallengeAccepted) {
                                        debug!(
                                            transport_peer = %peer,
                                            %connection_id,
                                            %error,
                                            "discovery auth challenge acknowledgement channel closed; isolating peer request"
                                        );
                                    }
                                    self.swarm
                                        .behaviour_mut()
                                        .request_response
                                        .send_request(&peer, WireRequest::HelloProof(Box::new(proof)));
'''
if challenge_old in text:
    text = text.replace(challenge_old, challenge_new, 1)
elif challenge_new not in text:
    raise SystemExit("missing HelloChallengeAccepted response anchor")

accepted_old = '''                                            self.respond(
                                                channel,
                                                WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                            )?;
                                            return Ok(DiscoveryNetworkEvent::Authenticated {
'''
accepted_new = '''                                            if let Err(error) = self.respond(
                                                channel,
                                                WireResponse::HelloAccepted { protocol_version: PROTOCOL_VERSION },
                                            ) {
                                                debug!(
                                                    transport_peer = %peer,
                                                    %connection_id,
                                                    %error,
                                                    "discovery auth acceptance channel closed after proof verification; keeping failure peer-local"
                                                );
                                            }
                                            return Ok(DiscoveryNetworkEvent::Authenticated {
'''
if accepted_old in text:
    text = text.replace(accepted_old, accepted_new, 1)
elif accepted_new not in text:
    raise SystemExit("missing HelloAccepted response anchor")

path.write_text(text)
print("FINAL-028 discovery auth response-channel closure is isolated to the peer/request")
