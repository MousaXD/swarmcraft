from pathlib import Path

p = Path("crates/swarm-cli/src/daemon.rs")
text = p.read_text()
old = '''        WireRequest::DiscoveryPublic { .. }\n        | WireRequest::DiscoveryResolve { .. }\n        | WireRequest::FriendPresence { .. } => {\n'''
new = '''        WireRequest::DiscoveryPublic { .. }\n        | WireRequest::DiscoveryResolve { .. }\n        | WireRequest::FriendPresence { .. }\n        | WireRequest::DiscoveryFreshnessContext { .. }\n        | WireRequest::DiscoveryFreshnessVote(_) => {\n'''
if old in text:
    p.write_text(text.replace(old, new, 1))
elif new not in text:
    raise SystemExit("missing daemon discovery endpoint dispatch anchor")
print("FINAL-028 daemon dispatch made exhaustive and fail-closed")
