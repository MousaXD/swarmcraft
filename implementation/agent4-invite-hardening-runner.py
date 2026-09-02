from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text()


def write(path: str, text: str) -> None:
    Path(path).write_text(text)


def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one target, found {count}: {old[:140]!r}")
    write(path, text.replace(old, new, 1))


# Invite DNS targets are re-resolved immediately before dial and every answer must
# still be in public scope. Local/LAN invites remain available through literal IP
# multiaddresses, so a mutable DNS name cannot silently pivot an Internet-looking
# bearer token into loopback/private address space.
replace_once(
    "crates/swarm-network/src/invite_connectivity.rs",
    "    net::{Ipv4Addr, Ipv6Addr},\n",
    "    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},\n",
)
replace_once(
    "crates/swarm-network/src/invite_connectivity.rs",
    "    #[error(\"invite connection hint is not remotely usable: {0}\")]\n    UnusableAddress(String),\n",
    "    #[error(\"invite connection hint is not remotely usable: {0}\")]\n    UnusableAddress(String),\n    #[error(\"invite DNS target {host} could not be resolved: {detail}\")]\n    DnsResolutionFailed { host: String, detail: String },\n    #[error(\"invite DNS target {host} resolved to disallowed address {address}\")]\n    DnsResolvedToDisallowedScope { host: String, address: String },\n",
)
marker = "pub fn invite_connectivity_from_snapshot(path: &Path) -> Result<InviteConnectivityV1, InviteConnectivityError> {\n"
dns_code = r'''#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsFamily {
    Any,
    V4,
    V6,
}

/// Re-resolve mutable DNS invite targets immediately before dialing and apply a
/// strict public-scope policy to every returned address. Private/LAN invites must
/// use literal private IP multiaddresses, whose local-only semantics are explicit
/// in the signed token instead of being hidden behind DNS rebinding.
pub fn validate_invite_dial_address(address: &Multiaddr) -> Result<(), InviteConnectivityError> {
    let mut dns_targets = Vec::<(String, DnsFamily)>::new();
    let mut transport_port = None;
    for protocol in address.iter() {
        match protocol {
            Protocol::Dns(name) => dns_targets.push((name.as_ref().to_owned(), DnsFamily::Any)),
            Protocol::Dns4(name) => dns_targets.push((name.as_ref().to_owned(), DnsFamily::V4)),
            Protocol::Dns6(name) => dns_targets.push((name.as_ref().to_owned(), DnsFamily::V6)),
            Protocol::Tcp(port) | Protocol::Udp(port) if transport_port.is_none() => transport_port = Some(port),
            _ => {}
        }
    }
    if dns_targets.is_empty() {
        return Ok(());
    }
    let port = transport_port.ok_or_else(|| InviteConnectivityError::UnusableAddress(address.to_string()))?;
    for (host, family) in dns_targets {
        let resolved = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|error| InviteConnectivityError::DnsResolutionFailed {
                host: host.clone(),
                detail: error.to_string(),
            })?
            .map(|socket| socket.ip())
            .filter(|ip| match (family, ip) {
                (DnsFamily::Any, _) | (DnsFamily::V4, IpAddr::V4(_)) | (DnsFamily::V6, IpAddr::V6(_)) => true,
                _ => false,
            })
            .collect::<Vec<_>>();
        validate_resolved_dns_target(&host, resolved)?;
    }
    Ok(())
}

fn validate_resolved_dns_target(
    host: &str,
    addresses: impl IntoIterator<Item = IpAddr>,
) -> Result<(), InviteConnectivityError> {
    let mut saw_address = false;
    for address in addresses {
        saw_address = true;
        let public = match address {
            IpAddr::V4(ip) => matches!(ipv4_scope(ip), Ok(IpScope::Public)),
            IpAddr::V6(ip) => matches!(ipv6_scope(ip), Ok(IpScope::Public)),
        };
        if !public {
            return Err(InviteConnectivityError::DnsResolvedToDisallowedScope {
                host: host.to_owned(),
                address: address.to_string(),
            });
        }
    }
    if !saw_address {
        return Err(InviteConnectivityError::DnsResolutionFailed {
            host: host.to_owned(),
            detail: "resolver returned no addresses for the requested family".into(),
        });
    }
    Ok(())
}

'''
replace_once("crates/swarm-network/src/invite_connectivity.rs", marker, dns_code + marker)
replace_once(
    "crates/swarm-network/src/invite_connectivity.rs",
    "    fn private_relay_endpoint_is_not_claimed_as_an_internet_relay() {\n",
    r'''    fn dns_rebinding_to_loopback_or_private_scope_is_rejected() {
        assert!(matches!(
            validate_resolved_dns_target("loopback.example", [IpAddr::V4(Ipv4Addr::LOCALHOST)]),
            Err(InviteConnectivityError::DnsResolvedToDisallowedScope { .. })
        ));
        assert!(matches!(
            validate_resolved_dns_target("private.example", [IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))]),
            Err(InviteConnectivityError::DnsResolvedToDisallowedScope { .. })
        ));
        assert!(validate_resolved_dns_target(
            "public.example",
            [IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]
        )
        .is_ok());
    }

    #[test]
    fn dns_family_with_no_resolved_answers_fails_closed() {
        assert!(matches!(
            validate_resolved_dns_target("empty.example", std::iter::empty()),
            Err(InviteConnectivityError::DnsResolutionFailed { .. })
        ));
    }

    #[test]
    fn private_relay_endpoint_is_not_claimed_as_an_internet_relay() {
''',
)
replace_once(
    "crates/swarm-network/src/lib.rs",
    "    invite_connectivity_from_snapshot, validate_invite_addresses, InviteConnectivityError, InviteConnectivityV1,\n",
    "    invite_connectivity_from_snapshot, validate_invite_addresses, validate_invite_dial_address,\n    InviteConnectivityError, InviteConnectivityV1,\n",
)

