import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { connectivityFromStatus, createBackendAdapter, MIGRATION_PHASES, normalizeMigrationState } from '../src/backend-adapter.js';

const srcRoot = new URL('../src/', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

test('connectivity parser keeps player-facing transport states distinct', () => {
  assert.equal(connectivityFromStatus({ Connectivity: 'Direct' }).label, 'Direct');
  assert.equal(connectivityFromStatus({ 'Network path': 'Circuit relay' }).label, 'Relay');
  assert.equal(connectivityFromStatus({ Connectivity: 'connecting' }).label, 'Connecting');
  assert.equal(connectivityFromStatus({ Reachability: 'offline' }).label, 'Offline');
  assert.equal(connectivityFromStatus({ Connectivity: 'limited NAT reachability' }).label, 'Limited connectivity');
  assert.equal(connectivityFromStatus({ Connectivity: 'Limited connectivity' }).label, 'Limited connectivity');
  assert.equal(connectivityFromStatus({ Connectivity: 'NAT blocked - action required' }).label, 'Action required');
  assert.equal(connectivityFromStatus({}).label, 'Not reported');
});

test('migration state only accepts the prepared structured phases', () => {
  for (const phase of MIGRATION_PHASES) {
    const state = normalizeMigrationState({ phase });
    assert.equal(state.phase, phase);
    assert.equal(state.available, true);
  }
  const unknown = normalizeMigrationState({ phase: 'elected-but-maybe-ready' });
  assert.equal(unknown.available, false);
  assert.equal(unknown.phase, null);
});

test('migration operations remain capability-gated until migration-core is connected', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push([command, payload]);
    return 'ok';
  });
  assert.deepEqual(backend.migration.capabilities, { status: false, transfer: false, wake: false });
  await assert.rejects(backend.migration.transferAuthority('scworld:test'), /not available in this build/i);
  assert.equal(calls.length, 0);
});

test('existing Tauri command names and camelCase payloads stay intact', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push({ command, payload });
    return 'ok';
  });
  await backend.createWorld({ name: 'Test', minecraft: '26.1.2', fabricLoader: '0.19.3', compatibility: 'vanilla-fabric', visibility: 'private' });
  await backend.createInvite({ world: 'scworld:test', expiresMinutes: 60, bootstrapAddrs: [] });
  await backend.hostWorld({ world: 'scworld:test', java: 'java', serverJar: '/server.jar', modJar: '/mod.jar', acceptEula: true });
  assert.equal(calls[0].command, 'create_world');
  assert.equal(calls[0].payload.fabricLoader, '0.19.3');
  assert.equal(calls[1].command, 'create_invite');
  assert.equal(calls[1].payload.expiresMinutes, 60);
  assert.equal(calls[2].command, 'host_world');
  assert.equal(calls[2].payload.acceptEula, true);
});

test('launcher markup keeps critical flow and accessibility anchors', async () => {
  const html = await text('index.html');
  for (const id of [
    'createForm', 'joinForm', 'playWorld', 'inviteWorld', 'seedOn', 'seedOff', 'verifyWorld',
    'sleepWorld', 'transferHost', 'wakeWorld', 'diagnosticsView', 'leaveDialog', 'inviteDialog',
  ]) {
    assert.match(html, new RegExp(`id=["']${id}["']`), `missing #${id}`);
  }
  assert.match(html, /aria-live="polite"/);
  assert.match(html, /<label for="createName">/);
  assert.match(html, /<label for="joinInvite">/);
});

test('frontend source and Tauri config preserve the global bridge contract', async () => {
  const app = await text('app.js');
  const tauri = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  assert.match(app, /window\.__TAURI__\?\.core\?\.invoke/);
  assert.equal(tauri.app.withGlobalTauri, true);
});
