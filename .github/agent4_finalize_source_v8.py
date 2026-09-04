from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing Agent 4 lifecycle-regression anchor in {path}: {old[:220]!r}")
    p.write_text(text.replace(old, new, 1))


network = "crates/swarm-network/src/discovery.rs"

replace_once(
    network,
    '''    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).map(|(peer, _)| *peer)
    }
''',
    '''    pub fn application_peer(&self, transport_peer: &TransportPeerId) -> Option<PeerId> {
        self.authenticated.get(transport_peer).map(|(peer, _)| *peer)
    }

    pub fn is_connected(&self, transport_peer: &TransportPeerId) -> bool {
        self.established_connections
            .get(transport_peer)
            .is_some_and(|connections| !connections.is_empty())
    }

    pub fn is_authenticated(&self, transport_peer: &TransportPeerId) -> bool {
        self.authenticated.get(transport_peer).is_some_and(|(_, connection_id)| {
            self.established_connections
                .get(transport_peer)
                .is_some_and(|connections| connections.contains(connection_id))
        })
    }

    pub fn established_connection_count(&self, transport_peer: &TransportPeerId) -> usize {
        self.established_connections.get(transport_peer).map_or(0, HashSet::len)
    }
''',
)

replace_once(
    network,
    '''        request.validate_limits()?;
        Ok(self.swarm.behaviour_mut().request_response.send_request(peer, request))
''',
    '''        request.validate_limits()?;
        let request_id = self.swarm.behaviour_mut().request_response.send_request(peer, request);
        debug!(
            transport_peer = %peer,
            active_connection = ?self.active_connections.get(peer),
            ?request_id,
            "discovery outbound request queued"
        );
        Ok(request_id)
''',
)

replace_once(
    network,
    '''                    self.connection_counts.insert(peer_id, established.len());
                    debug_assert_eq!(established.len(), num_established.get() as usize);

                    if !application_connection_allowed(
''',
    '''                    self.connection_counts.insert(peer_id, established.len());
                    debug_assert_eq!(established.len(), num_established.get() as usize);
                    debug!(
                        transport_peer = %peer_id,
                        %connection_id,
                        tracked_established = established.len(),
                        reported_established = num_established.get(),
                        active_connection = ?self.active_connections.get(&peer_id),
                        "discovery connection established"
                    );

                    if !application_connection_allowed(
''',
)

replace_once(
    network,
    '''                    self.connection_counts.insert(peer_id, established.len());
                    debug_assert_eq!(established.len(), num_established as usize);

                    let closing_application_peer = self
''',
    '''                    self.connection_counts.insert(peer_id, established.len());
                    debug_assert_eq!(established.len(), num_established as usize);
                    debug!(
                        transport_peer = %peer_id,
                        %connection_id,
                        tracked_remaining = established.len(),
                        remaining_established = num_established,
                        active_connection = ?self.active_connections.get(&peer_id),
                        another_connection_live = !established.is_empty(),
                        "discovery connection closed"
                    );

                    let closing_application_peer = self
''',
)

test_path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
text = test_path.read_text()

node_anchor = '''    let (x_identity, mut x_node, x_address, x_transport_peer) = make_node(X).await;
'''
node_replacement = '''    let (x_identity, mut x_node, x_address, x_transport_peer) = make_node(X).await;
    eprintln!(
        "FINAL-028 transport peers: current={b_transport_peer} voter={c_transport_peer} stale={a_transport_peer} attacker={x_transport_peer}"
    );
'''
if node_anchor in text:
    text = text.replace(node_anchor, node_replacement, 1)
elif node_replacement not in text:
    raise SystemExit("missing transport-peer diagnostics anchor")

