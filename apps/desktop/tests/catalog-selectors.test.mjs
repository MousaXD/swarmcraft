import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  CatalogSelectionState,
  catalogErrorMessage,
  chooseFabricLoader,
  chooseMinecraftVersion,
  normalizeFabricCatalog,
  normalizeMinecraftCatalog,
} from '../src/catalog-selectors.js';

const minecraftResponse = {
  provider: 'mojang',
  origin: 'network',
  versions: [
    { id: '26.3-snapshot-1', type: 'snapshot', release_time: '2026-08-20T10:00:00+00:00', supported: true },
    { id: '26.2', type: 'release', release_time: '2026-06-23T10:00:00+00:00', supported: true },
  ],
};

const fabricResponse = {
  provider: 'fabric',
  origin: 'network',
  versions: [
    { version: '0.20.0-beta.1', stable: false, minecraft_version: '26.2' },
    { version: '0.19.3', stable: true, minecraft_version: '26.2' },
  ],
};

test('normal Minecraft default remains a stable release when snapshots are present', () => {
  const versions = normalizeMinecraftCatalog(minecraftResponse);
  assert.equal(chooseMinecraftVersion(versions), '26.2');
});

test('Fabric default prefers an official stable loader', () => {
  const loaders = normalizeFabricCatalog(fabricResponse, '26.2');
  assert.equal(chooseFabricLoader(loaders), '0.19.3');
});

test('changing Minecraft invalidates the prior Fabric selection before revalidation', () => {
  const state = new CatalogSelectionState();
  state.setMinecraftCatalog(normalizeMinecraftCatalog(minecraftResponse));
  state.beginFabricLoad();
  state.setFabricCatalog('26.2', normalizeFabricCatalog(fabricResponse, '26.2'));
  assert.equal(state.fabricLoader, '0.19.3');
  assert.equal(state.ready, true);

  state.changeMinecraft('26.3-snapshot-1');
  assert.equal(state.fabricLoader, '');
  assert.deepEqual(state.fabricVersions, []);
  assert.equal(state.ready, false);
});

test('a stale Fabric response must still match the selected Minecraft version', () => {
  assert.throws(
    () => normalizeFabricCatalog(fabricResponse, '26.1.5'),
    /wrong Minecraft version/,
  );
});

test('empty Fabric catalog leaves Create World blocked', () => {
  const state = new CatalogSelectionState();
  state.setMinecraftCatalog(normalizeMinecraftCatalog(minecraftResponse));
  state.beginFabricLoad();
  state.setFabricCatalog('26.2', [], '');
  assert.equal(state.fabricLoader, '');
  assert.equal(state.ready, false);
});

test('malformed catalog entries are rejected instead of becoming selector values', () => {
  assert.throws(
    () => normalizeMinecraftCatalog({ versions: [{ id: '26.2', type: 'release', supported: true }] }),
    /invalid Minecraft version entry/,
  );
  assert.throws(
    () => normalizeFabricCatalog({ versions: [{ version: '0.19.3', minecraft_version: '26.2' }] }, '26.2'),
    /invalid loader entry/,
  );
});

test('provider failures surface player-facing retry copy', () => {
  assert.equal(
    catalogErrorMessage('mojang', { code: 'provider_unavailable' }),
    'Could not reach Mojang. Retry.',
  );
  assert.equal(
    catalogErrorMessage('fabric', { code: 'malformed_provider_response' }),
    'Fabric Meta returned invalid loader data. Retry.',
  );
});

test('Create World source uses selectors and thin Tauri catalog commands', async () => {
  const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
  const main = await readFile(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
  const controller = await readFile(new URL('../src/catalog-selectors.js', import.meta.url), 'utf8');

  assert.match(html, /<select id="createMinecraft"/);
  assert.match(html, /<select id="createLoader"/);
  assert.doesNotMatch(html, /<input id="createMinecraft"/);
  assert.doesNotMatch(html, /<input id="createLoader"/);
  assert.match(html, /id="createSnapshots"/);
  assert.match(main, /minecraft_versions/);
  assert.match(main, /fabric_loader_versions/);
  assert.match(main, /validate_fabric_selection/);
  assert.match(controller, /Fetching Minecraft versions\.\.\./);
  assert.match(controller, /Could not reach Mojang\. Retry\./);
});
