from pathlib import Path

p = Path("crates/swarm-cli/src/daemon.rs")
text = p.read_text()
old = '''        WireRequest::DiscoveryPublic { .. }\n        | WireRequest::DiscoveryResolve { .. }\n        | WireRequest::FriendPresence { .. } => {\n'''
new = '''        WireRequest::DiscoveryPublic { .. }\n        | WireRequest::DiscoveryResolve { .. }\n        | WireRequest::FriendPresence { .. }\n        | WireRequest::DiscoveryFreshnessContext { .. }\n        | WireRequest::DiscoveryFreshnessVote(_) => {\n'''
if old in text:
    p.write_text(text.replace(old, new, 1))
elif new not in text:
    raise SystemExit("missing daemon discovery endpoint dispatch anchor")

p = Path("crates/swarm-cli/src/discovery.rs")
text = p.read_text()
for signature in [
    "fn handle_discovery_request(\n",
    "pub fn validate_fresh_discovery_candidate(\n",
]:
    marker = "#[allow(clippy::too_many_arguments)]\n" + signature
    if marker in text:
        continue
    if signature not in text:
        raise SystemExit(f"missing FINAL-028 clippy anchor: {signature.strip()}")
    text = text.replace(signature, marker, 1)
p.write_text(text)

print("FINAL-028 daemon dispatch and security-boundary arity finalized")
