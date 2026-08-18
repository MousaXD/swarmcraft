import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  createBackendAdapter,
  normalizeRuntimeStatus,
  RUNTIME_COMPONENTS,
} from '../src/backend-adapter.js';

const srcRoot = new URL('../src/', import.meta.url);

async function source(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

test('runtime status stays machine-readable and preserves backend readiness', () => {
  const status = normalizeRuntimeStatus(JSON.stringify({
    state: 'ready',
    ready: true,
    eula_accepted: true,
    world_data_safe: true,
    retry_safe: true,
    components: {
      java: { state: 'managed', version: '21' },
      minecraft_server: { state: 'ready', version: '26.1.2' },
      fabric_loader: { state: 'installed', version: '0.19.3' },
      fabric_api: { state: 'ready', version: 'test' },
      swarmcraft_integration: { state: 'verified' },
      world_directories: { state: 'present' },
      server_mods: { state: 'compatible' },
    },
  }));

  assert.equal(status.ready, true);
  assert.equal(status.eulaAccepted, true);
  assert.equal(status.eulaRequired, false);
  assert.equal(status.worldDataSafe, true);
  assert.equal(status.retrySafe, true);
  assert.deepEqual(status.components.map(({ id }) => id), RUNTIME_COMPONENTS.map(([id]) => id));
  assert.ok(status.components.every(({ state }) => state === 'ready'));
});

test('EULA requirement is backend-reported and never silently accepted', () => {
  const status = normalizeRuntimeStatus({
    state: 'eula_required',
    eula_required: true,
    eula_accepted: false,
    components: { java: 'ready', minecraft_server: 'ready' },
  });
  assert.equal(status.ready, false);
  assert.equal(status.eulaRequired, true);
  assert.equal(status.eulaAccepted, false);
});

test('runtime adapter exposes the isolated automatic setup contract', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push({ command, payload });
    if (command === 'runtime_plan') return JSON.stringify({ actions: ['download_java'] });
    if (command === 'runtime_launch') return 4242;
    return JSON.stringify({ state: 'ready', ready: true, eula_accepted: payload?.acceptEula === true });
  });

  assert.equal((await backend.runtime.status('scworld:test')).ready, true);
  assert.deepEqual(await backend.runtime.plan('scworld:test'), { actions: ['download_java'] });
  await backend.runtime.install('scworld:test', { acceptEula: true });
  await backend.runtime.repair('scworld:test');
  await backend.runtime.verify('scworld:test');
  assert.equal(await backend.runtime.launch('scworld:test'), 4242);

  assert.deepEqual(calls, [
    { command: 'runtime_status', payload: { world: 'scworld:test' } },
    { command: 'runtime_plan', payload: { world: 'scworld:test' } },
    { command: 'runtime_install', payload: { world: 'scworld:test', acceptEula: true } },
    { command: 'runtime_repair', payload: { world: 'scworld:test' } },
    { command: 'runtime_verify', payload: { world: 'scworld:test' } },
    { command: 'runtime_launch', payload: { world: 'scworld:test' } },
  ]);
});

test('missing runtime commands become explicit unavailable capability errors', async () => {
  const backend = createBackendAdapter(async (command) => {
    throw new Error(`Command ${command} not found`);
  });
  await assert.rejects(
    backend.runtime.status('scworld:test'),
    (error) => error?.code === 'SWARMCRAFT_CAPABILITY_UNAVAILABLE' && /not available in this build/i.test(error.message),
  );
});

test('Play flow is wizard-first only when setup is missing, but ready runtimes launch directly', async () => {
  const wizard = await source('runtime-wizard.js');
  assert.match(wizard, /const status = await backend\.runtime\.status\(world\.id\)/);
  assert.match(wizard, /if \(status\.ready\) \{\s*await launchPreparedWorld/);
  assert.match(wizard, /dialog\.showModal\(\)/);
  assert.match(wizard, /backend\.runtime\.install\(world\.id, \{ acceptEula: acceptedEula \}\)/);
  assert.match(wizard, /backend\.runtime\.verify\(world\.id\)/);
  assert.match(wizard, /backend\.runtime\.launch\(world\.id\)/);
});

test('wizard exposes explicit EULA, retry safety, advanced fallback, progress, and keyboard restoration', async () => {
  const wizard = await source('runtime-wizard.js');
  for (const anchor of [
    'runtimeEulaAccept',
    'runtimeRetrySetup',
    'runtimeAdvancedSetup',
    'runtimeWorldSafety',
    'runtimeRetrySafety',
    'runtimeFailureDetails',
    'aria-live="polite"',
    'aria-busy',
  ]) {
    assert.match(wizard, new RegExp(anchor));
  }
  assert.match(wizard, /status\?\.retrySafe !== true/);
  assert.match(wizard, /latestStatus\?\.retrySafe !== true/);
  assert.match(wizard, /previousFocus\?\.focus/);
  assert.match(wizard, /dialog\.addEventListener\('cancel'/);
  assert.match(wizard, /pollRuntimeStatus/);
  assert.match(wizard, /Automatic Minecraft setup is not available in this build/);
  assert.doesNotMatch(wizard, /download\([^)]*https?:\/\//i);
});
