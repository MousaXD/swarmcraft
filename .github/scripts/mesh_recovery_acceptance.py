from pathlib import Path

path = Path('crates/swarm-cli/tests/recovery_successor_dies.rs')
text = path.read_text()
old = '''    let a_addr = transport_address(&a);
    let survivors = [&b, &c, &d, &e];
    let first_successor_id = survivors.iter().map(|peer| peer.identity.peer_id()).min().unwrap();
    let first_index = survivors
        .iter()
        .position(|peer| peer.identity.peer_id() == first_successor_id)
        .unwrap();

    let mut daemon_a = spawn_daemon(&a, &[], false);
    thread::sleep(Duration::from_secs(1));
    let mut survivor_daemons = Vec::new();
    for (index, peer) in survivors.iter().enumerate() {
        survivor_daemons.push(spawn_daemon(
            peer,
            std::slice::from_ref(&a_addr),
            index == first_index,
        ));
        thread::sleep(Duration::from_millis(350));
    }
'''
new = '''    let a_addr = transport_address(&a);
    let survivors = [&b, &c, &d, &e];
    let survivor_addrs = survivors.iter().map(|peer| transport_address(peer)).collect::<Vec<_>>();
    let first_successor_id = survivors.iter().map(|peer| peer.identity.peer_id()).min().unwrap();
    let first_index = survivors
        .iter()
        .position(|peer| peer.identity.peer_id() == first_successor_id)
        .unwrap();

    let mut daemon_a = spawn_daemon(&a, &[], false);
    thread::sleep(Duration::from_secs(1));
    let mut survivor_daemons = Vec::new();
    for (index, peer) in survivors.iter().enumerate() {
        let mut bootstraps = vec![a_addr.clone()];
        bootstraps.extend(
            survivor_addrs
                .iter()
                .enumerate()
                .filter(|(candidate_index, _)| *candidate_index != index)
                .map(|(_, address)| address.clone()),
        );
        survivor_daemons.push(spawn_daemon(peer, &bootstraps, index == first_index));
        thread::sleep(Duration::from_millis(350));
    }
'''
if old not in text:
    raise SystemExit('missing five-daemon bootstrap block')
path.write_text(text.replace(old, new, 1))
Path(__file__).unlink()
