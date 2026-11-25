import assert from 'node:assert/strict';
import test from 'node:test';
import { recoveryCopySelection, recoveryCopyReceipt } from '../src/recovery-copy.js';
import { createBackendAdapter } from '../src/backend-adapter.js';

const world = 'scworld:' + 'a'.repeat(64);
const copy = 'scworld:' + 'b'.repeat(64);
const checkpoint = 'c'.repeat(64);
const status = [
  'World: Example',
  'World ID: ' + world,
  'Safety: Canonical',
  'Latest snapshot: 4',
  'Checkpoint manifest: ' + checkpoint,
].join('\n');

test('recovery selection pins the exact local world and checkpoint', () => {
  assert.deepEqual(recoveryCopySelection(world, status), { world, checkpoint, snapshot: '4' });
  for (const invalid of [
    status.replace(world, copy),
    status.replace('Canonical', 'Conflict'),
    status.replace('Canonical', 'SoloUnreplicated'),
    status.replace('Latest snapshot: 4', 'Latest snapshot: none'),
    status.replace(checkpoint, 'invalid'),
    status + '\nCheckpoint manifest: ' + 'd'.repeat(64),
  ]) assert.throws(() => recoveryCopySelection(world, invalid), /canonical local checkpoint/);
});

test('receipt requires a new identity and the exact selected source', () => {
  const selection = recoveryCopySelection(world, status);
  const result = {
    source_world_id: world, source_snapshot_hash: checkpoint, source_snapshot_number: 4,
    world_id: copy, snapshot_hash: 'd'.repeat(64), display_name: 'Recovered: Example',
  };
  assert.deepEqual(recoveryCopyReceipt(selection, JSON.stringify(result)), {
    source: world, world: copy, name: 'Recovered: Example',
  });
  for (const change of [
    { source_world_id: copy }, { world_id: world }, { source_snapshot_hash: 'd'.repeat(64) },
    { source_snapshot_number: 5 }, { snapshot_hash: checkpoint }, { display_name: 'Example' },
  ]) assert.throws(() => recoveryCopyReceipt(selection, { ...result, ...change }), /Refresh Worlds/);
});

test('native recovery dispatch requires explicit new-identity confirmation', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, args) => { calls.push({ command, args }); return 'receipt'; });
  await assert.rejects(backend.recoverCopy(world, checkpoint, false), /separate world identity/);
  assert.equal(calls.length, 0);
  assert.equal(await backend.recoverCopy(world, checkpoint, true), 'receipt');
  assert.deepEqual(calls, [{ command: 'recover_copy', args: { world, checkpoint, confirmNewIdentity: true } }]);
});
