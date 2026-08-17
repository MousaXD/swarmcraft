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

test('migration state translates migration-core phases into launcher phases', () => {
  for (const phase of MIGRATION_PHASES) {
    const state = normalizeMigrationState({ phase });
    assert.equal(state.phase, phase);
    assert.equal(state.available, true);
  }
  assert.equal(normalizeMigrationState({ phase: 'waiting_for_authority' }).phase, 'preparing-successor');
  assert.equal(normalizeMigrationState({ phase: 'launching_runtime' }).phase, 'starting-minecraft');
  assert.equal(normalizeMigrationState({ phase: 'verifying_fabric' }).phase, 'waiting-for-host');
  assert.equal(normalizeMigrationState({ phase: 'checkpointing' }).phase, 'saving-world');
  assert.equal(normalizeMigrationState({ phase: 'awaiting_transfer_acceptance' }).phase, 'transferring-authority');
  assert.equal(normalizeMigrationState({ phase: 'sleeping' }).available, false);
  assert.equal(normalizeMigrationState({ phase: 'blocked', failure_reason: 'runtime config missing' }).label, 'Action required');
  const ready = normalizeMigrationState({ phase: 'ready', runtime_ready: true, game_endpoint: '127.0.0.1:25565' });
  assert.equal(ready.detail, 'Minecraft is ready at 127.0.0.1:25565.');
  assert.equal(ready.progress, 100);
  assert.equal(ready.runtimeReady, true);
  assert.equal(ready.gameEndpoint, '127.0.0.1:25565');
  const unknown = normalizeMigrationState({ phase: 'elected-but-maybe-ready' });
  assert.equal(unknown.available, false);
  assert.equal(unknown.phase, null);
});

test('migration status and wake probe real backend capability while transfer stays multi-stage', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push([command, payload]);
    if (command === 'migration_capabilities') return 'status,wake';
    if (command === 'migration_status') {
      return JSON.stringify({ phase: 'waiting_for_authority', runtime_ready: false, game_endpoint: null });
    }
    return 'ok';
  });
  assert.deepEqual(backend.migration.capabilities, { status: false, transfer: false, wake: false });
  await backend.listWorlds();
  assert.deepEqual(backend.migration.capabilities, { status: true, transfer: false, wake: true });
  const state = await backend.migration.readState('scworld:test');
  assert.equal(state.phase, 'waiting_for_authority');
  await backend.migration.wakeWorld('scworld:test');
  await assert.rejects(backend.migration.transferAuthority('scworld:test'), /not available in this build/i);
  assert.deepEqual(calls.map(([command]) => command), ['migration_capabilities', 'list_worlds', 'migration_status', 'wake_world']);
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
  assert.doesNotMatch(html, />Sleep world/);
  assert.match(html, /does not itself create a durable sleeping migration state/);
  assert.match(html, /aria-live="polite"/);
  assert.match(html, /<label for="createName">/);
  assert.match(html, /<label for="joinInvite">/);
});

test('frontend source and Tauri config preserve the global bridge contract', async () => {
  const app = await text('app.js');
  const tauri = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  assert.match(app, /window\.__TAURI__\?\.core\?\.invoke/);
  assert.match(app, /selectedWorldId === requestedWorldId/);
  assert.match(app, /document\.hidden/);
  assert.match(app, /setInterval\(refreshVisibleMigration, MIGRATION_REFRESH_MS\)/);
  assert.match(app, /!migration\.available \|\| migration\.failed/);
  assert.match(app, /renderMigration\(\{ detail: `Could not read host migration state:/);
  assert.equal(tauri.app.withGlobalTauri, true);
});
