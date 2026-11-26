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
import { fabricCatalogChoiceLabel } from '../src/runtime-catalog-policy.js';

const minecraftResponse = {
  provider: 'mojang',
  origin: 'network',
  versions: [
    { id: '26.3-snapshot-1', type: 'snapshot', release_time: '2026-08-20T10:00:00+00:00', supported: true, adapter_supported: false, selection_state: 'unsupported_adapter' },
    { id: '26.1.5', type: 'release', release_time: '2026-06-23T10:00:00+00:00', supported: true, adapter_supported: true, selection_state: 'adapter_compatible_unqualified' },
    { id: '26.1.2', type: 'release', release_time: '2026-01-01T00:00:00+00:00', supported: true, adapter_supported: true, selection_state: 'pinned_baseline' },
  ],
};

const fabricResponse = {
  provider: 'fabric',
  origin: 'network',
  versions: [
    { version: '0.20.0-beta.1', stable: false, minecraft_version: '26.1.2', adapter_supported: false, selection_state: 'unsupported_adapter' },
    { version: '0.20.1', stable: true, minecraft_version: '26.1.2', adapter_supported: true, selection_state: 'adapter_compatible_unqualified' },
    { version: '0.19.3', stable: true, minecraft_version: '26.1.2', adapter_supported: true, selection_state: 'pinned_baseline' },
  ],
};

test('normal Minecraft default remains a stable release when snapshots are present', () => {
  const versions = normalizeMinecraftCatalog(minecraftResponse);
  assert.equal(chooseMinecraftVersion(versions), '26.1.2');
});

test('Fabric default prefers the exact shipped baseline', () => {
  const loaders = normalizeFabricCatalog(fabricResponse, '26.1.2');
  assert.equal(chooseFabricLoader(loaders), '0.19.3');
});

test('a listed pinned Fabric release remains selectable when the provider stable flag changes', () => {
  const response = { ...fabricResponse, versions: fabricResponse.versions.map(loader =>
    loader.version === '0.19.3' ? { ...loader, stable: false } : loader) };
  const loaders = normalizeFabricCatalog(response, '26.1.2');
  const pinned = loaders.find(loader => loader.version === '0.19.3');
  assert.equal(pinned.stable, false, 'do not rewrite provider metadata');
  assert.equal(chooseFabricLoader(loaders, '0.20.1'), '0.19.3');
  assert.equal(fabricCatalogChoiceLabel(pinned.version, pinned.selectionState, pinned.stable),
    '0.19.3 (pinned baseline) — provider stable flag not set');
  const state = new CatalogSelectionState();
  state.setMinecraftCatalog(normalizeMinecraftCatalog(minecraftResponse));
  state.setFabricCatalog('26.1.2', loaders);
  assert.equal(state.ready, true);
  state.setFabricCatalog('26.1.2', loaders.filter(loader => loader !== pinned));
  assert.equal(state.ready, false, 'a newer provider-stable loader cannot replace a missing exact baseline');
  assert.equal(state.fabricLoader, '');
  assert.match(fabricCatalogChoiceLabel('0.20.0-beta.1', 'unsupported_adapter', false), /incompatible adapter/);
  assert.throws(() => normalizeFabricCatalog({ versions: [{ ...response.versions[2], stable: null }] }, '26.1.2'), /invalid loader entry/);
});

test('changing Minecraft invalidates the prior Fabric selection before revalidation', () => {
  const state = new CatalogSelectionState();
  state.setMinecraftCatalog(normalizeMinecraftCatalog(minecraftResponse));
  state.beginFabricLoad();
  state.setFabricCatalog('26.1.2', normalizeFabricCatalog(fabricResponse, '26.1.2'));
  assert.equal(state.fabricLoader, '0.19.3');
  assert.equal(state.ready, true);

  state.changeMinecraft('26.3-snapshot-1');
  assert.equal(state.fabricLoader, '');
  assert.deepEqual(state.fabricVersions, []);
  assert.equal(state.ready, false);
});

test('a Fabric response must match the selected Minecraft version', () => {
  assert.throws(
    () => normalizeFabricCatalog(fabricResponse, '26.1.5'),
    /wrong Minecraft version/,
  );
});

