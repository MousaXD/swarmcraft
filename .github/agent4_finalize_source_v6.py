from pathlib import Path

path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
text = path.read_text()
old = '''    let bootstraps = vec![b_address.clone(), c_address.clone(), a_address.clone(), x_address.clone()];
'''
new = '''    // Dial the stale and malformed locators first so the regression proves
    // noncanonical-first handling instead of depending on transport scheduling.
    let bootstraps = vec![a_address.clone(), x_address.clone(), b_address.clone(), c_address.clone()];
'''
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise SystemExit("missing adversarial bootstrap ordering anchor")
path.write_text(text)
print("FINAL-028 adversarial locator order is stale + malformed before current + voter")