# Invite tokens are explicitly reusable bearer capabilities, but exposure is
# bounded to 24 hours and old-authority invites are already rejected by daemon
# membership/authority checks.
replace_once(
    "crates/swarm-cli/src/invite.rs",
    "pub const MAX_INVITE_TOKEN_CHARS: usize = 32 * 1024;\n",
    "pub const MAX_INVITE_TOKEN_CHARS: usize = 32 * 1024;\npub const MAX_INVITE_LIFETIME_MINUTES: u64 = 24 * 60;\npub const MAX_INVITE_LIFETIME_MS: u64 = MAX_INVITE_LIFETIME_MINUTES * 60_000;\n",
)
replace_once(
    "crates/swarm-cli/src/invite.rs",
    "    let now = unix_time_ms()?;\n    if invite.expires_unix_ms <= now {\n        return Err(anyhow!(\"invite expired\"));\n    }\n    Ok(invite)\n}\n",
    "    validate_invite_expiry(invite.expires_unix_ms, unix_time_ms()?)?;\n    Ok(invite)\n}\n\n/// Validate the bearer-token exposure window. Invitations are intentionally\n/// reusable by multiple joining identities until expiry, but only while the\n/// signer remains the world's current authority. There is no hidden single-use\n/// nonce state: the signed nonce provides token uniqueness, not consumption.\npub fn validate_invite_expiry(expires_unix_ms: u64, now_unix_ms: u64) -> Result<()> {\n    if expires_unix_ms <= now_unix_ms {\n        return Err(anyhow!(\"invite expired\"));\n    }\n    let latest_allowed = now_unix_ms\n        .checked_add(MAX_INVITE_LIFETIME_MS)\n        .ok_or_else(|| anyhow!(\"invite lifetime overflows the timestamp range\"))?;\n    if expires_unix_ms > latest_allowed {\n        return Err(anyhow!(\"invite expiry exceeds the maximum {MAX_INVITE_LIFETIME_MINUTES}-minute bearer lifetime\"));\n    }\n    Ok(())\n}\n\npub fn expiry_from_minutes(now_unix_ms: u64, expires_minutes: u64) -> Result<u64> {\n    if expires_minutes == 0 || expires_minutes > MAX_INVITE_LIFETIME_MINUTES {\n        return Err(anyhow!(\"invite lifetime must be between 1 and {MAX_INVITE_LIFETIME_MINUTES} minutes\"));\n    }\n    let lifetime_ms = expires_minutes\n        .checked_mul(60_000)\n        .ok_or_else(|| anyhow!(\"invite lifetime overflows milliseconds\"))?;\n    now_unix_ms\n        .checked_add(lifetime_ms)\n        .ok_or_else(|| anyhow!(\"invite expiry overflows the timestamp range\"))\n}\n",
)
replace_once(
    "crates/swarm-cli/src/invite.rs",
    "    fn expired_invite_is_rejected() {\n",
    r'''    fn invite_is_explicitly_reusable_until_expiry() {
        let invite = signed_invite(unix_time_ms().unwrap() + 60_000, vec!["/ip4/1.1.1.1/udp/4001/quic-v1".into()]);
        let encoded = encode(&invite).unwrap();
        assert_eq!(decode(&encoded).unwrap(), invite);
        assert_eq!(decode(&encoded).unwrap(), invite);
    }

    #[test]
    fn invite_lifetime_is_bounded_and_checked() {
        let now = unix_time_ms().unwrap();
        assert!(expiry_from_minutes(now, 0).is_err());
        assert!(expiry_from_minutes(now, MAX_INVITE_LIFETIME_MINUTES + 1).is_err());
        assert_eq!(
            expiry_from_minutes(now, MAX_INVITE_LIFETIME_MINUTES).unwrap(),
            now + MAX_INVITE_LIFETIME_MS
        );
        let overlong = signed_invite(
            now + MAX_INVITE_LIFETIME_MS + 60_000,
            vec!["/ip4/1.1.1.1/udp/4001/quic-v1".into()],
        );
        assert!(decode(&encode(&overlong).unwrap()).unwrap_err().to_string().contains("maximum"));
    }

    #[test]
    fn expired_invite_is_rejected() {
''',
)
replace_once(
    "crates/swarm-cli/src/main.rs",
    "    /// Create an expiring signed invitation for an existing world.\n",
    "    /// Create a reusable bearer invitation, valid until expiry while this peer remains current authority.\n",
)
replace_once(
    "crates/swarm-cli/src/main.rs",
    "            let lifetime_ms = expires_minutes.saturating_mul(60_000);\n",
    "            let expires_unix_ms = invite::expiry_from_minutes(invite::unix_time_ms()?, expires_minutes)?;\n",
)
replace_once(
    "crates/swarm-cli/src/main.rs",
    "                expires_unix_ms: invite::unix_time_ms()?.saturating_add(lifetime_ms),\n",
    "                expires_unix_ms,\n",
)

