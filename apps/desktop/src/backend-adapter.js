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

const BACKEND_MIGRATION_PHASE_ALIASES = Object.freeze({
  'waiting-for-authority': 'preparing-successor',
  'waiting-for-quorum': 'preparing-successor',
  'selecting-snapshot': 'preparing-successor',
  'preparing-runtime': 'preparing-successor',
  'restoring-world': 'restoring-world',
  'launching-runtime': 'starting-minecraft',
  'verifying-fabric': 'waiting-for-host',
  checkpointing: 'saving-world',
  'awaiting-transfer-acceptance': 'transferring-authority',
  ready: 'ready',
  blocked: 'failed',
  failed: 'failed',
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
      runtimeReady: false,
      gameEndpoint: null,
    };
  }

  const source = typeof raw === 'string' ? { phase: raw } : raw;
  const backendPhase = slug(source.phase || source.state || source.status);
  const runtimeReady = Boolean(source.runtime_ready ?? source.runtimeReady);
  const gameEndpoint = source.game_endpoint ?? source.gameEndpoint ?? null;
  const failureReason = String(source.failure_reason ?? source.failureReason ?? '').trim();

  if (backendPhase === 'dormant' || backendPhase === 'sleeping' || backendPhase === 'superseded') {
    const labels = { dormant: 'Not active', sleeping: 'Sleeping', superseded: 'Host changed' };
    return {
      available: false,
      phase: null,
      label: labels[backendPhase],
      detail: failureReason || (backendPhase === 'sleeping'
        ? 'The world is durably sleeping and can be woken only through the backend safety checks.'
        : backendPhase === 'superseded'
          ? 'This device is no longer the accepted runtime authority.'
          : 'No host migration is active.'),
      progress: 0,
      failed: false,
      runtimeReady,
      gameEndpoint,
    };
  }

  const phase = BACKEND_MIGRATION_PHASE_ALIASES[backendPhase] || backendPhase;
  const knownPhase = MIGRATION_PHASES.includes(phase) ? phase : null;
  const index = knownPhase ? MIGRATION_PHASES.indexOf(knownPhase) : -1;
  const progress = knownPhase === 'failed'
    ? Math.max(0, Math.min(100, Number(source.progress || 0)))
    : knownPhase === 'ready'
      ? 100
      : index >= 0
        ? Math.round((index / (MIGRATION_PHASES.length - 2)) * 100)
        : 0;
  const blocked = backendPhase === 'blocked';
  const detail = String(
    source.detail
      || source.message
      || failureReason
      || (runtimeReady && gameEndpoint ? `Minecraft is ready at ${gameEndpoint}.` : ''),
  ).trim();

  return {
    available: Boolean(knownPhase),
    phase: knownPhase,
    label: blocked ? 'Action required' : knownPhase ? MIGRATION_LABELS[knownPhase] : 'Migration state unavailable',
    detail,
    progress,
    failed: knownPhase === 'failed',
    runtimeReady,
    gameEndpoint,
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

  const migrationCapabilities = { status: false, transfer: false, wake: false };
  let migrationCapabilityProbe = null;
  const ensureMigrationCapabilities = () => {
    if (!migrationCapabilityProbe) {
      migrationCapabilityProbe = Promise.resolve()
        .then(() => call('migration_capabilities'))
        .then((raw) => {
          const supported = new Set(String(raw || '').split(',').map(slug).filter(Boolean));
          migrationCapabilities.status = supported.has('status');
          migrationCapabilities.wake = supported.has('wake');
          // Manual transfer is deliberately still disabled. migration-core exposes a
          // signed multi-stage exchange that must not be collapsed into a fake one-click action.
          migrationCapabilities.transfer = false;
          return migrationCapabilities;
        })
        .catch(() => migrationCapabilities);
    }
    return migrationCapabilityProbe;
  };

  return Object.freeze({
    initializeNode: () => call('initialize_node'),
    nodeIdentity: () => call('node_identity'),
    listWorlds: async () => {
      await ensureMigrationCapabilities();
      return call('list_worlds');
    },
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

    migration: Object.freeze({
      capabilities: migrationCapabilities,
      refreshCapabilities: ensureMigrationCapabilities,
      readState: async (world) => {
        await ensureMigrationCapabilities();
        if (!migrationCapabilities.status) throw unavailable('Host migration status');
        const raw = await call('migration_status', { world });
        try {
          return JSON.parse(String(raw));
        } catch (error) {
          throw new Error(`Migration status was not valid JSON: ${error}`);
        }
      },
      transferAuthority: async () => { throw unavailable('Transfer host'); },
      wakeWorld: async (world) => {
        await ensureMigrationCapabilities();
        if (!migrationCapabilities.wake) throw unavailable('Wake world');
        return call('wake_world', { world });
      },
    }),
  });
}
