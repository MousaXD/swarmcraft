from pathlib import Path

path = Path('crates/swarm-cli/src/daemon.rs')
text = path.read_text()
old = '''    storage.save_recovery_certificate(&certificate)?;\n    let next = promote_recovery_epoch(storage, identity, previous, latest)?;'''
new = '''    storage.save_recovery_certificate(&certificate)?;\n    #[cfg(debug_assertions)]\n    if let Ok(delay_ms) = std::env::var("SWARMCRAFT_TEST_PAUSE_AFTER_RECOVERY_CERTIFICATE_MS") {\n        if let Ok(delay_ms) = delay_ms.parse::<u64>() {\n            std::thread::sleep(Duration::from_millis(delay_ms));\n        }\n    }\n    let next = promote_recovery_epoch(storage, identity, previous, latest)?;'''
if old not in text:
    raise SystemExit('missing recovery certificate promotion anchor')
path.write_text(text.replace(old, new, 1))
Path(__file__).unlink()
