from pathlib import Path

path = Path("crates/swarm-cli/src/daemon.rs")
text = path.read_text()
old = '''            if let Ok(epoch) = storage.load_epoch_record(record.world_id) {
                if epoch.mode == EpochMode::Recovery
                    && (record.epoch != epoch.epoch_number
                        || record.authority_peer_id != epoch.authority_peer_id
                        || record.authority_public_key != epoch.authority_public_key)
                {
                    return Err(anyhow!("membership does not match the accepted recovery epoch"));
                }
            }
'''
new = '''            if let Ok(epoch) = storage.load_epoch_record(record.world_id) {
                // Membership is subordinate to the accepted authority epoch. A recovery
                // authority may already have promoted its membership record while a stale
                // replica is still on the base epoch. Request-response delivery is not
                // ordered across concurrent requests, so never install that future
                // membership before the certified epoch itself is accepted. The authority
                // will retransmit membership after the peer acknowledges the recovery epoch.
                if record.epoch > epoch.epoch_number {
                    return Err(anyhow!("membership cannot advance before its authority epoch is accepted"));
                }
                if record.epoch == epoch.epoch_number
                    && (record.authority_peer_id != epoch.authority_peer_id
                        || record.authority_public_key != epoch.authority_public_key)
                {
                    return Err(anyhow!("membership authority does not match the accepted epoch"));
                }
            }
'''
if old in text:
    path.write_text(text.replace(old, new, 1))
elif new not in text:
    raise SystemExit("membership epoch validation block not found")
