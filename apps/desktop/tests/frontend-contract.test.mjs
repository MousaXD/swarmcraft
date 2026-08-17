import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { connectivityFromStatus, createBackendAdapter, MIGRATION_PHASES, normalizeMigrationState } from '../src/backend-adapter.js';

const srcRoot = new URL('../src/', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

function connectivity(state, recent_failures = []) {
  return { 'Connectivity JSON': JSON.stringify({ state, recent_failures }) };
}

test('connectivity parser consumes typed daemon JSON and never guesses from prose', () => {
  assert.equal(connectivityFromStatus(connectivity('DirectReachable')).label, 'Direct');
  assert.equal(connectivityFromStatus(connectivity('HolePunched')).label, 'Direct · hole punched');
  assert.equal(connectivityFromStatus(connectivity('RelayConnected')).label, 'Relay');
  assert.equal(connectivityFromStatus(connectivity('RelayRequired')).label, 'Relay required');
  assert.equal(connectivityFromStatus(connectivity('NoViablePath')).label, 'No viable path');
  const requestFailure = connectivityFromStatus(connectivity('NatStatusUnknown', [
    { kind: 'RequestFailed', detail: 'request timed out' },
  ]));
  assert.match(requestFailure.detail, /RequestFailed/);
  assert.match(requestFailure.detail, /request timed out/);
  assert.equal(connectivityFromStatus({ Connectivity: 'Direct' }).label, 'Not reported');
  assert.equal(connectivityFromStatus({ 'Network path': 'Circuit relay' }).label, 'Not reported');
  assert.equal(connectivityFromStatus({}).label, 'Not reported');
});

test('world status carries the daemon connectivity JSON without text classification', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push([command, payload]);
    if (command === 'world_status') return 'World: Test\nSafety: Canonical';
    if (command === 'connectivity_diagnostics') {
      return JSON.stringify({ state: 'DirectReachable', recent_failures: [] });
    }
    return 'ok';
  });
  const status = await backend.worldStatus('scworld:test');
  assert.match(status, /Connectivity JSON: \{"state":"DirectReachable"/);
  assert.deepEqual(calls.map(([command]) => command), ['world_status', 'connectivity_diagnostics']);
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
  await backend.configureWorldRuntime({ world: 'scworld:test', java: 'java', serverJar: '/server.jar', modJar: '/mod.jar', acceptEula: true });
  await backend.hostWorld({ world: 'scworld:test', java: 'java', serverJar: '/server.jar', modJar: '/mod.jar', acceptEula: true });
  assert.equal(calls[0].command, 'create_world');
  assert.equal(calls[0].payload.fabricLoader, '0.19.3');
  assert.equal(calls[1].command, 'create_invite');
  assert.equal(calls[1].payload.expiresMinutes, 60);
  assert.equal(calls[2].command, 'configure_world_runtime');
  assert.equal(calls[2].payload.acceptEula, true);
  assert.equal(calls[3].command, 'host_world');
  assert.equal(calls[3].payload.acceptEula, true);
});

test('Tauri bridge auto-starts networking and persists runtime configuration before host spawn', async () => {
  const main = await readFile(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
  const runtime = await readFile(new URL('../src-tauri/src/runtime.rs', import.meta.url), 'utf8');
  assert.match(main, /\.setup\(\|app\|/);
  assert.match(main, /\.start_daemon\(app\.handle\(\), DEFAULT_DAEMON_LISTEN\.into\(\)\)/);
  assert.match(main, /async fn configure_world_runtime/);
  assert.match(main, /"runtime-configure"\.into\(\)/);
  assert.match(main, /configure_world_runtime_impl\([\s\S]*?\)\s*\.await\?;[\s\S]*?processes\.start_host/);
  assert.match(main, /fn connectivity_diagnostics/);
  assert.match(runtime, /SWARMCRAFT_CONNECTIVITY_JSON/);
  assert.match(runtime, /connectivity_diagnostics/);
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
