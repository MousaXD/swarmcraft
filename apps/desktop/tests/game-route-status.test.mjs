import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createBackendAdapter } from '../src/backend-adapter.js';
import { clientContinuityFromMigration } from '../src/client-continuity.js';
import { ensureGameReconnectActions, gameReadinessCheckpoints, isUsableGameRoute, normalizeGameRouteStatus, UNVERIFIED_GAME_ROUTE, updateGameReadinessChecklist } from '../src/game-route-status.js';

const WORLD = 'scworld:1111111111111111111111111111111111111111111111111111111111111111';
const NOW = Date.now();
const live = () => ({
  world_id: WORLD, state: 'verified', quorum_verified: true,
  hosting_consent_verified: true, server_ready: true, route_verified: true,
  local_gateway: '127.0.0.1:25570', authority_peer_id: `scpeer:${'c'.repeat(64)}`,
  epoch: 10, fencing_token: 23, snapshot_hash: 'a'.repeat(64),
  verified_at_unix_ms: NOW - 100, valid_until_unix_ms: NOW + 10_000,
});

test('verified route is strictly live, localhost-only and world-bound', () => {
  const result = normalizeGameRouteStatus(live(), WORLD, NOW);
  assert.equal(result.verified, true);
  assert.equal(result.connectionTarget, '127.0.0.1:25570');
  assert.equal(result.automaticReconnect, false);
  for (const [name, value] of [
    ['world_id', 'scworld:wrong'],
    ['local_gateway', '192.168.1.2:25570'],
    ['local_gateway', 'localhost:25570'],
    ['local_gateway', '127.0.0.1:0'],
    ['local_gateway', '127.0.0.1:65536'],
    ['route_verified', false],
    ['quorum_verified', false],
    ['authority_peer_id', 'not-a-canonical-peer'],
    ['hosting_consent_verified', false],
    ['server_ready', false],
    ['snapshot_hash', 'not-verified'],
    ['epoch', -1],
    ['verified_at_unix_ms', NOW - 10_000],
    ['valid_until_unix_ms', NOW - 1],
  ]) {
    const altered = { ...live(), [name]: value };
    assert.throws(() => normalizeGameRouteStatus(altered, WORLD, NOW), undefined, name);
  }
});

test('no runtime or persisted migration status may substitute for an authenticated route', () => {
  assert.equal(normalizeGameRouteStatus(null, WORLD).connectionTarget, null);
  const blocked = normalizeGameRouteStatus({ ...live(), state: 'waiting_for_quorum', route_verified: false }, WORLD, NOW);
  assert.equal(blocked.verified, false);
  assert.equal(blocked.connectionTarget, null);
  assert.match(blocked.label, /quorum/i);
  assert.throws(() => normalizeGameRouteStatus({ ...live(), state: 'blocked' }, WORLD, NOW));
  const migration = { world_id: WORLD, phase: 'ready', runtime_ready: true, game_endpoint: 'evil.example:25565',
    authority_peer_id: 'old-host', epoch: 1, fencing_token: 1, snapshot_hash: 'old-hash' };
  assert.equal(clientContinuityFromMigration(migration, WORLD, 'Canonical').connectionTarget, null);
  assert.equal(clientContinuityFromMigration(migration, WORLD, 'Canonical', UNVERIFIED_GAME_ROUTE).connectionTarget, null);
});

test('verified route enables only a new manual session, never automatic reconnect', () => {
  const route = normalizeGameRouteStatus(live(), WORLD, NOW);
  const continuity = clientContinuityFromMigration(null, WORLD, 'Canonical', route);
  assert.equal(continuity.state, 'route_verified');
  assert.equal(continuity.connectionTarget, route.connectionTarget);
  assert.equal(continuity.automaticReconnect, false);
  assert.equal(clientContinuityFromMigration(null, WORLD, 'Conflict', route).connectionTarget, null);
  assert.equal(clientContinuityFromMigration(null, WORLD, 'Degraded', route).connectionTarget, null);
});

