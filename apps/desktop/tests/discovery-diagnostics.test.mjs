import test from 'node:test';
import assert from 'node:assert/strict';
import { formatDiscoveryResult, normalizeDiscoveryCard, normalizeWorldId, withDiscoveryDeadline } from '../src/discovery-diagnostics.js';

const worldId = 'scworld:' + 'a'.repeat(64);
const card = (now, overrides = {}) => ({
  world_id: worldId, name: 'private name', visibility: 'public',
  membership_policy: 'invite_only', join_action: 'invite_required',
  minecraft_version: '26.1.2', loader_id: 'fabric', loader_version: '0.19.3',
  issued_unix_ms: now - 1, verified_unix_ms: now, expires_unix_ms: now + 60_000,
  ...overrides,
});

test('exact lookup distinguishes directory, network, stale, and missing results without leaking details', () => {
  for (const [state, text] of [['provider_unavailable', 'directory unavailable'], ['network_unavailable', 'network unavailable'], ['not_found', 'No current signed'], ['stale', 'is stale'], ['invalid', 'could not be verified']]) {
    const result = formatDiscoveryResult({state, detail: 'secret token and address'}, worldId);
    assert.equal(result.tone, 'warning');
    assert.ok(result.message.includes(text));
    assert.ok(!result.copy.includes('secret token'));
    assert.ok(result.copy.includes('does not grant membership'));
  }
});

test('successful exact lookup has point-in-time verification, unlisted support and safe copy', () => {
  const now = 1000;
  const report = {state: 'found', world: card(now, {visibility: 'unlisted'})};
  const result = formatDiscoveryResult(report, worldId, now);
  assert.equal(result.tone, 'neutral');
  assert.equal(result.validUntil, now + 30_000);
  assert.match(result.message, /unlisted.*Verified signed announcement at.*listing expires at.*Signed invite required/);
  assert.ok(!result.copy.includes('private name'));
  assert.equal(formatDiscoveryResult(report, 'scworld:' + 'b'.repeat(64), now).tone, 'warning');
  assert.equal(formatDiscoveryResult(report, worldId, now+60_000).tone, 'warning');
});

test('renderer refuses private, unsupported membership, missing proof time and malformed lifetimes', () => {
  const now = 1000;
  for (const override of [
    {visibility:'private'}, {join_action:'join'}, {membership_policy:'open'}, {verified_unix_ms:undefined},
    {verified_unix_ms:now+1}, {expires_unix_ms:now}, {expires_unix_ms:now+600_001},
    {issued_unix_ms:now+60_001}, {world_id:'scworld:abc'},
  ]) assert.equal(normalizeDiscoveryCard(card(now, override), now), null);
  assert.equal(normalizeWorldId('scworld:' + 'A'.repeat(64)), worldId);
  assert.equal(normalizeWorldId('scinvite:token'), '');
  assert.equal(normalizeWorldId('scworld:abc'), '');
});

test('unresponsive native discovery has a bounded diagnostic outcome', async () => {
  assert.deepEqual(await withDiscoveryDeadline(new Promise(() => {}), 5), {state:'timeout'});
  assert.equal(await withDiscoveryDeadline(Promise.resolve('ok'), 50), 'ok');
});
