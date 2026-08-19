import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createBackendAdapter } from '../src/backend-adapter.js';
import { createImportRequest, parseImportResult } from '../src/import-flow.js';

const srcRoot = new URL('../src/', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

test('Import existing world is a visible normal launcher entry point', async () => {
  const html = await text('index.html');
  for (const id of ['openImport', 'quickImport', 'importPanel', 'importForm', 'importSource', 'importWorld']) {
    assert.match(html, new RegExp(`id=["']${id}["']`), `missing #${id}`);
  }
  assert.match(html, />Import existing world</);
});

test('import form validates required world metadata', () => {
  const base = {
    source: '/saves/world',
    name: 'Imported SMP',
    minecraft: '26.1.2',
    fabricLoader: '0.19.3',
    visibility: 'private',
    noServerMods: true,
  };
  for (const [key, message] of [
    ['source', /world folder is required/i],
    ['name', /display name is required/i],
    ['minecraft', /Minecraft version is required/i],
    ['fabricLoader', /Fabric Loader version is required/i],
  ]) {
    assert.throws(() => createImportRequest({ ...base, [key]: '' }), message);
  }
});

test('no-server-mods confirmation is required when no JARs are supplied', () => {
  assert.throws(
    () => createImportRequest({
      source: '/saves/world',
      name: 'Imported SMP',
      minecraft: '26.1.2',
      fabricLoader: '0.19.3',
      visibility: 'private',
      serverMods: '',
      noServerMods: false,
    }),
    /explicitly confirm/i,
  );
});

test('selected server-mod JARs pass through exactly and import never accepts EULA', () => {
  const request = createImportRequest({
    source: '/saves/world',
    name: 'Imported SMP',
    minecraft: '26.1.2',
    fabricLoader: '0.19.3',
    visibility: 'unlisted',
    serverMods: '/mods/lithium.jar\n/mods/ferritecore.jar\n',
    noServerMods: false,
  });
  assert.deepEqual(request.serverMods, ['/mods/lithium.jar', '/mods/ferritecore.jar']);
  assert.equal(Object.hasOwn(request, 'acceptEula'), false);
  assert.equal(Object.hasOwn(request, 'eula'), false);
});

test('backend import bridge forwards the explicit import contract without EULA state', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload) => {
    calls.push({ command, payload });
    return JSON.stringify({ world_id: 'scworld:imported' });
  });
  const request = createImportRequest({
    source: '/saves/world',
    name: 'Imported SMP',
    minecraft: '26.1.2',
    fabricLoader: '0.19.3',
    visibility: 'public',
    serverMods: ['/mods/lithium.jar'],
    noServerMods: false,
  });
  const result = await backend.importWorld(request);
  assert.equal(parseImportResult(result).worldId, 'scworld:imported');
  assert.deepEqual(calls, [{ command: 'import_world', payload: request }]);
  assert.equal(Object.hasOwn(calls[0].payload, 'acceptEula'), false);
});

test('successful import refreshes Worlds and selects the returned world', async () => {
  const app = await text('app.js');
  assert.match(app, /parseImportResult\(result\)/);
  assert.match(app, /selectedWorldId = importedWorldId/);
  assert.match(app, /await refreshWorlds\(\)/);
  assert.match(app, /showView\('worlds'\)/);
});

test('backend import failure is shown and cannot fall through to success refresh', async () => {
  const app = await text('app.js');
  assert.match(
    app,
    /Could not import world:[\s\S]*?return;[\s\S]*?parseImportResult\(result\)[\s\S]*?await refreshWorlds\(\)/,
  );
});

test('create, join, and managed runtime journeys remain wired alongside import', async () => {
  const html = await text('index.html');
  const app = await text('app.js');
  for (const id of ['createForm', 'joinForm', 'playWorld', 'runtimeSection']) {
    assert.match(html, new RegExp(`id=["']${id}["']`), `missing existing #${id}`);
  }
  assert.match(app, /bindAction\('createForm', createWorld/);
  assert.match(app, /bindAction\('joinForm', joinWorld/);
  assert.match(app, /bindAction\('playWorld', hostWorld/);
});
