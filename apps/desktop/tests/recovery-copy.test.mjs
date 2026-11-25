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

test('recovery review pins historical choices and never treats branch evidence as available files', async () => {
  const { normalizeRecoveryReview, recoveryExportReceipt } = await import('../src/recovery-copy.js');
  const digest = '9'.repeat(64);
  const branchHash = 'f'.repeat(64);
  const raw = {
    world_id: world, evidence: 'local_signed_history_not_live_authority',
    checkpoint_history: 'verified_history_files_checked_on_action', checkpoint_error: null,
    conflict_digest: digest,
    checkpoints: [
      { checkpoint_hash: checkpoint, snapshot_number: '4', epoch: '1', sequence: '4',
        state_root: 'a'.repeat(64), signer: 'scpeer:' + 'd'.repeat(64), files: 2, latest: true },
      { checkpoint_hash: 'e'.repeat(64), snapshot_number: '3', epoch: '1', sequence: '3',
        state_root: 'b'.repeat(64), signer: 'scpeer:' + 'd'.repeat(64), files: 1, latest: false },
    ],
    conflicts: [{ branch_hash: branchHash, base_checkpoint: 'e'.repeat(64), head_checkpoint: '7'.repeat(64),
      head_epoch: '1', head_sequence: '4', state_root: 'a'.repeat(64), signer: 'scpeer:' + 'd'.repeat(64),
      local_history: 'evidence_only' }],
  };
  const review = normalizeRecoveryReview(world, JSON.stringify(raw));
  assert.equal(review.choices[1].snapshot, '3');
  assert.equal(review.choices[1].conflictDigest, digest);
  const branch = review.choices[2];
  assert.equal(branch.localHistory, 'evidence_only');
  const result = { world_id: world, branch_hash: branchHash, checkpoint_hash: branch.checkpoint,
    evidence: 'signed_branch_evidence_only', files_exported: false, manifest_count: 0, destination: '/tmp/evidence' };
  assert.deepEqual(recoveryExportReceipt(branch, result, true), { destination: '/tmp/evidence', files: false });
  for (const change of [{ world_id: copy }, { branch_hash: '1'.repeat(64) }, { files_exported: true },
    { manifest_count: 1 }, { evidence: 'verified_local_checkpoint_not_authority' }]) {
    assert.throws(() => recoveryExportReceipt(branch, { ...result, ...change }, true), /could not be confirmed/);
  }
  for (const change of [{ world_id: copy }, { conflict_digest: null },
    { checkpoints: [raw.checkpoints[0], raw.checkpoints[0]] },
    { checkpoint_history: 'unavailable' }]) {
    assert.throws(() => normalizeRecoveryReview(world, { ...raw, ...change }), /recovery|checkpoint/i);
  }
});

test('explicit conflict acknowledgement must match recovered-copy receipt', () => {
  const selection = { ...recoveryCopySelection(world, status), conflictDigest: '9'.repeat(64) };
  const result = { source_world_id: world, source_snapshot_hash: checkpoint, source_snapshot_number: 4,
    world_id: copy, snapshot_hash: 'd'.repeat(64), display_name: 'Recovered: Example' };
  assert.throws(() => recoveryCopyReceipt(selection, result), /Refresh Worlds/);
  assert.equal(recoveryCopyReceipt(selection, { ...result, source_conflict_digest: selection.conflictDigest }).world, copy);
});
