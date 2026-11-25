import test from 'node:test';
import assert from 'node:assert/strict';
import { formatDiscoveryResult, withDiscoveryDeadline } from '../src/discovery-diagnostics.js';

test('exact lookup distinguishes directory, network, stale, and missing results without leaking details', () => {
  for (const [state, text] of [['provider_unavailable', 'directory unavailable'], ['network_unavailable', 'network unavailable'], ['not_found', 'No current signed'], ['stale', 'is stale'], ['invalid', 'could not be verified']]) {
    const result = formatDiscoveryResult({state, detail: 'secret token and address'}, 'world');
    assert.equal(result.tone, 'warning');
    assert.ok(result.message.includes(text));
    assert.ok(!result.copy.includes('secret token'));
    assert.ok(result.copy.includes('does not grant membership'));
  }
});

test('successful discovery is exact-world, unexpired, and safe to copy', () => {
  const now = 1000;
  const report = {state: 'found', world: {world_id: 'world', name: 'private name', visibility: 'public', expires_unix_ms: now+1}};
  const result = formatDiscoveryResult(report, 'world', now);
  assert.equal(result.tone, 'safe');
  assert.equal(result.validUntil, now+1);
  assert.ok(!result.copy.includes('private name'));
  assert.equal(formatDiscoveryResult(report, 'other', now).tone, 'warning');
  assert.equal(formatDiscoveryResult(report, 'world', now+1).tone, 'warning');
});

test('unresponsive native discovery has a bounded diagnostic outcome', async () => {
  assert.deepEqual(await withDiscoveryDeadline(new Promise(() => {}), 5), {state:'timeout'});
  assert.equal(await withDiscoveryDeadline(Promise.resolve('ok'), 50), 'ok');
});
