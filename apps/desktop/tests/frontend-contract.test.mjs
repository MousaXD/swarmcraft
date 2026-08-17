import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  connectivityFromStatus,
  createBackendAdapter,
  MIGRATION_PHASES,
  normalizeConnectivityDiagnostics,
  normalizeHostReadiness,
  normalizeMigrationState,
} from '../src/backend-adapter.js';

const srcRoot = new URL('../src/', import.meta.url);
const desktopRoot = new URL('../', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

async function desktopText(name) {
  return readFile(new URL(name, desktopRoot), 'utf8');
}

test('runtime configuration is persisted before direct host launch', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push({ command, payload });
    if (command === 'configure_world_runtime') return 'configured';
    if (command === 'host_world') return 4242;
    return 'ok';
  });

  const payload = {
    world: 'scworld:test',
    java: 'java',
    serverJar: '/server.jar',
    modJar: '/mod.jar',
    acceptEula: true,
  };
  const pid = await backend.hostWorld(payload);

  assert.equal(pid, 4242);
  assert.deepEqual(calls.map(({ command }) => command), ['configure_world_runtime', 'host_world']);
  assert.deepEqual(calls[0].payload, { ...payload, gameEndpoint: null });
});

test('failed runtime persistence never silently proceeds as failover-ready', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command) => {
    calls.push(command);
    if (command === 'configure_world_runtime') throw new Error('runtime config write failed');
    return 4242;
  });

  await assert.rejects(
    backend.hostWorld({
      world: 'scworld:test',
      java: 'java',
      serverJar: '/server.jar',
      modJar: '/mod.jar',
      acceptEula: true,
    }),
    /hosting was not started and automatic takeover is not prepared/i,
  );
  assert.deepEqual(calls, ['configure_world_runtime']);
});

test('startup automatically initializes the node and ensures the daemon before loading worlds', async () => {
  const app = await text('app.js');
  const startup = app.match(/async function startup\(\) \{([\s\S]*?)\n\}/)?.[1] || '';
  assert.match(startup, /backend\.initializeNode\(\)/);
  assert.match(startup, /ensureNetworkingService\(\{ quiet: true \}\)/);
  assert.match(startup, /refreshWorlds\(\)/);
  assert.ok(startup.indexOf('backend.initializeNode()') < startup.indexOf('ensureNetworkingService({ quiet: true })'));
  assert.ok(startup.indexOf('ensureNetworkingService({ quiet: true })') < startup.indexOf('refreshWorlds()'));
  assert.match(app, /startup\(\)\.catch/);
});

test('daemon ownership is idempotent and stale termination cannot clear a newer child', async () => {
  const runtime = await desktopText('src-tauri/src/runtime.rs');
  assert.match(runtime, /if let Some\(pid\) = guard\.existing_pid\(CommandChild::pid\) \{\s*return Ok\(pid\);/);
  assert.match(runtime, /clear_if_generation\(generation\)/);
  assert.match(runtime, /stale_termination_cannot_clear_newer_owned_process/);
  assert.match(runtime, /existing_owned_process_is_idempotent/);
});

test('Desktop stop path cannot target an unrelated external daemon', async () => {
  const runtime = await desktopText('src-tauri/src/runtime.rs');
  assert.match(runtime, /is not owned by this Desktop process/);
  assert.match(runtime, /stop_path_has_no_handle_for_external_processes/);
  assert.doesNotMatch(runtime, /pkill|taskkill|killall/);
});

test('structured connectivity JSON maps to player-facing labels without treating unknown NAT as offline', () => {
  const expectations = new Map([
    ['nat_status_unknown', 'Checking connectivity'],
    ['direct_reachable', 'Direct connection'],
    ['hole_punched', 'Direct connection established'],
    ['relay_connected', 'Connected through relay'],
    ['relay_required', 'Relay needed'],
    ['private_unreachable', 'Connection needs attention'],
    ['bootstrap_unavailable', 'Discovery unavailable'],
    ['no_viable_path', 'Could not reach other peers'],
  ]);
  for (const [state, label] of expectations) {
    assert.equal(normalizeConnectivityDiagnostics({ state }).label, label);
  }
  const unknown = normalizeConnectivityDiagnostics({ state: 'nat_status_unknown', nat_status: 'unknown' });
  assert.notEqual(unknown.label.toLowerCase(), 'offline');
  assert.match(unknown.diagnosticDetail, /NAT: unknown/);
  const direct = normalizeConnectivityDiagnostics({
    state: 'direct_reachable',
    nat_status: 'public',
    local_addresses: ['/ip4/0.0.0.0/udp/4000/quic-v1'],
    observed_public_address: '/ip4/1.2.3.4/udp/4000/quic-v1',
    selected_relay: '/dns4/relay.example/udp/4001/quic-v1/p2p/relay',
    last_failure: 'earlier direct dial timeout',
    recent_failures: [{ kind: 'direct_dial_failed', peer: 'peer-a', address: null, detail: 'timeout' }],
  });
  assert.equal(direct.label, 'Direct connection');
  assert.match(direct.diagnosticDetail, /NAT: public/);
  assert.match(direct.diagnosticDetail, /Local addresses: \/ip4\/0\.0\.0\.0/);
  assert.match(direct.diagnosticDetail, /Observed public address: \/ip4\/1\.2\.3\.4/);
  assert.match(direct.diagnosticDetail, /Recent failures: direct dial failed: timeout: peer peer-a/);
  assert.doesNotMatch(direct.diagnosticDetail, /\[object Object\]/);
  assert.equal(connectivityFromStatus({ Connectivity: 'Direct' }).label, 'Checking connectivity');
});

test('connectivity adapter requires machine-readable JSON', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command) => {
    calls.push(command);
    return JSON.stringify({ state: 'relay_connected', nat_status: 'private', local_addresses: ['/ip4/1.2.3.4/udp/1/quic-v1'] });
  });
  const state = await backend.connectivityDiagnostics();
  assert.equal(state.label, 'Connected through relay');
  assert.match(state.diagnosticDetail, /NAT: private/);
  assert.deepEqual(calls, ['connectivity_diagnostics']);
});

