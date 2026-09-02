from pathlib import Path

path = Path("crates/swarm-cli/tests/recovery_successor_dies.rs")
text = path.read_text()
old = r'''    let remaining = survivors
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != first_index)
        .map(|(_, peer)| *peer)
        .collect::<Vec<_>>();
    let second_successor_id = remaining.iter().map(|peer| peer.identity.peer_id()).min().unwrap();

    wait_until("newer recovery round completing after first successor dies", Duration::from_secs(60), || {
        remaining.iter().all(|peer| {
            peer.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.mode == EpochMode::Recovery
                    && record.authority_peer_id == second_successor_id
            })
        })
    });

    let second_successor = *remaining.iter().find(|peer| peer.identity.peer_id() == second_successor_id).unwrap();
    let certificate = second_successor.storage.load_recovery_certificate(world).unwrap();
    assert!(certificate.ballot.round >= 2);
    assert_eq!(certificate.ballot.candidate_peer_id, second_successor_id);
    wait_until("second successor live permit", Duration::from_secs(30), || {
        permit_generation(second_successor, world)
            .is_some_and(|(epoch, fencing, heartbeat)| epoch == 2 && fencing == 2 && heartbeat >= 2)
    });

    let authority_addr = transport_address(second_successor);
    let authority_bootstrap = vec![authority_addr];
    let mut restarted_first = spawn_daemon(first_successor, &authority_bootstrap, false);
    wait_until_with_daemon(
        "stale first successor adopting newer certified recovery",
        Duration::from_secs(40),
        &mut restarted_first,
        || {
            first_successor.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2 && record.fencing_token == 2 && record.authority_peer_id == second_successor_id
            })
        },
    );
    assert!(permit_generation(first_successor, world).is_none());
    restarted_first.stop();

    let mut restarted_a = spawn_daemon(&a, &authority_bootstrap, false);
    wait_until_with_daemon(
        "original stale authority adopting accepted recovery",
        Duration::from_secs(40),
        &mut restarted_a,
        || {
            a.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2 && record.fencing_token == 2 && record.authority_peer_id == second_successor_id
            })
        },
    );
    assert!(permit_generation(&a, world).is_none());
    restarted_a.stop();
'''
new = r'''    let remaining = survivors
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != first_index)
        .map(|(_, peer)| *peer)
        .collect::<Vec<_>>();

    // The round-one certificate is already a chosen value for target generation 2.
    // While that certified candidate is down, a later proposer may raise the round,
    // but it must not switch the candidate and commit a conflicting same-generation
    // Recovery epoch. Safety deliberately wins over same-generation failover here.
    thread::sleep(Duration::from_secs(20));
    for peer in &remaining {
        let record = peer.storage.load_epoch_record(world).unwrap();
        assert_eq!(record.epoch_number, 1);
        assert_eq!(record.fencing_token, 1);
        assert_eq!(record.authority_peer_id, a.identity.peer_id());
        assert!(permit_generation(peer, world).is_none());
        if let Ok(certificate) = peer.storage.load_recovery_certificate(world) {
            assert_eq!(certificate.ballot.candidate_peer_id, first_successor_id);
        }
    }

    // Resume the candidate that actually owns the chosen certificate. Its durable
    // certificate must be sufficient to finish the exact value it previously won,
    // and every live voter must converge on that one Recovery epoch.
    let remaining_addrs = remaining.iter().map(|peer| transport_address(peer)).collect::<Vec<_>>();
    let mut restarted_first = spawn_daemon(first_successor, &remaining_addrs, false);
    wait_until_with_daemon(
        "certified first successor resuming chosen recovery value",
        Duration::from_secs(40),
        &mut restarted_first,
        || {
            first_successor.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.mode == EpochMode::Recovery
                    && record.authority_peer_id == first_successor_id
            })
        },
    );
    wait_until("remaining voters adopting the chosen recovery value", Duration::from_secs(40), || {
        remaining.iter().all(|peer| {
            peer.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.mode == EpochMode::Recovery
                    && record.authority_peer_id == first_successor_id
            })
        })
    });
    wait_until("resumed certified successor live permit", Duration::from_secs(30), || {
        permit_generation(first_successor, world)
            .is_some_and(|(epoch, fencing, heartbeat)| epoch == 2 && fencing == 2 && heartbeat >= 2)
    });
    for peer in &remaining {
        assert!(permit_generation(peer, world).is_none());
    }

    let authority_addr = transport_address(first_successor);
    let authority_bootstrap = vec![authority_addr];
    let mut restarted_a = spawn_daemon(&a, &authority_bootstrap, false);
    wait_until_with_daemon(
        "original stale authority adopting the chosen recovery value",
        Duration::from_secs(40),
        &mut restarted_a,
        || {
            a.storage.load_epoch_record(world).is_ok_and(|record| {
                record.epoch_number == 2
                    && record.fencing_token == 2
                    && record.authority_peer_id == first_successor_id
            })
        },
    );
    assert!(permit_generation(&a, world).is_none());
    restarted_a.stop();
    restarted_first.stop();
'''
if text.count(old) != 1:
    raise SystemExit(f"recovery successor tail mismatch: expected 1, found {text.count(old)}")
text = text.replace(old, new, 1)
text = text.replace(
    "fn newer_successor_recovers_after_first_successor_dies_with_durable_votes() {",
    "fn formed_recovery_certificate_locks_value_until_certified_candidate_resumes() {",
    1,
)
path.write_text(text)
