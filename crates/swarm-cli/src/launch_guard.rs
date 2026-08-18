use anyhow::{bail, Result};
use swarm_protocol::WorldId;
use swarm_storage::Storage;

/// Direct authority launch is valid for a fresh/running authority and for a
/// durably sleeping solo world. A sleeping multi-member world must go through
/// a quorum-backed wake transition rather than allowing the first local Play
/// action to mint a solo authority generation.
pub fn ensure_direct_launch_safe(storage: &Storage, world: WorldId) -> Result<()> {
    if storage.load_sleep_record(world).is_err() {
        return Ok(());
    }
    let descriptor = storage.load_world_descriptor(world)?;
    let active_members = descriptor.members.iter().filter(|member| !member.banned).count();
    ensure_sleeping_member_count_safe(active_members)
}

fn ensure_sleeping_member_count_safe(active_members: usize) -> Result<()> {
    if active_members > 1 {
        bail!(
            "multi-member wake requires a quorum-backed authority transition; direct solo authority launch is fail-closed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleeping_solo_world_can_use_direct_launch() {
        assert!(ensure_sleeping_member_count_safe(1).is_ok());
    }

    #[test]
    fn sleeping_multi_member_world_cannot_self_elect_by_direct_launch() {
        let error = ensure_sleeping_member_count_safe(2).unwrap_err();
        assert!(error.to_string().contains("quorum-backed authority transition"));
        assert!(ensure_sleeping_member_count_safe(3).is_err());
    }
}