# Apply DNS resolution policy immediately before dialing pending signed invite
# hints, not merely at parse/sign time where DNS could later rebind.
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "    load_or_create_transport_key, BlobResumeV1, HostCapabilityV1, NetworkEvent, ReplicaAckV1, ResponseChannel,\n    SwarmNode, TransportPeerId, WireRequest, WireResponse, MAX_BLOB_CHUNK,\n",
    "    load_or_create_transport_key, validate_invite_dial_address, BlobResumeV1, HostCapabilityV1, NetworkEvent,\n    ReplicaAckV1, ResponseChannel, SwarmNode, TransportPeerId, WireRequest, WireResponse, MAX_BLOB_CHUNK,\n",
)
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "                Ok(address) => {\n                    if let Err(error) = node.dial(address) {\n",
    "                Ok(address) => {\n                    if let Err(error) = validate_invite_dial_address(&address) {\n                        warn!(world = %metadata.world_id, %value, %error, \"invite bootstrap DNS scope validation failed\");\n                        continue;\n                    }\n                    if let Err(error) = node.dial(address) {\n",
)
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "            if request.invite.expires_unix_ms < unix_millis()? {\n                return Err(anyhow!(\"invite has expired\"));\n            }\n",
    "            crate::invite::validate_invite_expiry(request.invite.expires_unix_ms, unix_millis()?)\n                .context(\"join invite lifetime is invalid\")?;\n",
)

# Consolidate descriptor authorization so proactive sending and inbound request
# dispatch are tested against exactly the same current/non-banned/key-bound rule.
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "        let Some(remote_member) = descriptor.member(application_peer) else { continue };\n        if remote_member.banned || peer_id_from_public_key(&remote_member.public_key) != application_peer {\n            continue;\n        }\n",
    "        if authorized_descriptor_member(&descriptor, application_peer).is_err() {\n            continue;\n        }\n",
)
replace_once(
    "crates/swarm-cli/src/daemon.rs",
    "fn authorize_member(storage: &Storage, world: WorldId, peer: PeerId) -> Result<()> {\n    let descriptor = storage.load_world_descriptor(world)?;\n    let member = descriptor.member(peer).context(\"peer is not an authorized member of this world\")?;\n    if swarm_protocol::peer_id_from_public_key(&member.public_key) != peer {\n        return Err(anyhow!(\"world membership public key does not match peer identity\"));\n    }\n    if member.banned {\n        return Err(anyhow!(\"peer is banned from this world\"));\n    }\n    Ok(())\n}\n",
    "fn authorized_descriptor_member(\n    descriptor: &WorldDescriptorV1,\n    peer: PeerId,\n) -> Result<&swarm_protocol::WorldMemberV1> {\n    let member = descriptor.member(peer).context(\"peer is not an authorized member of this world\")?;\n    if peer_id_from_public_key(&member.public_key) != peer {\n        return Err(anyhow!(\"world membership public key does not match peer identity\"));\n    }\n    if member.banned {\n        return Err(anyhow!(\"peer is banned from this world\"));\n    }\n    Ok(member)\n}\n\nfn authorize_member(storage: &Storage, world: WorldId, peer: PeerId) -> Result<()> {\n    let descriptor = storage.load_world_descriptor(world)?;\n    authorized_descriptor_member(&descriptor, peer)?;\n    Ok(())\n}\n",
)

