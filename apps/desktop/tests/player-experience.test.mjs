import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { deriveJourneyState } from '../src/player-experience.js';

const srcRoot = new URL('../src/', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

test('player journey treats Play readiness separately from shutdown redundancy', () => {
  const ready = deriveJourneyState({
    playDisabled: false,
    playDetail: 'Ready to launch Minecraft.',
    hostReadiness: 'Keep this PC on. No successor is ready yet.',
    migration: 'Not active',
    connectivity: 'Direct connection',
  });
  assert.equal(ready.kind, 'ready');
  assert.equal(ready.label, 'Ready to play');
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
