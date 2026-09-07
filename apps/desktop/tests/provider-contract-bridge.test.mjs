import test from 'node:test';
import assert from 'node:assert/strict';
import { adaptProviderInvocation, installProviderContractBridge } from '../src/provider-contract-bridge.js';

test('opaque Modrinth staging sessions use the Agent 5 native payload without exposing a destination path', () => {
  const adapted = adaptProviderInvocation('modrinth_download', {
    request: {
      locator: { project_id: 'lithium', version_id: '1', filename: 'lithium.jar' },
      destination_dir: 'desktop-123-456/modrinth/lithium/1',
      max_bytes: 42,
    },
  });
  assert.deepEqual(adapted, {
    command: 'modrinth_download',
    args: {
      locator: { project_id: 'lithium', version_id: '1', filename: 'lithium.jar' },
      stagingSession: 'desktop-123-456',
      maxBytes: 42,
    },
  });
  assert.equal(JSON.stringify(adapted).includes('destination'), false);
});

test('opaque CurseForge staging sessions use the Agent 5 native payload without exposing a destination path', () => {
  const adapted = adaptProviderInvocation('curseforge_download', {
    fileId: 99,
    destination: 'desktop-123-456/curseforge/10/99/example.jar',
  });
  assert.deepEqual(adapted, {
    command: 'curseforge_download',
    args: { fileId: 99, stagingSession: 'desktop-123-456' },
  });
  assert.equal(JSON.stringify(adapted).includes('destination'), false);
});

test('current path-based provider payloads remain untouched before Agent 5 backend integration', () => {
  const modrinth = {
    request: {
      locator: { project_id: 'lithium', version_id: '1' },
      destination_dir: '/tmp/swarmcraft/provider-staging/desktop-123/modrinth/lithium/1',
      max_bytes: null,
    },
  };
  const curseforge = {
    fileId: 99,
    destination: 'C:\\Users\\player\\SwarmCraft\\provider-staging\\desktop-123\\curseforge\\10\\99\\example.jar',
  };
  assert.deepEqual(adaptProviderInvocation('modrinth_download', modrinth), { command: 'modrinth_download', args: modrinth });
  assert.deepEqual(adaptProviderInvocation('curseforge_download', curseforge), { command: 'curseforge_download', args: curseforge });
});

test('bridge rewrites only recognized opaque-session provider download calls', async () => {
  const calls = [];
  const target = {
    __TAURI__: {
      core: {
        invoke(command, args) {
          calls.push({ command, args });
          return Promise.resolve('ok');
        },
      },
    },
  };
  assert.equal(installProviderContractBridge(target), true);
  assert.equal(installProviderContractBridge(target), false);

  await target.__TAURI__.core.invoke('modrinth_download', {
    request: {
      locator: { project_id: 'lithium', version_id: '1' },
      destination_dir: 'desktop-abc-123/modrinth/lithium/1',
      max_bytes: null,
    },
  });
  await target.__TAURI__.core.invoke('minecraft_versions', { includeSnapshots: false, refresh: false });

  assert.deepEqual(calls[0], {
    command: 'modrinth_download',
    args: {
      locator: { project_id: 'lithium', version_id: '1' },
      stagingSession: 'desktop-abc-123',
      maxBytes: null,
    },
  });
  assert.deepEqual(calls[1], {
    command: 'minecraft_versions',
    args: { includeSnapshots: false, refresh: false },
  });
});
