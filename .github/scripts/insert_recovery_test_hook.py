from pathlib import Path

path = Path("crates/swarm-network/src/node.rs")
text = path.read_text()
old = "*previous != connection_id && num_established > 1"
new = "*previous != connection_id && num_established.get() > 1"
if old in text:
    path.write_text(text.replace(old, new, 1))
elif new not in text:
    raise SystemExit("connection count comparison not found")
