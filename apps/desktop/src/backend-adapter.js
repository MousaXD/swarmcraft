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
  failed: 'failed',
});

const CONNECTIVITY_STATES = Object.freeze({
  nat_status_unknown: {
    kind: 'connecting',
    label: 'Checking connectivity',
    detail: 'SwarmCraft is still determining which network paths are available.',
  },
  direct_reachable: {
    kind: 'direct',
    label: 'Direct connection',
    detail: 'Other peers can reach this device directly.',
  },
  hole_punched: {
    kind: 'direct',
    label: 'Direct connection established',
    detail: 'SwarmCraft established a direct peer path through NAT traversal.',
  },
  relay_connected: {
    kind: 'relay',
    label: 'Connected through relay',
    detail: 'Peer traffic currently has a usable relay path.',
  },
  relay_required: {
    kind: 'limited',
    label: 'Relay needed',
    detail: 'A relay path is needed for reliable internet connectivity.',
  },
  private_unreachable: {
    kind: 'action',
    label: 'Connection needs attention',
    detail: 'This device does not currently have a usable inbound peer path.',
  },
  bootstrap_unavailable: {
    kind: 'action',
    label: 'Discovery unavailable',
    detail: 'SwarmCraft cannot currently reach bootstrap discovery infrastructure.',
  },
  no_viable_path: {
    kind: 'action',
    label: 'Could not reach other peers',
    detail: 'No direct, hole-punched, or relayed peer path is currently usable.',
  },
});

const HOST_READINESS_STATES = Object.freeze({
  safe: { label: 'Safe to shut down', kind: 'safe' },
  sleeping: { label: 'Safe to shut down', kind: 'safe' },
  world_will_stop: { label: 'World will go offline', kind: 'warning' },
  syncing: { label: 'Wait before shutting down', kind: 'syncing' },
  blocked_by_runtime: { label: 'Another host needs setup', kind: 'action' },
  blocked_by_mods: { label: 'Another host is missing mods', kind: 'action' },
  blocked_by_quorum: { label: 'Transfer hosting first', kind: 'action' },
  degraded_safety: { label: 'Host safety is degraded', kind: 'warning' },
  conflict: { label: 'World history needs attention', kind: 'danger' },
  not_current_host: { label: 'Shutdown safety not proven', kind: 'warning' },
  unknown: { label: 'Checking shutdown safety', kind: 'checking' },
});

function slug(value) {
  return String(value || '')
    .trim()
    .toLowerCase()
    .replace(/[_\s]+/g, '-')
    .replace(/[^a-z0-9-]/g, '');
}

function connectivityKey(value) {
  return String(value || '')
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, '_')
    .replace(/[^a-z0-9_]/g, '');
}

function compactList(value) {
  if (Array.isArray(value)) return value.map((item) => String(item).trim()).filter(Boolean);
  if (value === null || value === undefined || value === '') return [];
  return [String(value).trim()].filter(Boolean);
}

function connectivityFailures(source) {
  const failures = source.recent_failures ?? source.recentFailures ?? source.failures ?? source.failure_reasons ?? source.failureReasons ?? source.errors;
  if (!Array.isArray(failures)) return compactList(failures);
  return failures.map((failure) => {
    if (!failure || typeof failure !== 'object') return String(failure || '').trim();
    const kind = connectivityKey(failure.kind).replaceAll('_', ' ');
    const detail = String(failure.detail || '').trim();
    const peer = String(failure.peer || '').trim();
    const address = String(failure.address || '').trim();
    return [kind, detail, peer && `peer ${peer}`, address].filter(Boolean).join(': ');
  }).filter(Boolean);
}

export function normalizeConnectivityDiagnostics(raw) {
  const source = raw && typeof raw === 'object' ? raw : { state: raw };
  const state = connectivityKey(
    source.state
      ?? source.connectivity_state
      ?? source.connectivityState
      ?? source.path_state
      ?? source.pathState
      ?? source.status,
  );
  const mapped = CONNECTIVITY_STATES[state] || CONNECTIVITY_STATES.nat_status_unknown;
  const natState = source.nat_status ?? source.natStatus ?? source.nat_state ?? source.natState ?? null;
  const localAddresses = compactList(
    source.local_addresses
      ?? source.localAddresses
      ?? source.addresses
      ?? source.listen_addresses
      ?? source.listenAddresses,
  );
  const observedPublicAddress = String(source.observed_public_address ?? source.observedPublicAddress ?? '').trim();
  const selectedRelay = String(source.selected_relay ?? source.selectedRelay ?? '').trim();
  const lastFailure = String(source.last_failure ?? source.lastFailure ?? '').trim();
  const failures = connectivityFailures(source);
  const diagnosticParts = [];
  if (natState) diagnosticParts.push(`NAT: ${natState}`);
  if (localAddresses.length) diagnosticParts.push(`Local addresses: ${localAddresses.join(', ')}`);
  if (observedPublicAddress) diagnosticParts.push(`Observed public address: ${observedPublicAddress}`);
  if (selectedRelay) diagnosticParts.push(`Relay: ${selectedRelay}`);
  if (lastFailure) diagnosticParts.push(`Last failure: ${lastFailure}`);
  if (failures.length) diagnosticParts.push(`Recent failures: ${failures.join('; ')}`);

  return {
    state: CONNECTIVITY_STATES[state] ? state : 'nat_status_unknown',
    kind: mapped.kind,
    label: mapped.label,
    detail: mapped.detail,
    diagnosticDetail: diagnosticParts.join(' · ') || mapped.detail,
    raw: source,
  };
}