# Append direct authorization matrix tests to the daemon binary unit test target.
daemon = read("crates/swarm-cli/src/daemon.rs")
daemon += r'''

#[cfg(test)]
mod authorization_matrix_tests {
    use super::*;

    fn member(key: [u8; 32], banned: bool) -> swarm_protocol::WorldMemberV1 {
        swarm_protocol::WorldMemberV1 {
            peer_id: peer_id_from_public_key(&key),
            public_key: key,
            authority_eligible: true,
            banned,
        }
    }

    fn descriptor(members: Vec<swarm_protocol::WorldMemberV1>) -> WorldDescriptorV1 {
        WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId([9; 32]),
            compatibility_fingerprint: Hash32([8; 32]),
            members,
            preferred_replication_factor: 2,
        }
    }

    #[test]
    fn current_member_is_authorized_but_stranger_and_removed_member_are_not() {
        let current = member([1; 32], false);
        let current_peer = current.peer_id;
        let stranger_peer = peer_id_from_public_key(&[2; 32]);
        let current_descriptor = descriptor(vec![current]);
        assert!(authorized_descriptor_member(&current_descriptor, current_peer).is_ok());
        assert!(authorized_descriptor_member(&current_descriptor, stranger_peer).is_err());

        let removed_descriptor = descriptor(Vec::new());
        assert!(authorized_descriptor_member(&removed_descriptor, current_peer).is_err());
    }

    #[test]
    fn banned_and_key_mismatched_members_are_not_authorized() {
        let banned = member([3; 32], true);
        let banned_peer = banned.peer_id;
        assert!(authorized_descriptor_member(&descriptor(vec![banned]), banned_peer).is_err());

        let claimed_peer = peer_id_from_public_key(&[4; 32]);
        let mismatched = swarm_protocol::WorldMemberV1 {
            peer_id: claimed_peer,
            public_key: [5; 32],
            authority_eligible: true,
            banned: false,
        };
        assert!(authorized_descriptor_member(&descriptor(vec![mismatched]), claimed_peer).is_err());
    }
}
'''
write("crates/swarm-cli/src/daemon.rs", daemon)

# Exercise rate limiting through a real unauthenticated QUIC/request-response peer
# and prove a legitimate authenticated peer can still make progress afterward.
input_test = read("crates/swarm-network/tests/input_hardening.rs")
input_test += r'''

#[tokio::test]
async fn pre_auth_request_flood_is_rate_limited_without_blocking_a_valid_peer() {
    let mut victim = new_node();
    let victim_peer = victim.local_transport_peer_id();
    victim.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = listen_address(&mut victim).await;

    let behaviour = cbor::Behaviour::new(
        [(StreamProtocol::new(WIRE_PROTOCOL), ProtocolSupport::Full)],
        request_response::Config::default().with_request_timeout(Duration::from_secs(5)),
    );
    let mut attacker =
        SwarmBuilder::with_new_identity().with_tokio().with_quic().with_behaviour(|_| behaviour).unwrap().build();
    attacker.dial(address.clone()).unwrap();

    let mut sent = false;
    timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                victim_event = victim.next_event() => {
                    if let Err(error) = victim_event {
                        panic!("request flood terminated the victim event loop: {error:#}");
                    }
                }
                attacker_event = attacker.select_next_some() => match attacker_event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == victim_peer && !sent => {
                        for nonce in 0..16_u64 {
                            attacker.behaviour_mut().send_request(&victim_peer, WireRequest::Ping { nonce });
                        }
                        sent = true;
                    }
                    SwarmEvent::Behaviour(request_response::Event::Message {
                        message: request_response::Message::Response {
                            response: WireResponse::Error { code, .. },
                            ..
                        },
                        ..
                    }) if code == "RATE_LIMITED" => break,
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("pre-auth flood should hit an explicit admission budget");

    // Keep the abusive transport connected. A separate valid peer must still
    // authenticate and complete application traffic while the attacker is isolated.
    let mut valid = new_node();
    authenticate_pair(&mut valid, &mut victim, address).await;
    assert_ping_round_trip(&mut valid, &mut victim, 0x51afe).await;
    drop(attacker);
}
'''
write("crates/swarm-network/tests/input_hardening.rs", input_test)