test('blocked migration due to missing runtime config is actionable and distinct from failed', async () => {
  const blocked = normalizeMigrationState({ phase: 'blocked', failure_reason: 'runtime config missing' });
  assert.equal(blocked.label, 'Action required');
  assert.equal(blocked.failed, false);
  assert.equal(blocked.blocked, true);
  assert.equal(blocked.action, 'runtime-setup');
  assert.equal(blocked.detail, 'This device needs Minecraft runtime setup before it can take over hosting.');

  const failed = normalizeMigrationState({ phase: 'failed', failure_reason: 'launch crashed' });
  assert.equal(failed.failed, true);
  assert.equal(failed.label, 'Migration failed');

  const app = await text('app.js');
  assert.match(app, /migrationRuntimeSetup/);
  assert.match(app, /Set up Minecraft runtime/);
  assert.match(app, /showRuntimeValidation/);
});

test('stale migration requests cannot overwrite the newly selected world', async () => {
  const app = await text('app.js');
  assert.match(app, /const requestGeneration = \+\+migrationRequestGeneration/);
  assert.match(app, /selectedWorldId === requestedWorldId && requestGeneration === migrationRequestGeneration/);
  assert.match(app, /migrationRequestGeneration \+= 1/);
});

test('manual transfer remains unavailable without one safe backend orchestration command', async () => {
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
  const ready = normalizeMigrationState({ phase: 'ready', runtime_ready: true, game_endpoint: '127.0.0.1:25565' });
  assert.equal(ready.detail, 'Minecraft is ready at 127.0.0.1:25565.');
  assert.equal(ready.progress, 100);
  assert.equal(ready.runtimeReady, true);
  assert.equal(ready.gameEndpoint, '127.0.0.1:25565');
  const unknown = normalizeMigrationState({ phase: 'elected-but-maybe-ready' });
  assert.equal(unknown.available, false);
  assert.equal(unknown.phase, null);
});

test('Tauri bridge exposes runtime persistence, daemon ensure, and structured connectivity commands', async () => {
  const main = await desktopText('src-tauri/src/main.rs');
  const runtimeCommands = await desktopText('src-tauri/src/runtime_commands.rs');
  assert.match(main, /async fn configure_world_runtime/);
  assert.match(main, /"runtime-configure"\.into\(\)/);
  assert.match(main, /"--accept-eula"\.into\(\)/);
  assert.match(main, /async fn connectivity_diagnostics/);
  assert.match(main, /"diagnostics"\.into\(\)[\s\S]*"connectivity"\.into\(\)[\s\S]*"--json"\.into\(\)/);
  assert.match(runtimeCommands, /pub fn ensure_daemon_running/);
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
  assert.match(app, /document\.hidden/);
  assert.match(app, /setInterval\(refreshVisibleMigration, MIGRATION_REFRESH_MS\)/);
  assert.match(app, /renderMigration\(\{ detail: `Could not read host migration state:/);
  assert.match(app, /Automatic networking service could not start/);
  assert.equal(tauri.app.withGlobalTauri, true);
});


test('host readiness adapter preserves backend safety distinctions', async () => {
  const safe = normalizeHostReadiness({
    state: 'safe',
    safe_to_shutdown: true,
    successor_peer_id: 'scpeer:bob',
    world_data_replicated: true,
    detail: 'Bob can take over.',
  });
  assert.equal(safe.label, 'Safe to shut down');
  assert.equal(safe.safeToShutdown, true);
  assert.equal(safe.successorPeerId, 'scpeer:bob');

  const quorum = normalizeHostReadiness({
    state: 'blocked_by_quorum',
    safe_to_shutdown: false,
    handoff_candidate_peer_id: 'scpeer:bob',
    detail: 'Transfer hosting before shutdown.',
  });
  assert.equal(quorum.label, 'Transfer hosting first');
  assert.equal(quorum.safeToShutdown, false);
  assert.equal(quorum.handoffCandidatePeerId, 'scpeer:bob');

  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push({ command, payload });
    return JSON.stringify({ state: 'blocked_by_mods', safe_to_shutdown: false, detail: 'Bob is missing required server mods.' });
  });
  const readiness = await backend.hostReadiness('scworld:test');
  assert.equal(readiness.label, 'Another host is missing mods');
  assert.deepEqual(calls, [{ command: 'host_readiness', payload: { world: 'scworld:test' } }]);
});

test('Tauri exposes host readiness as structured CLI JSON without frontend inference', async () => {
  const tauriMain = await desktopText('src-tauri/src/main.rs');
  assert.match(tauriMain, /async fn host_readiness/);
  assert.match(tauriMain, /"host-readiness"\.into\(\).*"--json"\.into\(\)/s);
  assert.match(tauriMain, /host_readiness,/);
});
