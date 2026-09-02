from pathlib import Path

path = Path("crates/swarm-cli/src/daemon.rs")
text = path.read_text()
old = """            let promise = storage.load_membership_promise(world)?;\n            if promise.proposal.proposal_hash()? != proposal_hash\n"""
new = """            let Ok(promise) = storage.load_membership_promise(world) else {\n                // A certificate may have committed and cleared the durable prepare\n                // while this response was in flight. Late votes are then stale, not fatal.\n                return Ok(());\n            };\n            if promise.proposal.proposal_hash()? != proposal_hash\n"""
if text.count(old) != 1:
    raise SystemExit(f"expected exactly one late-vote response site, found {text.count(old)}")
path.write_text(text.replace(old, new, 1))
