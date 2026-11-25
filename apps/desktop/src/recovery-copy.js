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
      || !Number.isSafeInteger(result.source_snapshot_number)
      || String(result.source_snapshot_number) !== selection.snapshot
      || !validWorld(result.world_id) || result.world_id === selection.world
      || !validHash(result.snapshot_hash) || result.snapshot_hash === selection.checkpoint
      || typeof result.display_name !== 'string' || !result.display_name.startsWith('Recovered: ')) {
    throw new Error('Recovered world receipt could not be confirmed. Refresh Worlds before retrying.');
  }
  return Object.freeze({ source: selection.world, world: result.world_id, name: result.display_name });
}
