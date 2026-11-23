import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createBackendAdapter, normalizeHostReadiness } from '../src/backend-adapter.js';

const WORLD = 'scworld:1111111111111111111111111111111111111111111111111111111111111111';
const OTHER = 'scworld:2222222222222222222222222222222222222222222222222222222222222';
const NOW = 2_000_000;
const SUCCESSOR = `scpeer:${'a'.repeat(64)}`;
const CANDIDATE = `scpeer:${'b'.repeat(64)}`;
const report = () => ({
  world_id: WORLD, state: 'safe', safe_to_shutdown: true,
  generated_unix_ms: NOW - 200, detail: 'another host is reported ready',
  successor_peer_id: SUCCESSOR, handoff_candidate_peer_id: CANDIDATE,
  world_data_replicated: true, peers: [{ peer_id: SUCCESSOR }],
});

test('positive shutdown safety needs a fresh, matching world report and affirmative flag', () => {
  const current = normalizeHostReadiness(report(), WORLD, NOW);
  assert.equal(current.kind, 'safe');
  assert.equal(current.worldId, WORLD);
  assert.equal(current.validUntil, NOW + 5800);
  assert.equal(normalizeHostReadiness(report(), WORLD, NOW).successorPeerId, SUCCESSOR);
  for (const invalid of [
    { ...report(), safe_to_shutdown: false },
    { ...report(), safe_to_shutdown: undefined },
    { ...report(), generated_unix_ms: NOW - 7000 },
    { ...report(), generated_unix_ms: NOW - 6000 },
    { ...report(), generated_unix_ms: NOW + 4000 },
    { ...report(), generated_unix_ms: null },
    { ...report(), world_id: OTHER },
    { ...report(), world_id: undefined },
    { ...report(), state: 'unknown' },
  ]) {
    const state = normalizeHostReadiness(invalid, WORLD, NOW);
    assert.equal(state.safeToShutdown, false);
    assert.notEqual(state.kind, 'safe');
    assert.doesNotMatch(state.label, /safe to shut down/i);
    assert.equal(state.validUntil, null);
    assert.equal(state.worldId, null);
    assert.equal(state.successorPeerId, null);
    assert.equal(state.handoffCandidatePeerId, null);
    assert.equal(state.worldDataReplicated, false);
    assert.deepEqual(state.peers, []);
    assert.deepEqual(state.raw, {});
  }
});

test('blocked transfer may inspect fresh peers but must never recommend them as a safe successor', async () => {
  const blocked = normalizeHostReadiness({ ...report(), state: 'blocked_by_quorum', safe_to_shutdown: false }, WORLD, NOW);
  assert.equal(blocked.safeToShutdown, false);
  assert.equal(blocked.successorPeerId, null);
  const wizard = await readFile(new URL('../src/transfer-wizard.js', import.meta.url), 'utf8');
  assert.match(wizard, /readiness\?\.safeToShutdown === true/);
  assert.match(wizard, /\? readiness\.successorPeerId \|\| ''/);
});

test('native host-readiness adapter binds the requested world instead of trusting a status label', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (name, payload) => {
    calls.push([name, payload]);
    return JSON.stringify({ ...report(), generated_unix_ms: Date.now() });
  });
  assert.equal((await backend.hostReadiness(WORLD)).safeToShutdown, true);
  assert.deepEqual(calls, [['host_readiness', { world: WORLD }]]);
  assert.equal((await backend.hostReadiness(OTHER)).safeToShutdown, false);
});

test('shipping renderer refuses a false safe shutdown state even after normalization', async () => {
  const source = await readFile(new URL('../src/app.js', import.meta.url), 'utf8');
  assert.match(source, /readiness\?\.safeToShutdown !== true/);
  assert.match(source, /readiness\.validUntil > Date\.now\(\)/);
  assert.match(source, /readiness\?\.worldId === selectedWorldId/);
  assert.match(source, /readiness\.validUntil - Date\.now\(\)/);
  assert.match(source, /function invalidateHostReadiness\(\)/);
  assert.match(source, /document\.addEventListener\('visibilitychange', refreshVisibleHostReadiness\)/);
});