test('empty Fabric catalog leaves Create World blocked', () => {
  const state = new CatalogSelectionState();
  state.setMinecraftCatalog(normalizeMinecraftCatalog(minecraftResponse));
  state.beginFabricLoad();
  state.setFabricCatalog('26.1.2', [], '');
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

test('Create World bootstraps real selectors and thin Tauri catalog commands', async () => {
  const main = await readFile(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
  const controller = await readFile(new URL('../src/catalog-selectors.js', import.meta.url), 'utf8');
  const importFlow = await readFile(new URL('../src/import-flow.js', import.meta.url), 'utf8');

  assert.match(importFlow, /import '\.\/catalog-selectors\.js';/);
  assert.match(controller, /function upgradeInputToSelect/);
  assert.match(controller, /select\.disabled = true/);
  assert.match(controller, /Show Minecraft snapshots in the version selector/);
  assert.match(controller, /Fetching Minecraft versions\.\.\./);
  assert.match(controller, /Could not reach Mojang\. Retry\./);
  assert.match(main, /minecraft_versions/);
  assert.match(main, /fabric_loader_versions/);
  assert.match(main, /validate_fabric_selection/);
});

test('newer official releases remain visible but cannot become default for an older shipped Fabric adapter', () => {
  const versions = normalizeMinecraftCatalog({
    versions: [
      { id: '26.3', type: 'release', release_time: '2026-10-01T00:00:00Z', supported: true, adapter_supported: false, selection_state: 'unsupported_adapter' },
      { id: '26.1.2', type: 'release', release_time: '2026-01-01T00:00:00Z', supported: true, adapter_supported: true, selection_state: 'pinned_baseline' },
    ],
  });
  assert.equal(versions.length, 2, 'official version data must remain visible');
  assert.equal(versions[0].adapterSupported, false, 'newer version is not compatible with this build');
  assert.equal(chooseMinecraftVersion(versions), '26.1.2');
  assert.equal(chooseMinecraftVersion(versions, '26.3'), '26.1.2', 'incompatible preference must never override adapter');
  const state = new CatalogSelectionState();
  state.setMinecraftCatalog(versions);
  state.setFabricCatalog('26.1.2', normalizeFabricCatalog({
    versions: [{ version: '0.19.3', stable: true, minecraft_version: '26.1.2', adapter_supported: true, selection_state: 'pinned_baseline' }],
  }, '26.1.2'));
  assert.equal(state.ready, true);
  state.changeMinecraft('26.3');
  state.setFabricCatalog('26.3', normalizeFabricCatalog({
    versions: [{ version: '0.20.1', stable: true, minecraft_version: '26.3', adapter_supported: false, selection_state: 'unsupported_adapter' }],
  }, '26.3'));
  assert.equal(state.ready, false, 'incompatible release and loader cannot re-enable Create');
});

test('Fabric Loader defaults honor the installed adapter minimum rather than blindly using stable', () => {
  const versions = normalizeFabricCatalog({
    versions: [
      { version: '0.19.2', stable: true, minecraft_version: '26.1.2', adapter_supported: false, selection_state: 'unsupported_adapter' },
      { version: '0.20.1', stable: true, minecraft_version: '26.1.2', adapter_supported: true, selection_state: 'adapter_compatible_unqualified' },
      { version: '0.19.3', stable: true, minecraft_version: '26.1.2', adapter_supported: true, selection_state: 'pinned_baseline' },
    ],
  }, '26.1.2');
  assert.equal(versions.length, 3);
  assert.equal(chooseFabricLoader(versions, '0.19.2'), '0.19.3');
  assert.equal(chooseFabricLoader(versions, '0.20.1'), '0.19.3');
  assert.equal(chooseFabricLoader(versions), '0.19.3');
  assert.throws(
    () => normalizeMinecraftCatalog({ versions: [
      { id: '26.1.2', type: 'release', release_time: '2026-01-01Z', supported: true, adapter_supported: 'yes' },
    ] }),
    /compatibility flag/,
  );
});

test('missing or contradictory qualification metadata fails closed, and an unqualified range is never selectable', () => {
  const entry = { id: '26.1.5', type: 'release', release_time: '2026-01-01Z', supported: true, adapter_supported: true };
  assert.throws(() => normalizeMinecraftCatalog({ versions: [entry] }), /qualification state/);
  assert.throws(() => normalizeMinecraftCatalog({ versions: [{ ...entry, selection_state: 'unsupported_adapter' }] }), /qualification state/);
  const candidates = normalizeMinecraftCatalog({ versions: [{ ...entry, selection_state: 'adapter_compatible_unqualified' }] });
  assert.equal(chooseMinecraftVersion(candidates, '26.1.5'), '');
  const loaders = normalizeFabricCatalog({ versions: [{ version: '0.20.1', stable: true, minecraft_version: '26.1.2', adapter_supported: true, selection_state: 'adapter_compatible_unqualified' }] }, '26.1.2');
  assert.equal(chooseFabricLoader(loaders), '');
});
