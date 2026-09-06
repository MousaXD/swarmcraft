from pathlib import Path

path = Path('implementation/agent-4-network.md')
text = path.read_text()

old = '''## Remaining work

1. Add the canonical non-omittable current-authority/current-head proof primitive in the consensus/protocol/storage trust model.
2. Re-consume that primitive on Agent 4.
3. Bind public discovery announcements to that proof.
4. Add malicious self-signed provider, stale former authority, removed/banned member, malformed proof, wrong-world/history, and replay-after-transition tests for public browse and exact resolve.
5. Re-run Agent 4 exact-head validation and only then mark READY FOR INTEGRATION.
'''
new = '''## Historical FINAL-028 remaining work — RESOLVED

The items below were open at the earlier composed-validation stage and are retained only as audit lineage. The canonical final closure later in this ledger resolves all of them:

1. Canonical current-authority/current-head freshness primitive — RESOLVED by verifier-interactive FINAL-028 freshness proof.
2. Agent 4 consumption of that primitive — RESOLVED in the final production milestone.
3. Public discovery binding — RESOLVED for browse and exact resolve.
4. Malicious/stale/malformed/wrong-history/replay acceptance regressions — RESOLVED and passing.
5. Exact-head revalidation — RESOLVED by run `33931301852` on Linux, Windows, and macOS.
'''
if old not in text:
    raise SystemExit('historical remaining-work block not found')
text = text.replace(old, new, 1)

replacements = [
    (
        'Validated composed production SHA: `7f151439418833d89fe0e4fd3c961878c0b51093`',
        'Historical composed production SHA: `7f151439418833d89fe0e4fd3c961878c0b51093`',
    ),
    (
        'Exact validation run: `33760654684` SUCCESS',
        'Historical composed validation run: `33760654684` SUCCESS',
    ),
    (
        'Blocker: FINAL-028 cannot be closed safely until the canonical trust model provides a first-contact-verifiable, non-omittable current authority/current-head proof across legitimate membership/authority transitions.',
        'Historical blocker at that stage: FINAL-028 lacked a first-contact-verifiable current authority/current-head proof. This blocker is RESOLVED by the canonical final closure below.',
    ),
    (
        '- Integration head remained `f02bb0d54cb44df67e730f01be4c903e25d670ff`.',
        '- At that earlier composed-validation stage, integration head remained `f02bb0d54cb44df67e730f01be4c903e25d670ff`; the later FINAL-028 closure consumed authoritative Agent 1+2+3 ancestor `c9252820a560e6ed4d30bb77227e3a494c6ce869`.',
    ),
]
for old_value, new_value in replacements:
    if old_value not in text:
        raise SystemExit(f'missing historical ledger anchor: {old_value[:100]}')
    text = text.replace(old_value, new_value, 1)

path.write_text(text)
print('Agent 4 historical ledger wording reconciled without functional changes')
