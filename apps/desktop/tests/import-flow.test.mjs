import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createBackendAdapter } from '../src/backend-adapter.js';
import { createImportRequest, parseImportResult } from '../src/import-flow.js';
import { createImportReview, normalizeImportPreview } from '../src/import-review.js';

const srcRoot = new URL('../src/', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

test('Import existing world is a visible normal launcher entry point', async () => {
  const html = await text('index.html');
  for (const id of ['openImport', 'quickImport', 'importPanel', 'importForm', 'importSource', 'pickImportFolder', 'checkImport', 'importReview', 'importWorld']) {
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
  await backend.pickImportFolder();
  await backend.previewImport(request);
  const fingerprint = 'a'.repeat(64);
  const result = await backend.importWorld({ ...request, expectedCompatibility: fingerprint });
  assert.equal(parseImportResult(result).worldId, 'scworld:imported');
  assert.equal(calls[0].command, 'pick_import_folder');
  assert.deepEqual(calls.slice(1), [
    { command: 'preview_import_world', payload: request },
    { command: 'import_world', payload: { ...request, expectedCompatibility: fingerprint } },
  ]);
  assert.equal(Object.hasOwn(calls[2].payload, 'acceptEula'), false);
});

const previewRequest = createImportRequest({
  source: '/saves/world', name: 'Reviewed world', minecraft: '26.1.2',
  fabricLoader: '0.19.3', visibility: 'private', noServerMods: true,
});
const readyPreview = {
  state: 'ready', minecraft_version: '26.1.2', fabric_loader_version: '0.19.3',
  minimum_java_major: 25, source_files: 12, source_bytes: 2097152,
  required_server_mods: [], compatibility_fingerprint: 'a'.repeat(64),
};

test('native preview shows advisory source and runtime requirements without claiming eligibility', () => {
  const report = normalizeImportPreview(JSON.stringify(readyPreview), previewRequest);
  assert.equal(report.state, 'ready');
  assert.match(report.message, /Java 25/);
  assert.match(report.message, /2 MiB/);
  assert.match(report.message, /not inferred/);
  assert.match(report.message, /does not install Java, accept the server EULA, enable hosting/);
  assert.ok(!report.message.includes(previewRequest.source));
  for (const changes of [
    { compatibility_fingerprint: 'invalid' }, { minecraft_version: '26.2' },
    { source_files: 0 }, { source_bytes: -1 }, { required_server_mods: [{ artifact_id: 'unexpected', version: '1' }] },
  ]) {
    assert.throws(() => normalizeImportPreview({ ...readyPreview, ...changes }, previewRequest), /Invalid native/);
  }
  assert.match(normalizeImportPreview({ state: 'blocked', blocker: 'source_not_quiescent' }, previewRequest).message, /Close Minecraft/);
});

test('import review is single-use and stale asynchronous replies cannot authorize commit', () => {
  const review = createImportReview();
  assert.throws(() => review.take(previewRequest), /Check this import/);
  const first = review.begin(previewRequest);
  review.invalidate();
  assert.equal(review.finish(first, readyPreview, previewRequest), null);
  const second = review.begin(previewRequest);
  assert.equal(review.finish(second, readyPreview, { ...previewRequest, visibility: 'public' }), null);
  const third = review.begin(previewRequest);
  assert.equal(review.finish(third, readyPreview, previewRequest).state, 'ready');
  assert.throws(() => review.take({ ...previewRequest, source: '/different' }), /Check this import/);
  assert.equal(review.take(previewRequest), 'a'.repeat(64));
  assert.throws(() => review.take(previewRequest), /Check this import/);
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
    /Could not import world\.[\s\S]*?return;[\s\S]*?parseImportResult\(result\)[\s\S]*?await refreshWorlds\(\)/,
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
