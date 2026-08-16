from pathlib import Path

path = Path("crates/swarm-cli/tests/recovery_successor_dies.rs")
text = path.read_text()
old = '.env("RUST_LOG", "warn")'
new = '.env("RUST_LOG", "info")'
if old in text:
    path.write_text(text.replace(old, new, 1))
elif new not in text:
    raise SystemExit("recovery acceptance RUST_LOG setting not found")
