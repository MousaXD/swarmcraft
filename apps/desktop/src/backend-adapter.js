import { registerRuntimeWizard } from './runtime-wizard.js';

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

export const RUNTIME_COMPONENTS = Object.freeze([
  ['java', 'Java'],
  ['minecraft_server', 'Minecraft server'],
  ['fabric_loader', 'Fabric Loader'],
  ['fabric_api', 'Fabric API'],
  ['swarmcraft_integration', 'SwarmCraft integration'],
  ['world_directories', 'World directories'],
  ['server_mods', 'Server mods'],
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
  blocked_by_runtime: { label: 'Keep this PC on', kind: 'action' },
  blocked_by_mods: { label: 'Keep this PC on', kind: 'action' },
  blocked_by_quorum: { label: 'World will go offline', kind: 'action' },
  degraded_safety: { label: 'Keep this PC on', kind: 'warning' },
  conflict: { label: 'Host handoff unavailable', kind: 'danger' },
  not_current_host: { label: 'Shutdown safety not proven', kind: 'warning' },
  unknown: { label: 'Checking shutdown safety', kind: 'checking' },
});

const RUNTIME_COMPONENT_ALIASES = Object.freeze({
  java: ['java'],
  minecraft_server: ['minecraft_server', 'minecraftServer', 'server', 'minecraft'],
  fabric_loader: ['fabric_loader', 'fabricLoader', 'loader'],
  fabric_api: ['fabric_api', 'fabricApi'],
  swarmcraft_integration: ['swarmcraft_integration', 'swarmcraftIntegration', 'swarmcraft_mod', 'swarmcraftMod', 'swarmcraft_fabric', 'mod'],
  world_directories: ['world_directories', 'worldDirectories', 'server_directories', 'directories'],
  server_mods: ['server_mods', 'serverMods', 'mods'],
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

function parseJsonContract(raw, label) {
  if (raw && typeof raw === 'object') return raw;
  try {
    return JSON.parse(String(raw));
  } catch (error) {
    throw new Error(`${label} was not valid JSON: ${error}`);
  }
}

function optionalBoolean(value) {
  return typeof value === 'boolean' ? value : null;
}

function runtimeState(value) {
  const state = connectivityKey(value);
  if (['ready', 'installed', 'managed', 'compatible', 'present', 'verified'].includes(state)) return 'ready';
  if (['checking', 'pending', 'downloading', 'installing', 'preparing', 'verifying', 'working'].includes(state)) return 'working';
  if (['missing', 'required', 'not_installed', 'absent'].includes(state)) return 'missing';
  if (['incompatible', 'wrong_version', 'unsupported'].includes(state)) return 'incompatible';
  if (['corrupt', 'invalid', 'hash_mismatch', 'damaged'].includes(state)) return 'corrupt';
  if (['failed', 'error'].includes(state)) return 'failed';
  return 'unknown';
}

function normalizeRuntimeComponent(id, label, value) {
  const source = value && typeof value === 'object' ? value : { state: value };
  return {
    id,
    label,
    state: runtimeState(source.state ?? source.status ?? source.kind ?? value),
    detail: String(source.detail ?? source.message ?? source.reason ?? '').trim(),
    version: String(source.version ?? source.resolved_version ?? source.resolvedVersion ?? '').trim(),
    required: source.required !== false,
    raw: source,
  };
}

export function normalizeRuntimeStatus(raw) {
  const envelope = parseJsonContract(raw, 'Runtime status');
  const source = envelope.status && typeof envelope.status === 'object' ? envelope.status : envelope;
  const rawComponents = source.components ?? source.runtime_components ?? source.runtimeComponents ?? {};
  const componentSource = Array.isArray(rawComponents)
    ? Object.fromEntries(rawComponents.map((component) => [component.kind ?? component.id, component]))
    : rawComponents;
  const components = RUNTIME_COMPONENTS.map(([id, label]) => {
    const aliases = RUNTIME_COMPONENT_ALIASES[id] || [id];
    let value;
    for (const alias of aliases) {
      if (Object.prototype.hasOwnProperty.call(componentSource, alias)) {
        value = componentSource[alias];
        break;
      }
      if (Object.prototype.hasOwnProperty.call(source, alias)) {
        value = source[alias];
        break;
      }
    }
    return normalizeRuntimeComponent(id, label, value);
  });
  const completedPhases = envelope.completed_phases ?? envelope.completedPhases ?? [];
  const inferredPhase = Array.isArray(completedPhases) && completedPhases.length ? completedPhases.at(-1) : null;
  const overall = connectivityKey(source.state ?? source.status ?? source.phase ?? inferredPhase);
  const eulaAccepted = Boolean(source.eula_accepted ?? source.eulaAccepted ?? source.accept_eula ?? source.acceptEula);
  const eulaComponent = Array.isArray(rawComponents)
    ? rawComponents.find((component) => connectivityKey(component.kind ?? component.id) === 'eula')
    : componentSource.eula;
  const eulaRequired = !eulaAccepted && Boolean(
    source.eula_required
      ?? source.eulaRequired
      ?? (overall === 'eula_required')
      ?? false
  ) || (!eulaAccepted && connectivityKey(eulaComponent?.state ?? eulaComponent?.status) === 'required');
  const ready = Boolean(source.ready ?? source.runtime_ready ?? source.runtimeReady ?? overall === 'ready');
  const failure = source.failure ?? source.error ?? source.failure_reason ?? source.failureReason ?? null;
  const diagnosticDetail = failure && typeof failure === 'object'
    ? String(failure.detail ?? failure.message ?? JSON.stringify(failure)).trim()
    : String(failure ?? source.diagnostic_detail ?? source.diagnosticDetail ?? '').trim();

  return {
    ready,
    phase: connectivityKey(source.phase ?? source.state ?? source.status) || (ready ? 'ready' : 'checking'),
    detail: String(source.detail ?? source.message ?? '').trim(),
    eulaAccepted,
    eulaRequired,
    components,
    worldDataSafe: optionalBoolean(source.world_data_safe ?? source.worldDataSafe),
    retrySafe: optionalBoolean(source.retry_safe ?? source.retrySafe),
    diagnosticDetail,
    raw: source,
  };
}

function missingCommand(error) {
  const message = String(error || '');
  return /unknown command|command [^\n]*(?:not found|does not exist|not registered)|not a registered command/i.test(message);
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

  const callOptional = async (command, payload, feature) => {
    try {
      return await call(command, payload);
    } catch (error) {
      if (missingCommand(error)) throw unavailable(feature);
      throw error;
    }
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

  const adapter = Object.freeze({
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
    stopHost: (world) => call('stop_host', { world }),
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

    mods: Object.freeze({
      status: async (world) => parseJsonContract(
        await call('world_mods_status', { world }),
        'Server mod status',
      ),
      supplyRequiredJar: (world, jarPath) => call('world_mods_add', { world, jarPath }),
      removeLocal: (world, modId) => call('world_mods_remove', { world, modId }),
      openFolder: (world) => call('open_world_mods_folder', { world }),
    }),

    runtime: Object.freeze({
      status: async (world) => normalizeRuntimeStatus(await callOptional(
        'runtime_status',
        { world },
        'Automatic runtime setup',
      )),
      plan: async (world) => parseJsonContract(await callOptional(
        'runtime_plan',
        { world },
        'Runtime setup planning',
      ), 'Runtime plan'),
      install: async (world, options = {}) => normalizeRuntimeStatus(await callOptional(
        'runtime_install',
        { world, acceptEula: options.acceptEula === true },
        'Automatic runtime installation',
      )),
      repair: async (world) => normalizeRuntimeStatus(await callOptional(
        'runtime_repair',
        { world },
        'Automatic runtime repair',
      )),
      verify: async (world) => normalizeRuntimeStatus(await callOptional(
        'runtime_verify',
        { world },
        'Runtime verification',
      )),
      launch: (world) => callOptional('runtime_launch', { world }, 'Managed runtime launch'),
    }),

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

  registerRuntimeWizard(adapter);
  return adapter;
}