export function normalizeMigrationState(raw) {
  if (!raw) {
    return {
      available: false,
      phase: null,
      label: 'Not active',
      detail: 'No host migration is currently reported.',
      progress: 0,
      blocked: false,
      failed: false,
      action: null,
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
      blocked: false,
      failed: false,
      action: null,
      runtimeReady,
      gameEndpoint,
    };
  }

  if (backendPhase === 'blocked') {
    const runtimeConfigMissing = /runtime/i.test(failureReason)
      && /(config|configuration|setup|launch)/i.test(failureReason)
      && /(missing|not configured|unavailable|required)/i.test(failureReason);
    return {
      available: true,
      phase: 'blocked',
      label: 'Action required',
      detail: runtimeConfigMissing
        ? 'This device needs Minecraft runtime setup before it can take over hosting.'
        : failureReason || 'Host migration is blocked until the required local setup is completed.',
      progress: 0,
      blocked: true,
      failed: false,
      action: runtimeConfigMissing ? 'runtime-setup' : null,
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
  const detail = String(
    source.detail
      || source.message
      || failureReason
      || (runtimeReady && gameEndpoint ? `Minecraft is ready at ${gameEndpoint}.` : ''),
  ).trim();

  return {
    available: Boolean(knownPhase),
    phase: knownPhase,
    label: knownPhase ? MIGRATION_LABELS[knownPhase] : 'Migration state unavailable',
    detail,
    progress,
    blocked: false,
    failed: knownPhase === 'failed',
    action: null,
    runtimeReady,
    gameEndpoint,
  };
}

export function normalizeHostReadiness(raw) {
  const source = raw && typeof raw === 'object' ? raw : {};
  const state = connectivityKey(source.state || 'unknown');
  const mapped = HOST_READINESS_STATES[state] || HOST_READINESS_STATES.unknown;
  return {
    available: state !== 'unknown',
    state: HOST_READINESS_STATES[state] ? state : 'unknown',
    kind: mapped.kind,
    label: mapped.label,
    detail: String(source.detail || '').trim() || 'SwarmCraft has not yet proven whether this computer may safely shut down.',
    safeToShutdown: Boolean(source.safe_to_shutdown ?? source.safeToShutdown),
    successorPeerId: source.successor_peer_id ?? source.successorPeerId ?? null,
    handoffCandidatePeerId: source.handoff_candidate_peer_id ?? source.handoffCandidatePeerId ?? null,
    worldDataReplicated: Boolean(source.world_data_replicated ?? source.worldDataReplicated),
    peers: Array.isArray(source.peers) ? source.peers : [],
    raw: source,
  };
}

export function connectivityFromStatus(status = {}) {
  return normalizeConnectivityDiagnostics(status);
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
          // Manual transfer is deliberately disabled until migration-core exposes one
          // Desktop-safe orchestration command for the complete signed transfer flow.
          migrationCapabilities.transfer = false;
          return migrationCapabilities;
        })
        .catch(() => migrationCapabilities);
    }
    return migrationCapabilityProbe;
  };

  const configureWorldRuntime = (payload) => call('configure_world_runtime', {
    world: payload.world,
    java: payload.java,
    serverJar: payload.serverJar,
    modJar: payload.modJar,
    acceptEula: payload.acceptEula,
    gameEndpoint: payload.gameEndpoint || null,
  });

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
    hostReadiness: async (world) => {
      const raw = await call('host_readiness', { world });
      try {
        return normalizeHostReadiness(typeof raw === 'string' ? JSON.parse(raw) : raw);
      } catch (error) {
        throw new Error(`Host readiness was not valid JSON: ${error}`);
      }
    },
    worldCompatibility: (world) => call('world_compatibility', { world }),
    worldConflicts: (world) => call('world_conflicts', { world }),
    setBackgroundSeeding: (world, enabled) => call('set_background_seeding', { world, enabled }),
    worldPeers: (world) => call('world_peers', { world }),
    verifyWorld: (world) => call('verify_world', { world }),
    exportWorld: (world, destination) => call('export_world', { world, destination }),
    recoverWorld: (world, snapshot, destination) => call('recover_world', { world, snapshot, destination }),
    ensureDaemonRunning: (listen) => call('ensure_daemon_running', { listen }),
    startDaemon: (listen) => call('start_daemon', { listen }),
    stopDaemon: () => call('stop_daemon'),
    configureWorldRuntime,
    hostWorld: async (payload) => {
      try {
        await configureWorldRuntime(payload);
      } catch (error) {
        throw new Error(
          `Minecraft runtime setup could not be saved, so hosting was not started and automatic takeover is not prepared. ${String(error)}`,
        );
      }
      return call('host_world', payload);
    },
    stopHost: () => call('stop_host'),
    connectivityDiagnostics: async () => {
      const raw = await call('connectivity_diagnostics');
      let parsed = raw;
      if (typeof raw === 'string') {
        try {
          parsed = JSON.parse(raw);
        } catch (error) {
          throw new Error(`Connectivity diagnostics were not valid JSON: ${error}`);
        }
      }
      return normalizeConnectivityDiagnostics(parsed);
    },

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
