import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { clientContinuityFromMigration } from '../src/client-continuity.js';

const WORLD = 'scworld:alice';
const READY = Object.freeze({
  world_id: WORLD,
  phase: 'ready',
  runtime_ready: true,
  authority_peer_id: 'peer-bob',
  epoch: 3,
  fencing_token: 3,
  snapshot_hash: 'current-canonical-snapshot',
  // This configured field is not a signed or verified Minecraft route.
  game_endpoint: 'stale.host.example:25565',
});

function classify(status, safety = 'Canonical') {
  return clientContinuityFromMigration(status, WORLD, safety);
}

test('ready local Fabric report never becomes a trusted Minecraft endpoint or automatic join', () => {
  const state = classify(READY);
  assert.equal(state.state, 'manual_reconnect_required');
  assert.equal(state.automaticReconnect, false);
  assert.equal(state.connectionTarget, null);
  assert.match(state.detail, /cannot yet authenticate a client-reachable game address/i);
  assert.doesNotMatch(JSON.stringify(state), /stale\.host\.example/);
});

test('wrong-world and incomplete generation reports fail closed even when ready is claimed', () => {
  for (const invalid of [
    { ...READY, world_id: 'scworld:carol' },
    { ...READY, world_id: undefined },
    { ...READY, epoch: undefined },
    { ...READY, epoch: -1 },
    { ...READY, fencing_token: null },
    { ...READY, snapshot_hash: '' },
    { ...READY, authority_peer_id: '' },
    { ...READY, runtime_ready: false },
  ]) {
    const state = classify(invalid);
    assert.notEqual(state.state, 'manual_reconnect_required');
    assert.equal(state.connectionTarget, null);
    assert.equal(state.automaticReconnect, false);
  }
});

test('current-looking runtime cannot override conflicting or reduced-safety world history', () => {
  for (const safety of ['Conflict', 'Divergent', 'SoloUnreplicated', 'DegradedSafety', 'Unknown', '']) {
    const state = classify(READY, safety);
    assert.notEqual(state.state, 'manual_reconnect_required');
    assert.equal(state.connectionTarget, null);
  }
});

test('stale host, failed recovery and missing quorum have explicit failure instructions', () => {
  const stale = classify({ ...READY, phase: 'superseded' });
  assert.equal(stale.state, 'superseded');
  assert.match(stale.detail, /do not reconnect to the old host/i);
  const failed = classify({ ...READY, phase: 'failed' });
  assert.equal(failed.state, 'failed');
  const blocked = classify({ ...READY, phase: 'blocked', failure_reason: 'missing majority quorum' });
  assert.equal(blocked.state, 'quorum_unavailable');
  assert.match(blocked.title, /quorum/i);
  for (const state of [stale, failed, blocked]) {
    assert.equal(state.automaticReconnect, false);
    assert.equal(state.connectionTarget, null);
  }
});

test('planned transfer, crash recovery, sleep, unknown and absent reports do not claim reconnection', () => {
  for (const phase of [
    'waiting_for_authority', 'waiting_for_quorum', 'checkpointing',
    'awaiting_transfer_acceptance', 'restoring_world', 'launching_runtime',
    'verifying_fabric',
  ]) {
    assert.equal(classify({ ...READY, phase }).state, 'waiting');
  }
  assert.equal(classify({ ...READY, phase: 'sleeping' }).state, 'sleeping');
  assert.equal(classify(null).state, 'unknown');
  assert.equal(classify({ ...READY, phase: 'unrecognized' }).state, 'unknown');
});

test('malicious or private game endpoints are never repeated in reconnection guidance', () => {
  for (const candidate of ['127.0.0.1:25565', '192.168.1.2:25565', '<script>alert(1)</script>', 'evil.example:25565']) {
    const state = classify({ ...READY, game_endpoint: candidate });
    assert.equal(state.connectionTarget, null);
    assert.equal(state.automaticReconnect, false);
    assert.ok(!state.detail.includes(candidate));
  }
});

test('Desktop surfaces an accessible connection-status result bound to selected world', async () => {
  const root = new URL('../src/', import.meta.url);
  const app = await readFile(new URL('app.js', root), 'utf8');
  const html = await readFile(new URL('index.html', root), 'utf8');
  assert.match(app, /clientContinuityFromMigration\(state, selectedWorldId, selectedWorld\(\)\?\.status\?\.Safety\)/);
  assert.match(app, /\$\('clientContinuityTitle'\)\.textContent/);
  assert.match(app, /\$\('clientContinuityDetail'\)\.textContent/);
  assert.match(app, /renderMigration\(null\);[\s\S]*?refreshMigrationState\(world\)/);
  assert.match(html, /id="clientContinuityTitle"/);
  assert.match(html, /id="clientContinuityDetail"[^>]*role="status"[^>]*aria-live="polite"/);
});
