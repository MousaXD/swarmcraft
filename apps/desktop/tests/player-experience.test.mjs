import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { deriveJourneyState } from '../src/player-experience.js';

const srcRoot = new URL('../src/', import.meta.url);
const desktopRoot = new URL('../', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

async function desktopText(name) {
  return readFile(new URL(name, desktopRoot), 'utf8');
}

test('local Play readiness does not imply a verified Minecraft route', () => {
  const localReady = deriveJourneyState({
    playDisabled: false,
    playDetail: 'Ready to launch Minecraft.',
    hostReadiness: 'Keep this PC on. No successor is ready yet.',
    migration: 'Not active',
    connectivity: 'Direct connection',
  });
  assert.equal(localReady.kind, 'neutral');
  assert.equal(localReady.label, 'Local setup available');
  assert.match(localReady.detail, /not proof of a live multiplayer route/i);

  const routed = deriveJourneyState({
    playDisabled: true,
    playDetail: 'Local hosting not eligible',
    migration: 'Not active',
    connectivity: 'Connectivity diagnostics unavailable',
    routeVerified: true,
  });
  assert.equal(routed.kind, 'ready');
  assert.equal(routed.label, 'Minecraft route verified');
  assert.match(routed.detail, /actual game login has not yet been observed/i);
});

test('player journey elevates real play blockers and active host changes', () => {
  const blocked = deriveJourneyState({
    playDisabled: true,
    playDetail: 'Minecraft runtime setup is missing on this device.',
    migration: 'Not active',
    connectivity: 'Direct connection',
  });
  assert.equal(blocked.kind, 'action');
  assert.equal(blocked.label, 'Needs attention');

  const moving = deriveJourneyState({
    playDisabled: true,
    playDetail: 'Waiting for host.',
    migration: 'Transferring authority',
    connectivity: 'Connected through relay',
  });
  assert.equal(moving.kind, 'working');
  assert.equal(moving.label, 'Host change in progress');
});

test('world compatibility and connectivity do not claim hosting or Minecraft readiness', () => {
  const state = deriveJourneyState({
    playDisabled: false,
    playDetail: 'This device is compatible with the world. Local hosting requires opt-in.',
    connectivity: 'Direct connection',
  });
  assert.equal(state.kind, 'neutral');
  assert.equal(state.label, 'Play options');
});

test('player-first layout keeps proven command IDs while moving operator surfaces behind Advanced', async () => {
  const source = await text('player-experience.js');
  for (const id of [
    'playWorld', 'inviteWorld', 'transferHost', 'hostReadinessPanel', 'migrationCard',
    'modsPanel', 'worldNotice', 'advancedWorldTools', 'playerJourneyOverview',
  ]) {
    assert.match(source, new RegExp(id));
  }
  assert.match(source, /Advanced world controls/);
  assert.match(source, /Replication, hosting, mods and diagnostics/);
  assert.match(source, /Keep world available here/);
  assert.match(source, /data-primary-route-state/);
  assert.match(source, /data-primary-route-detail/);
  assert.match(source, /data-reconnect-world disabled/);
  assert.match(source, /attributeFilter: \['hidden', 'disabled', 'class', 'data-route-verified'\]/);
});

test('staged Svelte and shipping HTML use the same delegated proof-bound reconnect action', async () => {
  const candidate = await desktopText('frontend/src/components/WorldsView.svelte');
  const controller = await text('app.js');
  const route = await text('game-route-status.js');
  for (const field of ['data-primary-route-state', 'data-primary-route-detail', 'data-reconnect-world']) {
    assert.match(candidate, new RegExp(field));
    assert.match(route, new RegExp(field));
  }
  assert.match(controller, /updateGameRouteControls\(document, route, selectedWorldId/);
  assert.match(controller, /event\.target\?\.closest\?\.\('\[data-reconnect-world\]'\)/);
});

test('presentation layer does not recreate backend authority, fencing, quorum or runtime actions', async () => {
  const source = await text('player-experience.js');
  assert.doesNotMatch(source, /invoke\(|manual_transfer_step|transferPrepare|transferCommit|transferActivate|save_epoch|fencing_token|next_epoch|has_quorum/i);
  assert.doesNotMatch(source, /backend\./);
});

test('advanced navigation keeps diagnostics available without making them primary navigation', async () => {
  const source = await text('player-experience.js');
  assert.match(source, /advancedNavigation/);
  assert.match(source, /navActivity/);
  assert.match(source, /navDiagnostics/);
  assert.match(source, /advanced-navigation-body/);
});
