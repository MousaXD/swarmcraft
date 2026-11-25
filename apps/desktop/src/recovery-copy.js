const validWorld = (value) => typeof value === 'string' && /^scworld:[a-f0-9]{64}$/.test(value);
const validHash = (value) => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);

// The status text is only a selection, not proof of valid history. The
// native command independently verifies the checkpoint and signed chain.
export function recoveryCopySelection(world, status) {
  const field = (label) => {
    const prefix = label + ': ';
    const matches = String(status ?? '').split(/\r?\n/).filter(line => line.startsWith(prefix));
    return matches.length === 1 ? matches[0].slice(prefix.length).trim() : null;
  };
  const checkpoint = field('Checkpoint manifest');
  const snapshot = field('Latest snapshot');
  if (!validWorld(world) || field('World ID') !== world || !validHash(checkpoint)
      || !snapshot || !/^[1-9]\d*$/.test(snapshot) || field('Safety') !== 'Canonical') {
    throw new Error('No unambiguous canonical local checkpoint is available. Verify the world or export an available snapshot instead.');
  }
  return Object.freeze({ world, checkpoint, snapshot });
}

export function recoveryCopyReceipt(selection, raw) {
  const result = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (!result || result.source_world_id !== selection.world
      || result.source_snapshot_hash !== selection.checkpoint
      || (result.source_conflict_digest ?? null) !== (selection.conflictDigest ?? null)
      || !Number.isSafeInteger(result.source_snapshot_number)
      || String(result.source_snapshot_number) !== selection.snapshot
      || !validWorld(result.world_id) || result.world_id === selection.world
      || !validHash(result.snapshot_hash) || result.snapshot_hash === selection.checkpoint
      || typeof result.display_name !== 'string' || !result.display_name.startsWith('Recovered: ')) {
    throw new Error('Recovered world receipt could not be confirmed. Refresh Worlds before retrying.');
  }
  return Object.freeze({ source: selection.world, world: result.world_id, name: result.display_name });
}

const validNumber = value => typeof value === 'string' && /^[1-9]\d*$/.test(value) && value.length <= 20;
const validPeer = value => typeof value === 'string' && /^scpeer:[a-f0-9]{64}$/.test(value);

// A review is local evidence, never a route or a hosting grant. Reject
// malformed/mismatched native payloads before displaying actionable choices.
export function normalizeRecoveryReview(world, raw) {
  const review = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (!validWorld(world) || !review || review.world_id !== world
      || review.evidence !== 'local_signed_history_not_live_authority'
      || !['verified_history_files_checked_on_action', 'unavailable'].includes(review.checkpoint_history)
      || !Array.isArray(review.checkpoints) || review.checkpoints.length > 4096
      || !Array.isArray(review.conflicts) || review.conflicts.length > 256
      || (review.conflicts.length > 0 ? !validHash(review.conflict_digest) : review.conflict_digest !== null)
      || (review.checkpoint_history === 'unavailable' && review.checkpoints.length !== 0)) {
    throw new Error('Recovery review is unavailable or does not match the selected world.');
  }
  const seen = new Set();
  const choices = [];
  for (const item of review.checkpoints) {
    if (!item || !validHash(item.checkpoint_hash) || !validNumber(item.snapshot_number)
        || !validNumber(item.epoch) || !validNumber(item.sequence) || !validHash(item.state_root)
        || !validPeer(item.signer) || !Number.isSafeInteger(item.files) || item.files < 0
        || typeof item.latest !== 'boolean' || seen.has(item.checkpoint_hash)) {
      throw new Error('Invalid retained checkpoint in recovery review.');
    }
    seen.add(item.checkpoint_hash);
    choices.push(Object.freeze({
      kind: 'checkpoint', value: 'checkpoint:' + item.checkpoint_hash,
      world, checkpoint: item.checkpoint_hash, snapshot: item.snapshot_number,
      epoch: item.epoch, sequence: item.sequence, stateRoot: item.state_root, signer: item.signer,
      files: item.files, latest: item.latest, conflictDigest: review.conflict_digest,
    }));
  }
  const branchHashes = new Set();
  for (const item of review.conflicts) {
    if (!item || !validHash(item.branch_hash) || branchHashes.has(item.branch_hash)
        || !validHash(item.base_checkpoint) || !validHash(item.head_checkpoint)
        || !validNumber(item.head_epoch) || !validNumber(item.head_sequence)
        || !validHash(item.state_root) || !validPeer(item.signer)
        || !['matching_signed_manifests', 'evidence_only', 'unverified_local_history'].includes(item.local_history)) {
      throw new Error('Invalid signed branch in recovery review.');
    }
    branchHashes.add(item.branch_hash);
    choices.push(Object.freeze({
      kind: 'branch', value: 'branch:' + item.branch_hash,
      world, branch: item.branch_hash, checkpoint: item.head_checkpoint, base: item.base_checkpoint,
      epoch: item.head_epoch, sequence: item.head_sequence, stateRoot: item.state_root, signer: item.signer,
      localHistory: item.local_history, conflictDigest: review.conflict_digest,
    }));
  }
  if (review.checkpoints.length && review.checkpoints.filter(item => item.latest).length !== 1) {
    throw new Error('Recovery history does not identify one exact local head.');
  }
  return Object.freeze({
    world, choices: Object.freeze(choices), conflictDigest: review.conflict_digest,
    checkpointError: typeof review.checkpoint_error === 'string' ? review.checkpoint_error : null,
  });
}

export function recoveryExportReceipt(selection, raw, evidenceOnly) {
  const receipt = typeof raw === 'string' ? JSON.parse(raw) : raw;
  const branch = selection.kind === 'branch' ? selection.branch : null;
  const expectedEvidence = branch
    ? (evidenceOnly ? 'signed_branch_evidence_only' : 'signed_branch_head_files_not_canonical_authority')
    : 'verified_local_checkpoint_not_authority';
  if (!receipt || receipt.world_id !== selection.world || receipt.checkpoint_hash !== selection.checkpoint
      || receipt.branch_hash !== branch || receipt.evidence !== expectedEvidence
      || receipt.files_exported !== !evidenceOnly
      || typeof receipt.destination !== 'string' || !receipt.destination.trim()
      || !Number.isSafeInteger(receipt.manifest_count)
      || (evidenceOnly ? receipt.manifest_count !== 0 : receipt.manifest_count < 1)) {
    throw new Error('Export receipt could not be confirmed. Inspect the destination before retrying.');
  }
  return Object.freeze({ destination: receipt.destination, files: receipt.files_exported });
}
