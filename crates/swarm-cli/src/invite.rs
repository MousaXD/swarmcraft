use anyhow::{anyhow, Context, Result};
use std::time::{SystemTime, UNIX_EPOCH};
use swarm_core::verify_invite_signature;
use swarm_protocol::{InviteV1, PROTOCOL_VERSION};

const PREFIX: &str = "scinvite:";

pub fn encode(invite: &InviteV1) -> Result<String> {
    let bytes = postcard::to_allocvec(invite).context("cannot encode invite")?;
    Ok(format!("{PREFIX}{}", hex::encode(bytes)))
}

pub fn decode(value: &str) -> Result<InviteV1> {
    let encoded = value.strip_prefix(PREFIX).ok_or_else(|| anyhow!("invite must start with {PREFIX}"))?;
    let bytes = hex::decode(encoded).context("invite payload is not valid hex")?;
    let invite: InviteV1 = postcard::from_bytes(&bytes).context("invite payload is invalid")?;
    if invite.protocol_version != PROTOCOL_VERSION {
        return Err(anyhow!("invite protocol version {} is unsupported", invite.protocol_version));
    }
    if invite.genesis.world_id()? != invite.world_id {
        return Err(anyhow!("invite world ID does not match its genesis record"));
    }
    verify_invite_signature(&invite).context("invite signature is invalid")?;
    let now = unix_time_ms()?;
    if invite.expires_unix_ms <= now {
        return Err(anyhow!("invite expired"));
    }
    Ok(invite)
}

pub fn unix_time_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH).context("system clock is before Unix epoch")?.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarm_core::{random_nonce, PeerIdentity};
    use swarm_protocol::{Hash32, PeerId, WorldGenesisV1};

    #[test]
    fn signed_invite_round_trip() {
        let identity = PeerIdentity::generate();
        let genesis = WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([4; 32]),
            creation_nonce: random_nonce(),
            creator_public_key: identity.public_key(),
            initial_membership: vec![identity.peer_id(), PeerId([8; 32])],
        };
        let mut invite = InviteV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: genesis.world_id().unwrap(),
            display_name: "Test world".into(),
            genesis,
            inviter_peer_id: identity.peer_id(),
            inviter_public_key: identity.public_key(),
            bootstrap_addrs: vec!["/ip4/127.0.0.1/udp/4001/quic-v1".into()],
            expires_unix_ms: unix_time_ms().unwrap() + 60_000,
            nonce: random_nonce(),
            signature: Vec::new(),
        };
        identity.sign_invite(&mut invite).unwrap();
        assert_eq!(decode(&encode(&invite).unwrap()).unwrap(), invite);
    }
}