test('proof age caps a longer route ticket and binds it to the world being displayed', async () => {
  const route = normalizeGameRouteStatus(live(), WORLD, NOW);
  assert.equal(route.worldId, WORLD);
  assert.equal(route.validUntil, NOW + 4_900);
  assert.equal(isUsableGameRoute(route, WORLD, 'Canonical', NOW + 4_899), true);
  assert.equal(isUsableGameRoute(route, WORLD, 'Canonical', NOW + 4_900), false);
  assert.equal(isUsableGameRoute(route, 'scworld:other', 'Canonical', NOW), false);
  assert.equal(isUsableGameRoute(route, WORLD, 'SoloUnreplicated', NOW), false);
  assert.equal(isUsableGameRoute(route, WORLD, 'Conflict', NOW), false);
  const wrongWorld = { id: 'scworld:other', snapshots: 1, status: { Safety: 'Canonical' } };
  assert.match(gameReadinessCheckpoints({ world: wrongWorld, route }, NOW)[5].value, /not verified/i);
  const source = await readFile(new URL('../src/app.js', import.meta.url), 'utf8');
  assert.match(source, /isUsableGameRoute\(route, selectedWorldId/);
  assert.match(source, /result\.validUntil - Date\.now\(\)/);
  assert.match(source, /generation === gameRouteRequestGeneration && gameRouteState === result/);
});

test('checkpoint, quorum, consent, runtime and route are distinct', () => {
  const world = { id: WORLD, snapshots: 2, status: { Safety: 'Canonical', 'Latest snapshot': 'head-3' } };
  const withoutProof = gameReadinessCheckpoints({ world });
  assert.deepEqual(withoutProof.map((item) => item.label), [
    'Saved copy', 'Local integrity', 'Canonical quorum', 'Hosting consent', 'Live Fabric server', 'Minecraft route',
  ]);
  assert.match(withoutProof[2].value, /unverified/i);
  assert.match(withoutProof[5].value, /not verified/i);
  const withProof = gameReadinessCheckpoints({ world, route: normalizeGameRouteStatus(live(), WORLD, NOW),
    localVerification: { worldId: WORLD, checkpoint: 'head-3' } });
  assert.match(withProof[1].value, /head not independently bound/i);
  assert.match(withProof[5].value, /127\.0\.0\.1:25570/);
  assert.match(gameReadinessCheckpoints({ world, route: normalizeGameRouteStatus(live(), WORLD, NOW) }, NOW + 11_000)[5].value, /not verified/i);
  assert.match(gameReadinessCheckpoints({ world: { ...world, status: { Safety: 'Conflict' } },
    route: normalizeGameRouteStatus(live(), WORLD, NOW) })[5].value, /not verified/i);
});

test('native route bridge uses only a verified command and never game_endpoint', async () => {
  const called = [];
  const backend = createBackendAdapter(async (command, payload) => {
    called.push([command, payload]);
    if (command === 'game_route_status') return live();
    throw new Error('unexpected command');
  });
  assert.equal((await backend.gameRouteStatus(WORLD)).connectionTarget, '127.0.0.1:25570');
  assert.deepEqual(called, [['game_route_status', { world: WORLD }]]);
  const app = await readFile(new URL('../src/app.js', import.meta.url), 'utf8');
  const native = await readFile(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
  assert.match(app, /gameRouteRequestGeneration/);
  assert.match(app, /gameRouteState = UNVERIFIED_GAME_ROUTE/);
  assert.match(app, /refreshGameRoute\(world\)/);
  assert.match(native, /game_route_status,/);
  assert.match(native, /decode_current_game_route_status/);
});

function fakeSvelteCard() {
  const node = (tagName) => ({
    tagName, className: '', dataset: {}, children: [], parentNode: null,
    attributes: {}, textContent: '', disabled: false,
    append(...children) {
      for (const child of children) { child.parentNode = this; this.children.push(child); }
    },
    insertBefore(child, next) {
      const index = next ? this.children.indexOf(next) : -1;
      child.parentNode = this;
      this.children.splice(index >= 0 ? index : this.children.length, 0, child);
    },
    replaceChildren(...children) {
      this.children = [];
      this.append(...children);
    },
    setAttribute(name, value) { this.attributes[name] = value; },
    get nextSibling() {
      if (!this.parentNode) return null;
      return this.parentNode.children[this.parentNode.children.indexOf(this) + 1] || null;
    },
    querySelector(selector) {
      const candidates = [...this.children];
      while (candidates.length) {
        const candidate = candidates.shift();
        if (selector === '.safety-panel' && candidate.className.split(' ').includes('safety-panel')) return candidate;
        if (selector.startsWith('[data-')) {
          const attribute = selector.slice(6, -1).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
          if (Object.hasOwn(candidate.dataset, attribute)) return candidate;
        }
        candidates.push(...candidate.children);
      }
      return null;
    },
  });
  const card = node('section');
  const panel = node('div');
  panel.className = 'safety-panel neutral';
  card.append(panel);
  return { card, panel, doc: { getElementById: id => id === 'migrationCard' ? card : null, createElement: node } };
}

test('the staged Svelte DOM gets one reusable fail-closed reconnect UI without duplicate IDs', () => {
  const fixture = fakeSvelteCard();
  const checkpoints = gameReadinessCheckpoints();
  assert.equal(updateGameReadinessChecklist(fixture.doc, checkpoints), true);
  assert.equal(ensureGameReconnectActions(fixture.doc), true);
  const button = fixture.card.querySelector('[data-reconnect-world]');
  const address = fixture.card.querySelector('[data-verified-gateway-address]');
  assert.equal(button.disabled, true);
  assert.equal(address.textContent, 'No verified address');
  assert.equal(fixture.card.querySelector('[data-game-readiness-checkpoints]').children.length, 6);
  assert.equal(ensureGameReconnectActions(fixture.doc), true);
  assert.equal(fixture.card.children.length, 3);
  updateGameReadinessChecklist(fixture.doc, checkpoints);
  assert.equal(fixture.card.children.length, 3);
});
