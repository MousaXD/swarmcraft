const unavailable = (feature) => {
  const error = new Error(`${feature} is not available in this build.`);
  error.code = 'SWARMCRAFT_CAPABILITY_UNAVAILABLE';
  return error;
};

export const MIGRATION_PHASES = Object.freeze([
  'preparing-successor',
  'saving-world',
  'transferring-authority',
  'restoring-world',
  'starting-minecraft',
  'waiting-for-host',
  'ready',
  'failed',
]);

const MIGRATION_LABELS = Object.freeze({
  'preparing-successor': 'Preparing successor',
  'saving-world': 'Saving world',
  'transferring-authority': 'Transferring authority',
  'restoring-world': 'Restoring world',
  'starting-minecraft': 'Starting Minecraft',
  'waiting-for-host': 'Waiting for host',
  ready: 'Ready',
  failed: 'Migration failed',
});

const CONNECTIVITY_LABELS = Object.freeze({
  direct: 'Direct',
  relay: 'Relay',
  connecting: 'Connecting',
  offline: 'Offline',
  limited: 'Limited connectivity',
  action: 'Action required',
  unknown: 'Not reported',
});

function slug(value) {
  return String(value || '')
    .trim()
    .toLowerCase()
    .replace(/[_\s]+/g, '-')
    .replace(/[^a-z0-9-]/g, '');
}

export function normalizeMigrationState(raw) {
  if (!raw) {
    return {
      available: false,
      phase: null,
      label: 'Not active',
      detail: 'No host migration is currently reported.',
      progress: 0,
      failed: false,
    };
  }

  const source = typeof raw === 'string' ? { phase: raw } : raw;
  const phase = slug(source.phase || source.state || source.status);
  const knownPhase = MIGRATION_PHASES.includes(phase) ? phase : null;
  const index = knownPhase ? MIGRATION_PHASES.indexOf(knownPhase) : -1;
  const progress = knownPhase === 'failed'
    ? Math.max(0, Math.min(100, Number(source.progress || 0)))
    : knownPhase === 'ready'
      ? 100
      : index >= 0
        ? Math.round((index / (MIGRATION_PHASES.length - 2)) * 100)
        : 0;

  return {
    available: Boolean(knownPhase),
    phase: knownPhase,
    label: knownPhase ? MIGRATION_LABELS[knownPhase] : 'Migration state unavailable',
    detail: String(source.detail || source.message || '').trim(),
    progress,
    failed: knownPhase === 'failed',
  };
}

export function connectivityFromStatus(status = {}) {
  const raw = [
    status.Connectivity,
    status['Connectivity state'],
    status.Connection,
    status['Network path'],
    status.Reachability,
  ].find((value) => String(value || '').trim());

  if (!raw) {
    return {
      kind: 'unknown',
      label: CONNECTIVITY_LABELS.unknown,
      detail: 'The current backend does not expose structured connectivity for this world.',
      raw: '',
    };
  }

  const value = String(raw).toLowerCase();
  let kind = 'unknown';
  if (value.includes('direct')) kind = 'direct';
  else if (value.includes('relay')) kind = 'relay';
  else if (value.includes('offline') || value.includes('disconnected')) kind = 'offline';
  else if (value.includes('action') || value.includes('blocked')) kind = 'action';
  else if (value.includes('limited') || value.includes('degraded')) kind = 'limited';
  else if (value.includes('connect')) kind = 'connecting';
  else if (value.includes('nat')) kind = 'action';

  const detailByKind = {
    direct: 'Peers can reach this world over a direct path.',
    relay: 'Traffic is using a relay path. Gameplay may still work, but the path is less direct.',
    connecting: 'SwarmCraft is still establishing a usable path.',
    offline: 'No usable network path is currently reported.',
    limited: 'Connectivity is available with restrictions or reduced reachability.',
    action: 'Networking needs attention before a reliable path is available.',
    unknown: `Backend reported: ${String(raw).trim()}`,
  };

  return { kind, label: CONNECTIVITY_LABELS[kind], detail: detailByKind[kind], raw: String(raw).trim() };
}

export function createBackendAdapter(invoke) {
  const call = (command, payload = {}) => {
    if (typeof invoke !== 'function') {
      throw new Error('Tauri runtime is unavailable. Open this interface from the SwarmCraft desktop application.');
    }
    return invoke(command, payload);
  };

  return Object.freeze({
    initializeNode: () => call('initialize_node'),
    nodeIdentity: () => call('node_identity'),
    listWorlds: () => call('list_worlds'),
    createWorld: (payload) => call('create_world', payload),
    joinWorld: (invite) => call('join_world', { invite }),
    leaveWorld: (world) => call('leave_world', { world }),
    createInvite: (payload) => call('create_invite', payload),
    worldStatus: (world) => call('world_status', { world }),
    worldCompatibility: (world) => call('world_compatibility', { world }),
    worldConflicts: (world) => call('world_conflicts', { world }),
    setBackgroundSeeding: (world, enabled) => call('set_background_seeding', { world, enabled }),
    worldPeers: (world) => call('world_peers', { world }),
    verifyWorld: (world) => call('verify_world', { world }),
    exportWorld: (world, destination) => call('export_world', { world, destination }),
    recoverWorld: (world, snapshot, destination) => call('recover_world', { world, snapshot, destination }),
    startDaemon: (listen) => call('start_daemon', { listen }),
    stopDaemon: () => call('stop_daemon'),
    hostWorld: (payload) => call('host_world', payload),
    stopHost: () => call('stop_host'),

    // migration-core owns the authoritative implementation. Keeping these entry points
    // capability-gated prevents the desktop from inventing consensus or recovery behavior.
    migration: Object.freeze({
      capabilities: Object.freeze({ status: false, transfer: false, wake: false }),
      readState: async () => normalizeMigrationState(null),
      transferAuthority: async () => { throw unavailable('Transfer host'); },
      wakeWorld: async () => { throw unavailable('Wake world'); },
    }),
  });
}
