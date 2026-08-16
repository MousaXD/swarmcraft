from pathlib import Path

path = Path("crates/swarm-cli/tests/recovery_successor_dies.rs")
text = path.read_text()
text = text.replace('.env("RUST_LOG", "info")', '.env("RUST_LOG", "warn")', 1)
old = '''    let remaining_addrs = remaining.iter().map(|peer| transport_address(peer)).collect::<Vec<_>>();
    let mut restarted_first = spawn_daemon(first_successor, &remaining_addrs, false);
'''
new = '''    let authority_addr = transport_address(second_successor);
    let authority_bootstrap = vec![authority_addr];
    let mut restarted_first = spawn_daemon(first_successor, &authority_bootstrap, false);
'''
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise SystemExit("stale successor bootstrap block not found")
old_a = '''    let mut restarted_a = spawn_daemon(&a, &remaining_addrs, false);
'''
new_a = '''    let mut restarted_a = spawn_daemon(&a, &authority_bootstrap, false);
'''
if old_a in text:
    text = text.replace(old_a, new_a, 1)
elif new_a not in text:
    raise SystemExit("stale original authority bootstrap block not found")
path.write_text(text)