resilience = r'''

fn spawn_echo_peer(mut node: DiscoveryNode, label: &'static str) -> JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        loop {
            match node.next_event().await.map_err(|error| format!("{label} discovery node failed: {error:#}"))? {
                DiscoveryNetworkEvent::InboundRequest { channel, .. } => {
                    node.respond(
                        channel,
                        WireResponse::Error {
                            code: "TEST_OK".into(),
                            message: label.into(),
                        },
                    )
                    .map_err(|error| format!("{label} response failed: {error:#}"))?;
                }
                _ => {}
            }
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_dials_and_peer_disconnect_keep_surviving_provider_usable() {
    let (_, closing_node, closing_address, _) = make_node([71; 32]).await;
    let (_, surviving_node, surviving_address, _) = make_node([72; 32]).await;
    let (_, mut client, _, _) = make_node([73; 32]).await;

    let mut closing_task = spawn_echo_peer(closing_node, "closing-provider");
    let mut surviving_task = spawn_echo_peer(surviving_node, "surviving-provider");

    let closing_peer = client.add_bootstrap_address(closing_address.parse().unwrap()).unwrap();
    let surviving_peer = client.add_bootstrap_address(surviving_address.parse().unwrap()).unwrap();

    for _ in 0..8 {
        client.dial_peer(closing_peer).unwrap();
        client.dial_peer(surviving_peer).unwrap();
    }

    timeout(Duration::from_secs(10), async {
        loop {
            if client.is_authenticated(&closing_peer) && client.is_authenticated(&surviving_peer) {
                break;
            }
            client.next_event().await.expect("client discovery event while authenticating providers");
        }
    })
    .await
    .expect("both providers must authenticate");

    assert_eq!(client.established_connection_count(&closing_peer), 1);
    assert_eq!(client.established_connection_count(&surviving_peer), 1);

    for _ in 0..8 {
        client.dial_peer(closing_peer).unwrap();
        client.dial_peer(surviving_peer).unwrap();
    }
    let _ = timeout(Duration::from_millis(500), async {
        loop {
            client.next_event().await.expect("client discovery event while suppressing duplicate dials");
            assert!(client.established_connection_count(&closing_peer) <= 1);
            assert!(client.established_connection_count(&surviving_peer) <= 1);
        }
    })
    .await;
    assert_eq!(client.established_connection_count(&closing_peer), 1);
    assert_eq!(client.established_connection_count(&surviving_peer), 1);

    closing_task.abort();
    let closing_outcome = closing_task.await;
    assert!(closing_outcome.is_err_and(|error| error.is_cancelled()));

    timeout(Duration::from_secs(10), async {
        loop {
            if !client.is_connected(&closing_peer) {
                break;
            }
            client.next_event().await.expect("client discovery event while closing one provider");
        }
    })
    .await
    .expect("closing provider disconnect must be observed");

    assert!(client.is_connected(&surviving_peer));
    assert!(client.is_authenticated(&surviving_peer));
    assert_eq!(client.established_connection_count(&surviving_peer), 1);

    let request_id = client
        .send_request(
            &surviving_peer,
            WireRequest::DiscoveryPublic {
                filter: swarm_protocol::DiscoveryFilterV1::default(),
            },
        )
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            match client.next_event().await.expect("client discovery event awaiting surviving provider response") {
                DiscoveryNetworkEvent::Response {
                    transport_peer,
                    request_id: observed,
                    response: WireResponse::Error { code, .. },
                } if transport_peer == surviving_peer && observed == request_id => {
                    assert_eq!(code, "TEST_OK");
                    break;
                }
                DiscoveryNetworkEvent::OutboundFailure {
                    transport_peer,
                    request_id: observed,
                    error,
                } if transport_peer == surviving_peer && observed == request_id => {
                    panic!("surviving provider request failed after peer disconnect: {error}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("surviving provider must remain usable");

    surviving_task.abort();
    let surviving_outcome = surviving_task.await;
    assert!(surviving_outcome.is_err_and(|error| error.is_cancelled()));
}
'''

if "duplicate_dials_and_peer_disconnect_keep_surviving_provider_usable" not in text:
    text += resilience

test_path.write_text(text)
print("FINAL-028 lifecycle diagnostics + permanent duplicate-dial/disconnect resilience regression added")
