import { createBackendAdapter, connectivityFromStatus, MIGRATION_PHASES, normalizeMigrationState } from './backend-adapter.js';
import { createImportRequest, parseImportResult } from './import-flow.js';
import { installImportReview } from './import-review.js';
import { normalizeRecoveryReview, recoveryCopyReceipt, recoveryExportReceipt } from './recovery-copy.js';
import { validateInviteToken, inspectInviteReceipt, revocationRequest, revokeInviteReceipt, pendingMembershipMessage } from './invite-lifecycle.js';
import { clientContinuityFromMigration, ensureClientContinuityPanel } from './client-continuity.js';
import { UNVERIFIED_GAME_ROUTE, connectionDiagnosis, ensureGameReconnectActions, gameReadinessCheckpoints, isUsableGameRoute, normalizeGameRouteStatus, updateGameReadinessChecklist, updateGameRouteControls } from './game-route-status.js';

const $ = (id) => document.getElementById(id);
const invoke = window.__TAURI__?.core?.invoke;
const backend = createBackendAdapter(invoke);
const worldCache = new Map();
let selectedWorldId = '';
let currentView = 'worlds';
let migrationRequestGeneration = 0;
let hostReadinessRequestGeneration = 0;
let gameRouteRequestGeneration = 0;
let gameRouteInFlightWorld = '';
// World-switch operations must be serialized: a slow selection of world A
// must never re-open A's gateway after the player has moved to world B.
let gatewaySelectionQueue = Promise.resolve();
let gameRouteState = UNVERIFIED_GAME_ROUTE;
let latestMigrationState = null;
let localCheckpointVerification = null;
let latestHostReadiness = null;
let modsRequestGeneration = 0;
let connectivityRefreshInFlight = false;
let connectivityGeneration = 0;
let networkStoppedBySvelte = false;
let structuredConnectivity = connectivityFromStatus({ state: 'nat_status_unknown' });
let serviceWarning = '';
let startupInProgress = false;
let lastWorldError = null;
let recoveryCopyTarget = null;
let recoveryReview = null;
let recoveryCopyPending = false;
let inviteManagementWorld = '';
let inviteManagementSelection = null;
let inviteManagementBusy = false;
let membershipRequestGeneration = 0;
const MIGRATION_REFRESH_MS = 5000;
const CONNECTIVITY_REFRESH_MS = 10000;

const viewMeta = {
  worlds: ['Worlds', 'Choose a world and play'],
  create: ['Create', 'Start a replicated Minecraft world'],
  import: ['Import', 'Bring an existing Minecraft world into SwarmCraft'],
  join: ['Join', 'Join with a signed invite'],
  activity: ['Activity', 'Recent actions and detailed errors'],
  diagnostics: ['Diagnostics', 'Advanced connectivity, runtime, and recovery tools'],
};

const migrationLabels = {
  'preparing-successor': 'Preparing successor',
  'saving-world': 'Saving world',
  'transferring-authority': 'Transferring authority',
  'restoring-world': 'Restoring world',
  'starting-minecraft': 'Starting Minecraft',
  'waiting-for-host': 'Waiting for host',
  ready: 'Ready',
  failed: 'Migration failed',
};

function setStatus(message, tone = 'neutral') {
  if (window.__swarmcraftStartup?.failed) return;
  // A candidate-specific daemon stop or uncertain side effect must not be
  // repainted as green Ready by an unrelated world-list refresh or action.
  // Shipping legacy semantics remain unchanged without Svelte ownership.
  if (window.__swarmcraftSvelteNetworkingOwned === true && message === 'Ready'
      && (networkStoppedBySvelte || serviceWarning)) {
    message = 'Networking needs attention';
    tone = 'warning';
  }
  $('status').textContent = message;
  $('statusDot').className = `status-dot ${tone}`;
}

function setOutput(output, label = '') {
  const text = String(output || 'Done.');
  const stamp = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const entry = `${stamp}${label ? `  ${label}` : ''}\n${text}`;
  // The Svelte migration may own activity state. The unmodified shipping
  // frontend has no bridge and keeps the original direct DOM behavior.
  if (window.__swarmcraftSvelteActivity?.append(entry) === true) return;
  const console = $('output');
  console.textContent = ['SwarmCraft is ready.', 'SwarmCraft is starting.', 'Activity cleared.'].includes(console.textContent.trim())
    ? entry
    : `${console.textContent.trim()}\n\n${entry}`;
  console.scrollTop = console.scrollHeight;
}

function showInline(id, message, tone = 'neutral') {
  const element = $(id);
  if (!element) return;
  element.textContent = message;
  element.hidden = !message;
  element.dataset.tone = tone;
}

function serviceWarningElement() {
  let element = $('serviceWarning');
  if (element) return element;
  element = document.createElement('div');
  element.id = 'serviceWarning';
  element.className = 'page-notice notice-danger';
  element.setAttribute('role', 'status');
  element.setAttribute('aria-live', 'polite');
  element.hidden = true;
  document.querySelector('.topbar')?.insertAdjacentElement('afterend', element);
  return element;
}

function setServiceWarning(message) {
  serviceWarning = String(message || '').trim();
  const element = serviceWarningElement();
  element.textContent = serviceWarning;
  element.hidden = !serviceWarning;
}

function activityLabel(label) {
  return String(label).replace(/…$/, '');
}

async function run(label, work, { quiet = false, successMessage = null, logResult = true } = {}) {
  if (!quiet) setStatus(label);
  try {
    const result = await work();
    if (!quiet) {
      const message = typeof successMessage === 'function' ? successMessage(result) : successMessage;
      if (message) setOutput(message, activityLabel(label));
      else if (logResult && result !== undefined && result !== null && String(result).trim()) setOutput(result, activityLabel(label));
      if (!startupInProgress) setStatus('Ready', 'safe');
    }
    return result;
  } catch (error) {
    if (!quiet) {
      setOutput(String(error), `${activityLabel(label)} failed`);
      setStatus('Action failed', 'danger');
      try { error.swarmcraftActivityLogged = true; } catch (_) {}
    }
    throw error;
  }
}

function showView(name, { focus = true } = {}) {
  if (!viewMeta[name]) return;
  currentView = name;
  for (const panel of document.querySelectorAll('[data-view-panel]')) {
    const active = panel.dataset.viewPanel === name;
    panel.hidden = !active;
    panel.classList.toggle('is-active', active);
  }
  for (const nav of document.querySelectorAll('.nav-item[data-view]')) {
    const active = nav.dataset.view === name;
    nav.classList.toggle('is-active', active);
    if (active) nav.setAttribute('aria-current', 'page');
    else nav.removeAttribute('aria-current');
  }
  const [title, context] = viewMeta[name];
  $('viewTitle').textContent = title;
  $('viewContext').textContent = context;
  if (name === 'diagnostics') {
    updateDiagnosticsContext();
    // Svelte candidate owns the Diagnostics network display. Shipping legacy
    // keeps the exact existing backend refresh when that owner is absent.
    if (window.__swarmcraftSvelteConnectivityOwned === true) {
      void window.__swarmcraftSvelteConnectivity?.refresh();
    } else {
      refreshConnectivityDiagnostics({ logFailure: false }).catch(() => {});
    }
  }
  if (focus) {
    const panel = document.querySelector(`[data-view-panel="${name}"]`);
    panel?.querySelector('input:not([type="hidden"]):not(:disabled), textarea:not(:disabled), button:not(:disabled), summary, [tabindex]')?.focus({ preventScroll: true });
  }
}

function parseLines(text) {
  const values = {};
  for (const line of String(text || '').split('\n')) {
    const index = line.indexOf(':');
    if (index > 0) values[line.slice(0, index).trim()] = line.slice(index + 1).trim();
  }
  return values;
}

function parseWorldList(text) {
  return String(text || '')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.startsWith('scworld:'))
    .map((line) => {
      const match = line.match(/^(scworld:[^\s]+)\s+(.*?)\s+snapshots=(\d+)$/);
      return match ? { id: match[1], name: match[2], snapshots: Number(match[3]) } : null;
    })
    .filter(Boolean);
}

function safetyKind(safety) {
  const value = String(safety || '').toLowerCase();
  if (value.includes('conflict')) return 'danger';
  if (value.includes('solo') || value.includes('degraded')) return 'warning';
  if (value.includes('canonical') || value.includes('quorum')) return 'safe';
  return 'neutral';
}

function safetySummary(world) {
  const kind = safetyKind(world?.status?.Safety);
  if (kind === 'danger') return 'Divergent history is preserved. Review conflicts before hosting or recovering this world.';
  if (kind === 'warning') return 'This world has reduced safety guarantees while solo or degraded history is active.';
  if (kind === 'safe') return 'Canonical world history is available. Connectivity and replica availability are tracked separately.';
  return 'Safety state is not available yet. Refresh before making recovery or hosting decisions.';
}

function hostingEligibility(world) {
  if (!world) return { enabled: false, reason: 'Choose a world first.' };
  if (pendingMembershipMessage(world.status?.Membership)) {
    return { enabled: false, reason: pendingMembershipMessage(world.status.Membership) };
  }
  if (safetyKind(world.status.Safety) === 'danger') {
    return { enabled: false, reason: 'Play is unavailable while divergent history needs review.' };
  }
  const value = String(world.compatibility?.['Authority eligibility'] || '').trim();
  if (value === 'Compatible') {
    return { enabled: true, reason: 'This device is compatible with the world. Local hosting requires opt-in.' };
  }
  if (value.toLowerCase().includes('not authority eligible') || value.toLowerCase().startsWith('replica only')) {
    return { enabled: false, reason: 'This device can keep a replica, but it is not eligible to host this world.' };
  }
  return { enabled: false, reason: 'Play is unavailable until compatibility and host eligibility are known.' };
}

function selectedWorld() {
  return worldCache.get(selectedWorldId) || null;
}

function worldId() {
  const value = $('world').value.trim();
  if (!value) throw new Error('Choose a world first.');
  return value;
}

function setNodeState(text, kind = 'neutral') {
  $('nodeBadge').textContent = text;
  $('nodeStateDot').className = `state-dot ${kind}`;
}

function renderConnectivity() {
  const connectivity = structuredConnectivity;
  $('selectedConnectivity').textContent = connectivity.label;
  $('selectedConnectivityDetail').textContent = connectivity.detail;
  window.__swarmcraftSvelteWorlds?.setConnectivity(connectivity.label);
  if (window.__swarmcraftSvelteConnectivityOwned !== true) {
    $('diagnosticConnectivity').textContent = connectivity.label;
    $('diagnosticConnectivity').className = `status-badge ${connectivity.kind}`;
    $('diagnosticConnectivityDetail').textContent = connectivity.diagnosticDetail;
  }
}

async function refreshConnectivityDiagnostics({ logFailure = false } = {}) {
  // An intentionally stopped local daemon cannot be classified as reachable
  // from a stale persisted connectivity snapshot in the Svelte candidate.
  if (window.__swarmcraftSvelteNetworkingOwned === true && networkStoppedBySvelte) return;
  if (connectivityRefreshInFlight) return;
  const readGeneration = connectivityGeneration;
  connectivityRefreshInFlight = true;
  try {
    const result = await backend.connectivityDiagnostics();
    if (readGeneration !== connectivityGeneration) return;
    structuredConnectivity = result;
  } catch (error) {
    if (readGeneration !== connectivityGeneration) return;
    structuredConnectivity = connectivityFromStatus({ state: 'nat_status_unknown' });
    structuredConnectivity = {
      ...structuredConnectivity,
      diagnosticDetail: `Connectivity diagnostics unavailable: ${String(error)}`,
    };
    if (logFailure) setOutput(String(error), 'Connectivity diagnostics failed');
  } finally {
    connectivityRefreshInFlight = false;
  }
  if (readGeneration !== connectivityGeneration) return;
  renderConnectivity();
  for (const element of document.querySelectorAll('.world-row-connectivity')) {
    element.textContent = structuredConnectivity.label;
  }
}

function updatePlayState(world) {
  const eligibility = hostingEligibility(world);
  $('playWorld').disabled = !eligibility.enabled;
  $('playAvailability').textContent = eligibility.reason;
  $('playAvailability').dataset.tone = eligibility.enabled ? 'safe' : 'warning';
  $('runtimeEligibilityHint').textContent = eligibility.reason;
  updateWorldSpecificControls();
}

function updateWorldSpecificControls() {
  const world = selectedWorld();
  const hasWorld = Boolean(world);
  const eligibility = hostingEligibility(world);
  for (const control of document.querySelectorAll('.world-required')) control.disabled = !hasWorld;
  for (const control of document.querySelectorAll('.authority-required')) control.disabled = !hasWorld || !eligibility.enabled;
  $('playWorld').disabled = !hasWorld || !eligibility.enabled;

  const migration = backend.migration.capabilities;
  $('transferHost').disabled = !hasWorld || !migration.transfer || !eligibility.enabled;
  $('wakeWorld').disabled = !hasWorld || !migration.wake;
  $('transferAvailability').hidden = migration.transfer;
  $('transferAvailability').textContent = migration.transfer ? '' : 'Manual host transfer is not available in this build.';
  if (migration.transfer && migration.wake) {
    $('migrationAvailability').textContent = 'Transfer and wake actions are available only after backend safety checks pass.';
  } else if (migration.wake) {
    $('migrationAvailability').textContent = 'Safe wake is available when the backend allows it. Manual host transfer remains unavailable.';
  } else if (migration.transfer) {
    $('migrationAvailability').textContent = 'Host transfer is available when backend safety checks pass. Wake is unavailable in this build.';
  } else {
    $('migrationAvailability').textContent = 'Transfer and wake are unavailable in this build. Existing Play and graceful stop remain available.';
  }
}

function clearSelection() {
  membershipRequestGeneration += 1;
  showInline('membershipNotice', '');
  if ($('refreshMembership')) $('refreshMembership').hidden = true;
  selectedWorldId = '';
  migrationRequestGeneration += 1;
  hostReadinessRequestGeneration += 1;
  gameRouteRequestGeneration += 1;
  gameRouteState = UNVERIFIED_GAME_ROUTE;
  localCheckpointVerification = null;
  modsRequestGeneration += 1;
  $('world').value = '';
  $('selectionContent').hidden = true;
  $('noSelection').hidden = false;
  if (window.__swarmcraftSvelteWorldsOwned === true) {
    window.__swarmcraftSvelteWorlds?.setSelected('');
  } else {
    for (const row of document.querySelectorAll('.world-row')) {
      row.classList.remove('is-selected');
      row.setAttribute('aria-pressed', 'false');
    }
  }
  showInline('worldNotice', '');
  showInline('runtimeNotice', '');
  if (window.__swarmcraftSvelteReadinessOwned === true) {
    window.__swarmcraftSvelteReadiness?.clear();
  } else {
    renderHostReadiness(null);
  }
  updateWorldSpecificControls();
  updateDiagnosticsContext();
  renderMigration(null);
  renderGameRoute();
}

function updateDiagnosticsContext() {
  const world = selectedWorld();
  if (!world) {
    $('diagnosticWorldName').textContent = 'No world selected';
    $('diagnosticWorldId').textContent = 'World-specific controls are unavailable.';
    $('diagnosticSafety').hidden = true;
    renderConnectivity();
    return;
  }
  $('diagnosticWorldName').textContent = world.name;
  $('diagnosticWorldId').textContent = world.id;
  const safety = world.status.Safety || 'Unknown';
  $('diagnosticSafety').textContent = safety;
  $('diagnosticSafety').className = `status-badge ${safetyKind(safety)}`;
  $('diagnosticSafety').hidden = false;
  renderConnectivity();
}

function runtimeSetupActionButton() {
  let button = $('migrationRuntimeSetup');
  if (button) return button;
  button = document.createElement('button');
  button.id = 'migrationRuntimeSetup';
  button.type = 'button';
  button.className = 'button button-secondary';
  button.textContent = 'Set up Minecraft runtime';
  button.hidden = true;
  button.addEventListener('click', () => showRuntimeValidation([
    null,
    'This device needs Minecraft runtime setup before it can take over hosting.',
  ]));
  $('migrationCard')?.querySelector('.compact-actions')?.prepend(button);
  return button;
}

function renderMigration(state) {
  latestMigrationState = state;
  const mismatched = Boolean(state && typeof state === 'object' && 'phase' in state && state.world_id !== selectedWorldId);
  const migration = normalizeMigrationState(mismatched
    ? { phase: 'blocked', failure_reason: 'Host status was not bound to the selected world.' }
    : state);
  const continuity = clientContinuityFromMigration(state, selectedWorldId, selectedWorld()?.status?.Safety, gameRouteState);
  const targets = ensureClientContinuityPanel(document);
  if (targets) {
    targets.title.textContent = continuity.title;
    targets.detail.textContent = continuity.detail;
    targets.detail.dataset.tone = continuity.tone;
  }
  $('migrationBadge').textContent = migration.label;
  // The persisted Ready phase can survive a hard crash. It is not a live
  // Minecraft process check or a signed client route, so never paint it green.
  $('migrationBadge').className = `status-badge ${migration.failed || continuity.tone === 'danger' ? 'danger' : migration.available ? 'warning' : 'neutral'}`;
  $('migrationSummary').textContent = migration.detail || (migration.available ? migration.label : 'No host migration is active.');
  $('migrationProgress').hidden = !migration.available || migration.failed || migration.blocked || migration.phase === 'ready';
  $('migrationProgressBar').style.width = `${migration.progress}%`;
  runtimeSetupActionButton().hidden = migration.action !== 'runtime-setup';

  const steps = $('migrationSteps');
  steps.replaceChildren();
  if (!migration.available || migration.failed || migration.blocked) return;
  const activeIndex = MIGRATION_PHASES.indexOf(migration.phase);
  MIGRATION_PHASES.filter((phase) => phase !== 'failed').forEach((phase) => {
    const item = document.createElement('li');
    const index = MIGRATION_PHASES.indexOf(phase);
    item.textContent = migrationLabels[phase];
    if (migration.phase === 'failed' && index <= activeIndex) item.classList.add('is-failed');
    else if (phase === migration.phase) item.classList.add(migration.failed ? 'is-failed' : 'is-current');
    else if (index < activeIndex && migration.phase !== 'failed') item.classList.add('is-complete');
    steps.append(item);
  });
}

function renderGameRoute() {
  updateGameReadinessChecklist(document, gameReadinessCheckpoints({
    world: selectedWorld(), route: gameRouteState,
    localVerification: localCheckpointVerification,
    readiness: latestHostReadiness,
  }));
  ensureGameReconnectActions(document);
  const route = gameRouteState;
  const diagnosis = connectionDiagnosis(route.verified && !isUsableGameRoute(route, selectedWorldId, selectedWorld()?.status?.Safety) ? UNVERIFIED_GAME_ROUTE : route, selectedWorldId);
  if ($('routeDiagnosis')) $('routeDiagnosis').textContent =
    diagnosis.route + '\n' + diagnosis.detail + '\n' + diagnosis.session;
  const routeVerified = updateGameRouteControls(document, route, selectedWorldId, selectedWorld()?.status?.Safety);
  // Also revoke the accessible continuity headline immediately. Do not leave
  // a green route claim visible between a stop, tab switch and next CLI poll.
  const continuity = clientContinuityFromMigration(
    latestMigrationState, selectedWorldId, selectedWorld()?.status?.Safety,
    routeVerified ? route : UNVERIFIED_GAME_ROUTE,
  );
  const continuityTargets = ensureClientContinuityPanel(document);
  if (continuityTargets) {
    continuityTargets.title.textContent = continuity.title;
    continuityTargets.detail.textContent = continuity.detail;
    continuityTargets.detail.dataset.tone = continuity.tone;
  }
}

function invalidateGameRoute() {
  gameRouteRequestGeneration += 1;
  gameRouteState = UNVERIFIED_GAME_ROUTE;
  renderGameRoute();
}

async function refreshGameRoute(world) {
  const requestedWorldId = world?.id;
  // A fresh signed offer + challenge + independently collected quorum may
  // legitimately take longer than one UI polling interval. Coalesce only
  // concurrent requests for the SAME world: superseding each slow request
  // every five seconds would prevent even a valid route from being shown.
  // World switches still increment the generation and immediately invalidate.
  if (requestedWorldId && gameRouteInFlightWorld === requestedWorldId) return;
  gameRouteInFlightWorld = requestedWorldId || '';
  const generation = ++gameRouteRequestGeneration;
  // Never keep an old route live while a refresh, world switch, or a daemon
  // error is pending. Each poll must produce a fresh independent proof.
  gameRouteState = UNVERIFIED_GAME_ROUTE;
  renderGameRoute();
  if (!requestedWorldId) return;
  try {
    let timer;
    const result = await Promise.race([
      backend.gameRouteStatus(requestedWorldId),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(normalizeGameRouteStatus({ world_id: requestedWorldId, state: 'unavailable', route_verified: false, diagnostic: 'route_check_timed_out' }, requestedWorldId)), 24_000);
      }),
    ]).finally(() => clearTimeout(timer));
    if (requestedWorldId !== selectedWorldId || generation !== gameRouteRequestGeneration) return;
    gameRouteState = result;
    if (result.verified === true && Number.isSafeInteger(result.validUntil)) {
      // Invalidate at the earlier proof expiry, even when a poll is delayed.
      // The generation guard prevents older timers from revoking newer proof.
      setTimeout(() => {
        if (generation === gameRouteRequestGeneration && gameRouteState === result) {
          invalidateGameRoute();
        }
      }, Math.max(0, result.validUntil - Date.now()) + 1);
    }
    else if (Number.isSafeInteger(result.diagnosticValidUntil)) {
      setTimeout(() => {
        if (generation === gameRouteRequestGeneration && gameRouteState === result) invalidateGameRoute();
      }, Math.max(0, result.diagnosticValidUntil - Date.now()) + 1);
    }
  } catch (_) {
    if (requestedWorldId !== selectedWorldId || generation !== gameRouteRequestGeneration) return;
    gameRouteState = UNVERIFIED_GAME_ROUTE;
  } finally {
    if (gameRouteInFlightWorld === requestedWorldId) gameRouteInFlightWorld = '';
  }
  renderGameRoute();
  // A new signed route can become available even though the local migration
  // report still describes the old authority; never fabricate that proof.
  await refreshMigrationState(selectedWorld());
}

async function retryConnectionDiagnosis() {
  const world = selectedWorld();
  if (!world) return;
  if (gameRouteInFlightWorld === world.id) return;
  await queueGameGatewaySelection(world);
}

function copyConnectionDiagnosis() {
  if (!selectedWorldId) return;
  const route = gameRouteState;
  const diagnosis = connectionDiagnosis(route.verified && !isUsableGameRoute(route, selectedWorldId, selectedWorld()?.status?.Safety) ? UNVERIFIED_GAME_ROUTE : route, selectedWorldId);
  void copyText(['SwarmCraft connection diagnosis', diagnosis.route, diagnosis.detail, diagnosis.session].join('\n'), 'Connection diagnosis');
}

function refreshVisibleGameRoute() {
  if (document.hidden) {
    gameRouteRequestGeneration += 1;
    gameRouteState = UNVERIFIED_GAME_ROUTE;
    renderGameRoute();
    return;
  }
  const world = selectedWorld();
  if (world) void refreshGameRoute(world);
}

function reconnectWorld() {
  const route = gameRouteState;
  if (!isUsableGameRoute(route, selectedWorldId, selectedWorld()?.status?.Safety)) {
    gameRouteState = UNVERIFIED_GAME_ROUTE;
    renderGameRoute();
    throw new Error('No current authenticated Minecraft route. Refresh the world before reconnecting.');
  }
  // This is an explicit manual reconnect instruction, never an attempt to
  // migrate an existing game TCP session or launch an unverified process.
  showInline('worldNotice', `Open Minecraft Java Multiplayer and connect to ${route.connectionTarget}. Disconnect from the old server first.`, 'safe');
}

async function refreshMigrationState(world) {
  if (!world || !backend.migration.capabilities.status) {
    renderMigration(null);
    return;
  }
  const requestedWorldId = world.id;
  const requestGeneration = ++migrationRequestGeneration;
  try {
    const state = await backend.migration.readState(requestedWorldId);
    if (selectedWorldId === requestedWorldId && requestGeneration === migrationRequestGeneration) renderMigration(state);
  } catch (error) {
    if (selectedWorldId === requestedWorldId && requestGeneration === migrationRequestGeneration) {
      renderMigration({ world_id: requestedWorldId, phase: 'status_unavailable', detail: `Could not read host migration state: ${String(error)}` });
    }
  }
}

function refreshVisibleMigration() {
  if (document.hidden || !backend.migration.capabilities.status) return;
  const world = selectedWorld();
  if (world) refreshMigrationState(world);
}


function renderHostReadiness(readiness) {
  latestHostReadiness = readiness;
  renderGameRoute();
  const panel = $('hostReadinessPanel');
  if (!panel) return;
  const claimed = readiness?.state || 'unknown';
  const proofIsCurrent = readiness?.worldId === selectedWorldId
    && Number.isSafeInteger(readiness?.validUntil) && readiness.validUntil > Date.now();
  const state = (claimed === 'safe' || claimed === 'sleeping')
    && (readiness?.safeToShutdown !== true || !proofIsCurrent)
    ? 'unknown' : claimed;
  const copy = {
    safe: ['Safe to shut down this PC', 'Another ready device can take over this world.'],
    sleeping: ['Safe to shut down this PC', 'This world is durably stopped and its latest checkpoint is saved.'],
    blocked_by_runtime: ['Keep this PC on', 'Another device has a current copy, but its Minecraft runtime is not ready.'],
    blocked_by_mods: ['Keep this PC on', 'Another device is missing or has incompatible required server mods.'],
    syncing: ['Wait before shutting down', 'Another device is still syncing the latest world state.'],
    world_will_stop: ['World will go offline', 'No other ready host is currently reachable.'],
    blocked_by_quorum: ['World will go offline', 'Another device has a copy, but the remaining members cannot safely complete host takeover.'],
    conflict: ['Host handoff unavailable', 'This world has conflicting history that needs attention.'],
    degraded_safety: ['Keep this PC on', 'SwarmCraft cannot currently prove a safe host takeover.'],
    not_current_host: ['Shutdown safety not proven', 'This device is not the current host; SwarmCraft is not claiming takeover safety from this report.'],
    unknown: ['Checking shutdown safety…', readiness?.detail || 'SwarmCraft has not produced a fresh host-readiness report yet.'],
  };
  const [title, detail] = copy[state] || copy.unknown;
  const tone = state === 'safe' || state === 'sleeping'
    ? 'safe'
    : state === 'conflict'
      ? 'danger'
      : state === 'unknown'
        ? 'neutral'
        : 'warning';
  panel.className = `safety-panel ${tone}`;
  $('hostReadinessTitle').textContent = title;
  $('hostReadinessDetail').textContent = detail;
}

async function refreshHostReadiness(world) {
  if (window.__swarmcraftSvelteReadinessOwned === true) {
    // Candidate view uses only the actual Rust host_readiness command.
    // No duplicate legacy command, legacy DOM overwrite or renderer grant.
    if (!world) window.__swarmcraftSvelteReadiness?.clear();
    else await window.__swarmcraftSvelteReadiness?.select(world.id);
    return;
  }
  if (!world) {
    renderHostReadiness(null);
    return;
  }
  const requestedWorldId = world.id;
  const requestGeneration = ++hostReadinessRequestGeneration;
  try {
    const readiness = await backend.hostReadiness(requestedWorldId);
    if (selectedWorldId === requestedWorldId && requestGeneration === hostReadinessRequestGeneration) {
      renderHostReadiness(readiness);
      if (Number.isSafeInteger(readiness.validUntil)) {
        setTimeout(() => {
          if (selectedWorldId === requestedWorldId && requestGeneration === hostReadinessRequestGeneration) {
            renderHostReadiness(null);
          }
        }, Math.max(0, readiness.validUntil - Date.now()) + 1);
      }
    }
  } catch (error) {
    if (selectedWorldId === requestedWorldId && requestGeneration === hostReadinessRequestGeneration) {
      renderHostReadiness({ state: 'unknown', detail: `Shutdown safety is unavailable: ${String(error)}` });
    }
  }
}

function refreshVisibleHostReadiness() {
  if (document.hidden) {
    hostReadinessRequestGeneration += 1;
    if (window.__swarmcraftSvelteReadinessOwned === true) {
      window.__swarmcraftSvelteReadiness?.clear();
    } else {
      renderHostReadiness(null);
    }
    return;
  }
  const world = selectedWorld();
  if (world) void refreshHostReadiness(world);
}

function invalidateHostReadiness() {
  hostReadinessRequestGeneration += 1;
  if (window.__swarmcraftSvelteReadinessOwned === true) {
    window.__swarmcraftSvelteReadiness?.clear();
  } else {
    renderHostReadiness(null);
  }
}

// The signed-transfer wizard and Svelte hosting-consent controls are mounted
// separately from this legacy controller. They may revoke stale presentation
// claims before native mutations, but they may never manufacture authority.
window.__swarmcraftInvalidateTransientSafety = (world) => {
  if (!selectedWorldId || world !== selectedWorldId) return;
  invalidateGameRoute();
  invalidateHostReadiness();
};

function modComponent(runtimeStatus, id) {
  return runtimeStatus?.components?.find((component) => component.id === id) || null;
}

function renderWorldMods(runtimeStatus, modsStatus) {
  const fabricApi = modComponent(runtimeStatus, 'fabric_api');
  const swarmcraft = modComponent(runtimeStatus, 'swarmcraft_integration');
  $('fabricApiState').textContent = fabricApi ? `${fabricApi.state}${fabricApi.version ? ` · ${fabricApi.version}` : ''}` : 'Not reported';
  $('swarmcraftModState').textContent = swarmcraft ? `${swarmcraft.state}${swarmcraft.version ? ` · ${swarmcraft.version}` : ''}` : 'Not reported';

  const list = $('serverModsList');
  list.replaceChildren();
  const required = Array.isArray(modsStatus?.required)
    ? modsStatus.required.filter((item) => item.component_kind !== 'managed_runtime')
    : [];
  const installed = Array.isArray(modsStatus?.installed)
    ? modsStatus.installed.filter((item) => item.component_kind !== 'managed_runtime')
    : [];
  const issues = Array.isArray(modsStatus?.issues) ? modsStatus.issues : [];

  if (!required.length) {
    const row = document.createElement('div');
    row.className = 'detail-row';
    row.textContent = 'No third-party server mods are required by this world.';
    list.append(row);
  } else {
    for (const requirement of required) {
      const row = document.createElement('div');
      row.className = 'detail-row';
      const label = document.createElement('span');
      label.textContent = `${requirement.mod_id} ${requirement.version}`;
      const problem = issues.find((issue) => issue.mod_id === requirement.mod_id);
      const status = document.createElement('strong');
      status.textContent = problem ? problem.message : 'Verified';
      row.append(label, status);
      list.append(row);
    }
  }

  for (const item of installed) {
    const row = document.createElement('div');
    row.className = 'detail-row';
    const label = document.createElement('span');
    label.textContent = `${item.mod_id} ${item.version} · local`;
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'text-button';
    remove.textContent = 'Remove local copy';
    remove.addEventListener('click', async () => {
      try {
        await run('Removing local server mod…', () => backend.mods.removeLocal(worldId(), item.mod_id), {
          successMessage: `Removed the local ${item.mod_id} artifact. The signed world modpack was not changed.`,
        });
        refreshWorldMods(selectedWorld());
      } catch (_) {}
    });
    row.append(label, remove);
    list.append(row);
  }

  const ready = modsStatus?.ready === true;
  $('modsBadge').textContent = ready ? 'Verified' : 'Needs attention';
  $('modsBadge').className = `status-badge ${ready ? 'safe' : 'warning'}`;
  $('modsSummary').textContent = ready
    ? 'All required third-party server mods on this computer match the signed world profile.'
    : 'One or more required server mods are missing, incompatible, duplicated, or corrupt.';
  $('modsIssues').hidden = !issues.length;
  $('modsIssues').textContent = issues.map((issue) => issue.message).join(' · ');
}

async function refreshWorldMods(world) {
  if (!world) return;
  const requestedWorldId = world.id;
  const requestGeneration = ++modsRequestGeneration;
  const [runtimeResult, modsResult] = await Promise.allSettled([
    backend.runtime.status(requestedWorldId),
    backend.mods.status(requestedWorldId),
  ]);
  if (selectedWorldId !== requestedWorldId || requestGeneration !== modsRequestGeneration) return;
  const runtimeStatus = runtimeResult.status === 'fulfilled' ? runtimeResult.value : null;
  const modsStatus = modsResult.status === 'fulfilled'
    ? modsResult.value
    : { ready: false, required: [], installed: [], issues: [{ message: String(modsResult.reason) }] };
  renderWorldMods(runtimeStatus, modsStatus);
}

async function supplyRequiredMod() {
  const jarPath = $('modJarPath').value.trim();
  if (!jarPath) {
    showInline('worldNotice', 'Choose or enter the local path to the required Fabric mod JAR first.', 'warning');
    $('modJarPath').focus();
    return;
  }
  try {
    await run('Supplying required server mod…', () => backend.mods.supplyRequiredJar(worldId(), jarPath), {
      successMessage: 'Required server mod verified and copied into this world’s local runtime profile.',
    });
    $('modJarPath').value = '';
    await refreshWorldMods(selectedWorld());
    await refreshHostReadiness(selectedWorld());
  } catch (_) {}
}

async function openModsFolder() {
  try {
    await run('Opening mods folder…', () => backend.mods.openFolder(worldId()), { logResult: false });
  } catch (_) {}
}

function queueGameGatewaySelection(world) {
  if (!world?.id) return gatewaySelectionQueue;
  // A daemon restart invalidates the old process-owned TCP gateway. Reapply
  // only the player's CURRENT selection using the same serialized queue as
  // world switching; this action never represents game-route verification.
  gatewaySelectionQueue = gatewaySelectionQueue.catch(() => {}).then(async () => {
    if (selectedWorldId !== world.id) return;
    await backend.gameRouteSelect(world.id);
    if (selectedWorldId === world.id) void refreshGameRoute(world);
  }).catch(() => {
    if (selectedWorldId === world.id) invalidateGameRoute();
  });
  return gatewaySelectionQueue;
}

function selectWorld(world, { focusDetail = false } = {}) {
  hostReadinessRequestGeneration += 1;
  if (window.__swarmcraftSvelteReadinessOwned !== true) renderHostReadiness(null);
  gameRouteRequestGeneration += 1;
  gameRouteState = UNVERIFIED_GAME_ROUTE;
  if (selectedWorldId !== world.id) localCheckpointVerification = null;
  selectedWorldId = world.id;
  queueGameGatewaySelection(world);
  $('world').value = world.id;
  // The candidate now renders read-only selected-world metadata directly
  // from Svelte's current native Rust world-list response. Preserve legacy
  // updates in the shipping frontend and keep all action gates in Rust.
  if (window.__swarmcraftSvelteWorldsOwned !== true) {
    $('selectedName').textContent = world.name;
    $('selectedWorldId').textContent = world.id;
    $('selectedAuthority').textContent = world.status.Authority || 'Not reported';
    $('selectedMembers').textContent = world.status['Authorized peers'] || 'Unknown';
    $('selectedMinecraft').textContent = world.status.Minecraft || 'Unknown';
    $('selectedCheckpoint').textContent = world.status['Latest snapshot'] || (world.snapshots ? `${world.snapshots} stored` : 'None');
    $('selectedCompatibility').textContent = world.compatibility['Authority eligibility'] || 'Not synchronized';
    for (const field of document.querySelectorAll('[data-local-role-field]')) {
      field.textContent = world.status[field.dataset.localRoleField] || 'Not verified';
    }

    const safety = world.status.Safety || 'Unknown';
    const kind = safetyKind(safety);
    $('selectedSafety').textContent = safety;
    $('selectedSafety').className = `status-badge ${kind}`;
    $('safetyPanel').className = `safety-panel ${kind}`;
    $('selectedSummary').textContent = safetySummary(world);
  }

  $('noSelection').hidden = true;
  $('selectionContent').hidden = false;
  membershipRequestGeneration += 1;
  renderMembershipNotice(world);
  showInline('worldNotice', '');
  showInline('runtimeNotice', '');
  if (window.__swarmcraftSvelteWorldsOwned === true) {
    window.__swarmcraftSvelteWorlds?.setSelected(world.id);
  } else {
    for (const row of document.querySelectorAll('.world-row')) {
      const selected = row.dataset.worldId === world.id;
      row.classList.toggle('is-selected', selected);
      row.setAttribute('aria-pressed', selected ? 'true' : 'false');
    }
  }
  renderConnectivity();
  updatePlayState(world);
  updateDiagnosticsContext();
  renderMigration(null);
  renderGameRoute();
  refreshMigrationState(world);
  refreshGameRoute(world);
  refreshHostReadiness(world);
  refreshWorldMods(world);
  if (focusDetail) $('selectedName').scrollIntoView({ block: 'nearest' });
}

function renderWorld(world) {
  const row = document.createElement('button');
  row.type = 'button';
  row.className = 'world-row';
  row.dataset.worldId = world.id;
  row.setAttribute('aria-pressed', world.id === selectedWorldId ? 'true' : 'false');
  row.addEventListener('click', () => selectWorld(world));

  const title = document.createElement('span');
  title.className = 'world-row-title';
  const name = document.createElement('strong');
  name.textContent = world.name;
  const id = document.createElement('span');
  id.className = 'world-row-id';
  id.textContent = world.id;
  title.append(name, id);

  const safety = document.createElement('span');
  safety.className = `status-badge ${safetyKind(world.status.Safety)}`;
  safety.textContent = world.status.Safety || 'Unknown';

  const meta = document.createElement('span');
  meta.className = 'world-row-meta';
  const minecraft = document.createElement('span');
  minecraft.textContent = `Minecraft ${world.status.Minecraft || 'unknown'}`;
  const host = document.createElement('span');
  host.textContent = world.status.Authority ? `Host ${world.status.Authority}` : 'Host not reported';
  const connectivity = document.createElement('span');
  connectivity.className = 'world-row-connectivity';
  connectivity.textContent = structuredConnectivity.label;
  meta.append(minecraft, host, connectivity);

  row.append(title, safety, meta);
  if (world.id === selectedWorldId) row.classList.add('is-selected');
  return row;
}

function renderEmptyWorlds() {
  const container = $('worldCards');
  container.replaceChildren();
  const empty = document.createElement('div');
  empty.className = 'empty-state';
  const mark = document.createElement('div');
  mark.className = 'empty-mark';
  mark.textContent = '▦';
  mark.setAttribute('aria-hidden', 'true');
  const title = document.createElement('h2');
  title.textContent = 'No worlds yet';
  const body = document.createElement('p');
  body.textContent = 'Create a world, import an existing Minecraft save, or join with a signed invite.';
  const actions = document.createElement('div');
  actions.className = 'compact-actions';
  const create = document.createElement('button');
  create.type = 'button';
  create.className = 'button button-primary';
  create.textContent = 'Create world';
  create.addEventListener('click', () => showView('create'));
  const importExisting = document.createElement('button');
  importExisting.type = 'button';
  importExisting.className = 'button button-secondary';
  importExisting.textContent = 'Import existing world';
  importExisting.addEventListener('click', () => showView('import'));
  const join = document.createElement('button');
  join.type = 'button';
  join.className = 'button button-secondary';
  join.textContent = 'Join with invite';
  join.addEventListener('click', () => showView('join'));
  actions.append(create, importExisting, join);
  empty.append(mark, title, body, actions);
  container.append(empty);
  container.setAttribute('aria-busy', 'false');
  $('worldCount').textContent = 'No local worlds';
  clearSelection();
}

async function refreshWorlds() {
  lastWorldError = null;
  if (window.__swarmcraftSvelteWorldsOwned === true) {
    // The installed candidate reads the exact existing Rust list/status/
    // compatibility commands through Svelte. Never render a second legacy
    // DOM list, double-fetch, or infer membership from cached display text.
    setStatus('Reading worlds…');
    try {
      const owner = window.__swarmcraftSvelteWorlds;
      if (!owner) throw new Error('Svelte world controller not mounted');
      const entries = await owner.refresh();
      if (!Array.isArray(entries)) throw new Error('Native world list was not verified');
      const preferred = selectedWorldId;
      worldCache.clear();
      for (const entry of entries) worldCache.set(entry.id, entry);
      const next = worldCache.get(preferred) || worldCache.values().next().value;
      if (next) selectWorld(next);
      else clearSelection();
      // Updating connectivity is independent of canonical world-list reads;
      // don't hold startup indefinitely behind a network diagnostic.
      void refreshConnectivityDiagnostics({ logFailure: false });
      if (!startupInProgress) setStatus('Ready', 'safe');
    } catch {
      lastWorldError = new Error('Native world list unavailable');
      worldCache.clear();
      clearSelection();
      setStatus('Worlds unavailable', 'danger');
    }
    return;
  }
  $('worldCards').setAttribute('aria-busy', 'true');
  showInline('worldLoadError', '');
  setStatus('Reading worlds…');
  let raw;
  try {
    raw = await run('Reading worlds…', () => backend.listWorlds(), { quiet: true });
  } catch (error) {
    lastWorldError = error;
    renderEmptyWorlds();
    $('worldCount').textContent = 'Could not read local worlds';
    showInline('worldLoadError', `Worlds could not be loaded. ${String(error)}`, 'danger');
    setOutput(String(error), 'World refresh failed');
    setStatus('Worlds unavailable', 'danger');
    return;
  }

  await refreshConnectivityDiagnostics({ logFailure: currentView === 'diagnostics' });
  const worlds = parseWorldList(raw);
  const previousSelection = selectedWorldId;
  const container = $('worldCards');
  container.replaceChildren();
  worldCache.clear();
  if (!worlds.length) {
    renderEmptyWorlds();
    if (!startupInProgress) setStatus('Ready', 'safe');
    return;
  }

  $('worldCount').textContent = `${worlds.length} world${worlds.length === 1 ? '' : 's'}`;
  for (const world of worlds) {
    let statusText = '';
    let compatibilityText = '';
    try {
      [statusText, compatibilityText] = await Promise.all([
        backend.worldStatus(world.id),
        backend.worldCompatibility(world.id),
      ]);
    } catch (_) {
      // A partial local copy still belongs in the launcher. Its unavailable fields remain explicit.
    }
    const model = { ...world, status: parseLines(statusText), compatibility: parseLines(compatibilityText) };
    worldCache.set(world.id, model);
    container.append(renderWorld(model));
  }
  container.setAttribute('aria-busy', 'false');
  const next = worldCache.get(previousSelection) || worldCache.values().next().value;
  if (next) selectWorld(next);
  else clearSelection();
  if (!startupInProgress) setStatus('Ready', 'safe');
}

async function ensureNetworkingService({ quiet = false } = {}) {
  try {
    const pid = await backend.ensureDaemonRunning($('daemonListen').value.trim());
    setServiceWarning('');
    if (!quiet) setOutput(`Networking daemon is running. PID ${pid}.`, 'Networking');
    return pid;
  } catch (error) {
    const message = `Automatic networking service could not start. Local worlds can still be used, but replication and automatic migration supervision may be unavailable. ${String(error)}`;
    setServiceWarning(message);
    if (!quiet) {
      setOutput(String(error), 'Networking start failed');
      setStatus('Networking needs attention', 'warning');
      try { error.swarmcraftActivityLogged = true; } catch (_) {}
      throw error;
    }
    setOutput(String(error), 'Automatic networking start failed');
    return null;
  }
}

// Svelte owns the identity presentation only when its candidate shell is
// mounted. The shipping legacy HTML retains the original direct-DOM path.
function publishIdentity(value, source = 'identity') {
  if (window.__swarmcraftSvelteIdentity?.publish(String(value), source) === true) return;
  $('identity').textContent = String(value);
}

async function initialize() {
  const result = await run('Setting up this device…', () => backend.initializeNode());
  publishIdentity(result, 'setup');
  setNodeState('Ready', 'safe');
  await ensureNetworkingService({ quiet: true });
  await refreshWorlds();
}

// Bounded UI waits are intentionally scoped to startup. A timeout is not a
// backend success: the launcher stays failed until the operation is retried.
async function startupStep(label, action, timeoutMs) {
  window.__swarmcraftStartup?.step(label);
  let timeout;
  try {
    return await Promise.race([
      Promise.resolve().then(action),
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs / 1000}s. Restart SwarmCraft if it continues.`)), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timeout);
  }
}

async function startup() {
  if (startupInProgress) return;
  startupInProgress = true;
  window.__swarmcraftStartup?.begin();
  setStatus('Starting SwarmCraft…');
  setNodeState('Checking…');
  try {
    const result = await startupStep('Device setup', () => backend.initializeNode(), 30000);
    publishIdentity(result, 'setup');
    setNodeState('Ready', 'safe');

    // Local world creation is useful without a network, but never call such a
    // partially initialized session fully ready.
    const daemonPid = await startupStep('Networking service', () => ensureNetworkingService({ quiet: true }), 10000)
      .catch((error) => {
        setServiceWarning('Automatic networking service could not start. Local worlds remain available; try restarting the networking service in Diagnostics.');
        setOutput('Networking service startup timed out.', 'Networking');
        return null;
      });

    const identity = await startupStep('Device identity', () => showIdentity({ quiet: true }), 15000);
    if (!identity) throw new Error('Device identity could not be verified. Open Diagnostics and use Set up device.');

    await startupStep('World list', () => refreshWorlds(), 45000);
    if (lastWorldError) throw new Error('The local world list could not be read. Check the SwarmCraft CLI sidecar and application data permissions.');

    refreshVisibleMigration();
    refreshVisibleHostReadiness();
    if (daemonPid === null) {
      setStatus('Networking needs attention', 'warning');
      window.__swarmcraftStartup?.ready({ degraded: true });
    } else {
      setStatus('Ready', 'safe');
      window.__swarmcraftStartup?.ready();
    }
  } finally {
    startupInProgress = false;
  }
}

async function showIdentity({ quiet = false } = {}) {
  try {
    const result = await run('Reading device identity…', () => backend.nodeIdentity(), { quiet });
    publishIdentity(result);
    setNodeState('Ready', 'safe');
    return result;
  } catch (error) {
    publishIdentity(quiet ? 'Device is not initialized.' : String(error), 'unavailable');
    setNodeState(invoke ? 'Not set up' : 'Desktop bridge unavailable', invoke ? 'warning' : 'danger');
    if (!quiet) throw error;
    return null;
  }
}

function validateCreate() {
  const required = [
    ['createName', 'World name'],
    ['createMinecraft', 'Minecraft version'],
    ['createLoader', 'Fabric loader version'],
    ['createCompatibility', 'Compatibility profile'],
  ];
  for (const [id, label] of required) {
    if (!$(id).value.trim()) {
      showInline('createError', `${label} is required.`, 'danger');
      $(id).focus();
      return false;
    }
  }
  showInline('createError', '');
  return true;
}

async function createWorld() {
  if (!validateCreate()) return;
  let result;
  try {
    result = await run('Creating world…', () => backend.createWorld({
      name: $('createName').value.trim(),
      minecraft: $('createMinecraft').value.trim(),
      fabricLoader: $('createLoader').value.trim(),
      compatibility: $('createCompatibility').value.trim(),
      visibility: $('createVisibility').value,
    }));
  } catch (error) {
    showInline('createError', `Could not create world: ${String(error)}`, 'danger');
    return;
  }
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) selectedWorldId = match[1];
  $('createForm').reset();
  $('createMinecraft').value = '26.1.2';
  $('createLoader').value = '0.19.3';
  $('createCompatibility').value = 'vanilla-fabric';
  await refreshWorlds();
  showView('worlds');
}

function importRequestFromForm() {
  return createImportRequest({
    source: $('importSource').value,
    name: $('importName').value,
    minecraft: $('importMinecraft').value,
    fabricLoader: $('importLoader').value,
    visibility: $('importVisibility').value,
    serverMods: $('importServerMods').value,
    noServerMods: $('importNoServerMods').checked,
  });
}

async function importWorld() {
  let request;
  try {
    request = importRequestFromForm();
    request.expectedCompatibility = importReview.take(request);
    showInline('importError', '');
  } catch (error) {
    showInline('importError', String(error.message || error), 'danger');
    if (error.field) $(error.field)?.focus();
    return;
  }

  let result;
  try {
    result = await run('Importing existing world…', () => backend.importWorld(request), {
      logResult: false,
      successMessage: 'Existing world imported into canonical SwarmCraft storage.',
    });
  } catch (error) {
    showInline('importError', 'Could not import world. Verify the source and refresh Worlds before checking the import again.', 'danger');
    return;
  }

  let importedWorldId;
  try {
    importedWorldId = parseImportResult(result).worldId;
  } catch (error) {
    showInline('importError', `Import completed but its result could not be read: ${String(error)}`, 'danger');
    return;
  }
  selectedWorldId = importedWorldId;
  $('importForm').reset();
  $('importMinecraft').value = '26.1.2';
  $('importLoader').value = '0.19.3';
  await refreshWorlds();
  showView('worlds');
  showInline(
    'worldNotice',
    'Existing world imported. Minecraft EULA acceptance and this device’s runtime setup were not changed.',
    'safe',
  );
}

async function joinWorld() {
  const invite = $('joinInvite').value.trim();
  if (!invite) {
    showInline('joinError', 'Paste a signed invite first.', 'danger');
    $('joinInvite').focus();
    return;
  }
  showInline('joinError', '');
  let result;
  try {
    result = await run('Joining world…', () => backend.joinWorld(invite, $('joinHostCandidate').checked));
  } catch (error) {
    showInline('joinError', `Could not join world: ${String(error)}`, 'danger');
    return;
  }
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) selectedWorldId = match[1];
  $('joinInvite').value = '';
  $('joinHostCandidate').checked = false;
  await refreshWorlds();
  showView('worlds');
}

function joinWorldId() {
  const world = $('joinWorldId').value.trim();
  if (!world) {
    showInline('joinWorldIdNotice', 'Enter a World ID first.', 'danger');
    $('joinWorldId').focus();
    return;
  }
  const message = `Discovery can look for ${world}, but it does not grant membership. A private world still needs a signed invite.`;
  showInline('joinWorldIdNotice', message);
  setOutput(message, 'World discovery');
}

function renderMembershipNotice(world) {
  const message = pendingMembershipMessage(world?.status?.Membership);
  showInline('membershipNotice', message, 'warning');
  const refresh = $('refreshMembership');
  if (refresh) refresh.hidden = !message;
}

async function refreshMembershipStatus() {
  const world = selectedWorld();
  if (!world) return;
  const generation = ++membershipRequestGeneration;
  try {
    const raw = await backend.worldStatus(world.id);
    if (generation !== membershipRequestGeneration || selectedWorldId !== world.id) return;
    const status = parseLines(raw);
    if (status['World ID'] !== world.id) throw new Error('Wrong world status');
    // Refresh the complete world model if membership has advanced. A
    // successful status read alone does not establish a playable route.
    if (!pendingMembershipMessage(status.Membership)) {
      await refreshWorlds();
      return;
    }
    const current = selectedWorld();
    if (current) {
      const updated = { ...current, status };
      worldCache.set(world.id, updated);
      renderMembershipNotice(updated);
      updatePlayState(updated);
    }
  } catch (_) {
    if (generation === membershipRequestGeneration && selectedWorldId === world.id) {
      showInline('membershipNotice', 'Could not confirm membership status. Refresh the world and check connectivity.', 'warning');
    }
  }
}

function clearInviteInspection() {
  inviteManagementSelection = null;
  $('inviteInspection').textContent = '';
  $('inviteInspection').hidden = true;
  $('revokeInviteConfirm').checked = false;
  $('revokeInviteConfirmWrap').hidden = true;
  $('revokeInvite').disabled = true;
}

function setInviteManagementBusy(busy) {
  inviteManagementBusy = busy;
  for (const id of ['manageInviteToken', 'inspectInvite', 'closeInviteManagement', 'revokeInviteConfirm']) $(id).disabled = busy;
  $('revokeInvite').disabled = busy || !inviteManagementSelection?.receipt.canRequestRevocation || !$('revokeInviteConfirm').checked;
}

function openInviteManagement() {
  const world = selectedWorld();
  if (!world || inviteManagementBusy || $('inviteManagementDialog').open) return;
  inviteManagementWorld = world.id;
  // Begin with an empty token; the backend independently verifies the
  // signature and selected world, and no bearer is logged.
  $('manageInviteToken').value = '';
  clearInviteInspection();
  showInline('inviteManagementNotice', '');
  setInviteManagementBusy(false);
  $('inviteManagementDialog').showModal();
}

async function inspectManagedInvite() {
  if (inviteManagementBusy || !$('inviteManagementDialog').open) return;
  clearInviteInspection();
  showInline('inviteManagementNotice', '');
  try {
    if (selectedWorldId !== inviteManagementWorld) throw new Error('Selection changed');
    const token = validateInviteToken($('manageInviteToken').value);
    const world = inviteManagementWorld;
    setInviteManagementBusy(true);
    const raw = await backend.inspectInvite(token);
    if (! $('inviteManagementDialog').open || selectedWorldId !== world || world !== inviteManagementWorld
        || $('manageInviteToken').value.trim() !== token) return;
    const receipt = inspectInviteReceipt(world, raw);
    inviteManagementSelection = Object.freeze({ token, receipt });
    const states = {
      lifetime_valid: 'Lifetime valid; admission still requires current authority and membership approval.',
      expired: 'Expired or invalid lifetime.',
      revoked: 'Revoked by this issuer.',
    };
    $('inviteInspection').textContent = [
      'Invite ID: ' + receipt.id,
      'World: ' + receipt.world,
      'Issuer: ' + receipt.issuer,
      'Expires: ' + new Date(receipt.expires).toLocaleString(),
      'Reuse: reusable bearer grant',
      'Status: ' + states[receipt.status],
      receipt.locallyIssued ? 'Issued by this device. Revocation checks current authority and any prepared membership decision.'
        : 'Issued by another device. Only the current issuing identity can revoke it.',
    ].join('\n');
    $('inviteInspection').hidden = false;
    $('revokeInviteConfirmWrap').hidden = !receipt.canRequestRevocation;
  } catch (_) {
    showInline('inviteManagementNotice', 'Could not verify an invitation for this selected world. Check the token and try again.', 'warning');
  } finally {
    setInviteManagementBusy(false);
  }
}

async function revokeManagedInvite() {
  if (inviteManagementBusy || !$('inviteManagementDialog').open) return;
  try {
    if (selectedWorldId !== inviteManagementWorld || !inviteManagementSelection) throw new Error('Selection changed');
    const selection = inviteManagementSelection;
    const payload = revocationRequest(selection, $('manageInviteToken').value, $('revokeInviteConfirm').checked);
    setInviteManagementBusy(true);
    const raw = await backend.revokeInvite(payload);
    const newlyRevoked = revokeInviteReceipt(selection.receipt, raw);
    clearInviteInspection();
    $('manageInviteToken').value = '';
    showInline('inviteManagementNotice', newlyRevoked
      ? 'Invitation revoked. Existing committed members retain membership.'
      : 'This invitation was already revoked. Existing committed members retain membership.', 'safe');
    setOutput('Invitation revocation confirmed. Existing membership is unchanged.', 'Invite management');
  } catch (error) {
    clearInviteInspection();
    const prepared = String(error).includes('membership decision is already prepared');
    showInline('inviteManagementNotice', prepared
      ? 'A membership decision is already prepared. Resolve it before revoking this grant.'
      : 'Revocation was not confirmed. Inspect the invitation again before retrying.', 'warning');
  } finally {
    setInviteManagementBusy(false);
  }
}

function openInviteDialog() {
  const world = selectedWorld();
  if (!world) throw new Error('Choose a world first.');
  $('inviteWorldName').textContent = world.name;
  if (window.__swarmcraftSvelteInviteOwned) window.__swarmcraftResetSvelteInvite?.();
  $('inviteResult').value = '';
  $('inviteResultWrap').hidden = true;
  showInline('inviteError', '');
  $('inviteDialog').showModal();
  $('inviteMinutes').focus();
}

async function createInvite() {
  const bootstrapAddrs = $('bootstrapAddrs').value.split('\n').map((value) => value.trim()).filter(Boolean);
  let result;
  try {
    result = await run('Creating invite…', () => backend.createInvite({
      world: worldId(),
      expiresMinutes: Number($('inviteMinutes').value || 60),
      bootstrapAddrs,
    }), { logResult: false });
  } catch (error) {
    showInline('inviteError', `Could not create invite: ${String(error)}`, 'danger');
    return;
  }
  $('inviteResult').value = String(result || '');
  $('inviteResultWrap').hidden = false;
  $('inviteResult').focus();
  setOutput('Signed invite created.', 'Invite');
}

async function copyText(text, label) {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
  } catch (_) {
    const temp = document.createElement('textarea');
    temp.value = text;
    temp.setAttribute('readonly', '');
    temp.style.position = 'fixed';
    temp.style.opacity = '0';
    document.body.append(temp);
    temp.select();
    document.execCommand('copy');
    temp.remove();
  }
  setStatus(`${label} copied`, 'safe');
}

async function worldStatus() {
  await run('Reading world status…', () => backend.worldStatus(worldId()));
  showView('activity');
}

async function worldCompatibility() {
  await run('Checking compatibility…', () => backend.worldCompatibility(worldId()));
  showView('activity');
}

async function worldConflicts() {
  showView('diagnostics');
  await openRecoveryCopy();
}

async function worldPeers() {
  await run('Reading world members…', () => backend.worldPeers(worldId()));
  showView('activity');
}

async function verifyWorld() {
  const requestedWorldId = worldId();
  localCheckpointVerification = null;
  renderGameRoute();
  const result = await run('Verifying replica…', () => backend.verifyWorld(requestedWorldId));
  // Successful CLI exit verifies local storage integrity, not a signed live
  // canonical authority, game route, or permission to start Minecraft.
  if (requestedWorldId === selectedWorldId) {
    const hashes = [...String(result ?? '').matchAll(/^Verified checkpoint: ([a-f\d]{64})\s*$/gim)];
    const manifestHash = hashes.at(-1)?.[1];
    if (manifestHash) localCheckpointVerification = { worldId: requestedWorldId, manifestHash };
  }
  renderGameRoute();
  showInline('worldNotice', 'Replica verification completed. Detailed output is available in Activity.', 'safe');
}

async function setSeeding(enabled) {
  await run(`${enabled ? 'Enabling' : 'Disabling'} background replica…`, () => backend.setBackgroundSeeding(worldId(), enabled), {
    successMessage: enabled ? 'This device will keep serving a background replica.' : 'Background replica serving was disabled.',
  });
  showInline('worldNotice', enabled ? 'Background replica enabled.' : 'Background replica disabled.', enabled ? 'safe' : 'warning');
}

function openSleepDialog() {
  if (!selectedWorld()) throw new Error('Choose a world first.');
  $('sleepDialog').showModal();
}

async function sleepWorld() {
  invalidateGameRoute();
  invalidateHostReadiness();
  await run('Stopping local Minecraft runtime…', () => backend.stopHost(worldId()), {
    successMessage: 'World runtime stopped gracefully. Replica storage can continue separately.',
  });
  $('sleepDialog').close();
  showInline('worldNotice', 'World runtime stopped gracefully.', 'safe');
}

function openLeaveDialog() {
  const world = selectedWorld();
  if (!world) throw new Error('Choose a world first.');
  $('leaveDialogText').textContent = `This stages a signed leave request for ${world.name}. Local membership changes after the request is accepted.`;
  if (window.__swarmcraftSvelteLeaveOwned) window.__swarmcraftResetSvelteLeave?.();
  $('leaveDialog').showModal();
}

async function performLeaveWorld() {
  await run('Requesting membership leave…', () => backend.leaveWorld(worldId()));
  $('leaveDialog').close();
  selectedWorldId = '';
  migrationRequestGeneration += 1;
  gameRouteRequestGeneration += 1;
  gameRouteState = UNVERIFIED_GAME_ROUTE;
  localCheckpointVerification = null;
  $('world').value = '';
  await refreshWorlds();
}

function runtimeValidationIssue() {
  const world = selectedWorld();
  if (!world) return [null, 'Choose a world first.'];
  const eligibility = hostingEligibility(world);
  if (!eligibility.enabled) return [null, eligibility.reason];
  if (!$('serverJar').value.trim()) return ['serverJar', 'Set the Fabric server jar in Diagnostics before Play can start the current runtime.'];
  if (!$('modJar').value.trim()) return ['modJar', 'Set the SwarmCraft Fabric mod jar in Diagnostics before Play can start the current runtime.'];
  if (!$('eula').checked) return ['eula', 'Accept the Minecraft server EULA in Diagnostics before Play can start the current runtime.'];
  return null;
}

function showRuntimeValidation(issue) {
  const [fieldId, message] = issue;
  showView('diagnostics', { focus: false });
  showInline('runtimeNotice', message, 'warning');
  $('runtimeSection').scrollIntoView({ block: 'start' });
  if (fieldId) $(fieldId).focus();
}

async function hostWorld() {
  invalidateGameRoute();
  invalidateHostReadiness();
  const issue = runtimeValidationIssue();
  if (issue) {
    showRuntimeValidation(issue);
    return;
  }
  const requestedWorld = worldId();
  showInline('runtimeNotice', '');
  try {
    const pid = await run('Starting Minecraft…', () => backend.hostWorld({
      world: requestedWorld,
      java: $('java').value.trim(),
      serverJar: $('serverJar').value.trim(),
      modJar: $('modJar').value.trim(),
      acceptEula: $('eula').checked,
    }), { successMessage: (pid) => pid == null
      ? 'Fresh signed Minecraft route verified. Desktop did not start a separate Java server or confirm a player login.'
      : `Minecraft authority runtime started. PID ${pid}.` });
    // An in-flight launch may finish after the player selects a different
    // world. Never repaint the new world's panel with the old route/PID.
    if (selectedWorldId !== requestedWorld) return;
    showView('worlds');
    showInline('worldNotice', pid == null
      ? 'Authenticated Minecraft route verified. Use Join current host to find the localhost address and reconnect in Minecraft Java Multiplayer.'
      : 'Minecraft runtime started for this solo world. Runtime setup is persisted.', 'safe');
  } catch (error) {
    showInline('runtimeNotice', `Could not start Minecraft: ${String(error)}`, 'danger');
    throw error;
  }
}

async function stopHost() {
  invalidateGameRoute();
  invalidateHostReadiness();
  await run('Stopping Minecraft runtime…', () => backend.stopHost(worldId()), { successMessage: 'World stopped safely. Latest world state is checkpointed and sleeping.' });
}

async function startDaemon() {
  await run('Starting networking…', () => backend.ensureDaemonRunning($('daemonListen').value.trim()), {
    successMessage: (pid) => `Networking daemon is running. PID ${pid}.`,
  });
  setServiceWarning('');
  await queueGameGatewaySelection(selectedWorld());
  await refreshConnectivityDiagnostics({ logFailure: true });
}

async function stopDaemon() {
  invalidateGameRoute();
  invalidateHostReadiness();
  await run('Stopping networking…', () => backend.stopDaemon(), { successMessage: 'Desktop-owned networking daemon stopped.' });
  setServiceWarning('Networking is stopped for this Desktop session. Start networking to restore replication and automatic migration supervision.');
}

async function exportWorld() {
  const destination = $('destination').value.trim();
  if (!destination) {
    showInline('runtimeNotice', 'Choose an export destination first.', 'warning');
    $('destination').focus();
    return;
  }
  await run('Exporting verified world…', () => backend.exportWorld(worldId(), destination));
}

async function recoverWorld() {
  const destination = $('destination').value.trim();
  if (!destination) {
    setStatus('Recovery destination required', 'warning');
    $('destination').focus();
    return;
  }
  await run('Recovering verified snapshot…', () => backend.recoverWorld(worldId(), Number($('snapshotNumber').value || 1), destination));
}

function updateRecoveryChoiceControls() {
  const target = recoveryCopyTarget;
  const acknowledged = !target?.conflictDigest || $('recoveryConflictAck').checked;
  const ready = Boolean(target && acknowledged && !recoveryCopyPending);
  const hasDestination = Boolean($('recoveryExportDestination').value.trim());
  $('recoveryChoice').disabled = recoveryCopyPending || !recoveryReview?.choices.length;
  $('recoveryConflictAck').disabled = recoveryCopyPending;
  $('recoveryExportDestination').disabled = recoveryCopyPending;
  $('confirmRecoveryCopy').hidden = target?.kind !== 'checkpoint';
  $('confirmRecoveryCopy').disabled = !ready || target?.kind !== 'checkpoint';
  $('exportRecoveryChoice').hidden = !target || (target.kind === 'branch' && target.localHistory !== 'matching_signed_manifests');
  $('exportRecoveryChoice').disabled = !ready || !hasDestination;
  $('exportRecoveryEvidence').hidden = target?.kind !== 'branch';
  $('exportRecoveryEvidence').disabled = !ready || !hasDestination;
}

function selectRecoveryChoice() {
  recoveryCopyTarget = recoveryReview?.choices.find(choice => choice.value === $('recoveryChoice').value) ?? null;
  const target = recoveryCopyTarget;
  $('recoveryConflictAck').checked = false;
  $('recoveryConflictAckWrap').hidden = !target?.conflictDigest;
  showInline('recoveryCopyResult', '');
  if (!target) {
    $('recoveryCopySource').textContent = 'No locally verified recovery choice is available.';
  } else if (target.kind === 'checkpoint') {
    $('recoveryCopySource').textContent = [
      'Snapshot: ' + target.snapshot + (target.latest ? ' (local head)' : ' (earlier checkpoint)'),
      'Manifest: ' + target.checkpoint, 'Original: ' + target.world, 'Epoch / sequence: ' + target.epoch + ' / ' + target.sequence,
      'State: ' + target.stateRoot, 'Signer: ' + target.signer,
      'Signed history checked; ' + target.files + ' files will be verified before action.',
    ].join('\n');
  } else {
    $('recoveryCopySource').textContent = [
      target.localHistory === 'matching_signed_manifests' ? 'Matching signed local manifests' : 'Evidence only here',
      'Branch: ' + target.branch, 'Head: ' + target.checkpoint,
      'Original: ' + target.world, 'Base: ' + target.base,
      'Epoch / sequence: ' + target.epoch + ' / ' + target.sequence,
      'State: ' + target.stateRoot, 'Signer: ' + target.signer,
      target.localHistory === 'matching_signed_manifests'
        ? 'Matching signed local manifests. Head files must pass verification before export.'
        : 'Evidence only here. Export files from the branch holder; no remote data is assumed available.',
    ].join('\n');
  }
  updateRecoveryChoiceControls();
}

async function openRecoveryCopy() {
  if (recoveryCopyPending || $('recoveryCopyDialog').open) return;
  const requestedWorld = worldId();
  showInline('recoveryCopyNotice', 'Reading local signed history…');
  try {
    const raw = await backend.recoveryReview(requestedWorld);
    if (selectedWorldId !== requestedWorld || currentView !== 'diagnostics'
        || recoveryCopyPending || $('recoveryCopyDialog').open) return;
    recoveryReview = normalizeRecoveryReview(requestedWorld, raw);
    const options = recoveryReview.choices.map(choice => {
      const option = document.createElement('option');
      option.value = choice.value;
      option.textContent = choice.kind === 'checkpoint'
        ? 'Checkpoint #' + choice.snapshot + (choice.latest ? ' · local head' : ' · earlier')
        : 'Branch ' + choice.branch.slice(0, 12) + '… · ' + (choice.localHistory === 'matching_signed_manifests' ? 'local manifests' : 'evidence only');
      return option;
    });
    $('recoveryChoice').replaceChildren(...options);
    $('recoveryReviewBody').hidden = false;
    $('recoveryReviewBody').scrollTop = 0;
    $('recoveryCopyTitle').textContent = 'Review recovery choices';
    $('cancelRecoveryCopy').textContent = 'Cancel';
    $('recoveryExportDestination').value = '';
    $('recoveryConflictAck').checked = false;
    const count = recoveryReview.choices.filter(choice => choice.kind === 'branch').length;
    const notice = [count ? count + ' preserved signed branch record(s). Review each branch before choosing; existing hosting fences stay in place.' : '',
      recoveryReview.checkpointError ? 'Certified checkpoint review unavailable: ' + recoveryReview.checkpointError : ''].filter(Boolean).join(' ');
    showInline('recoveryChoiceNotice', notice, count || recoveryReview.checkpointError ? 'warning' : 'neutral');
    showInline('recoveryCopyNotice', '');
    selectRecoveryChoice();
    $('recoveryCopyDialog').showModal();
  } catch (error) {
    if (selectedWorldId === requestedWorld) showInline('recoveryCopyNotice', String(error), 'warning');
  }
}

async function performRecoveryChoice(action) {
  if (!recoveryCopyTarget || recoveryCopyPending || !$('recoveryCopyDialog').open) return;
  const target = recoveryCopyTarget;
  if (selectedWorldId !== target.world) {
    recoveryCopyTarget = null;
    updateRecoveryChoiceControls();
    showInline('recoveryCopyResult', 'The selected world changed. Close and review the intended world again.', 'warning');
    return;
  }
  if (target.conflictDigest && !$('recoveryConflictAck').checked) return;
  const destination = $('recoveryExportDestination').value.trim();
  if (action === 'copy' && target.kind !== 'checkpoint') return;
  if (action !== 'copy' && !destination) return;
  if (action === 'evidence' && target.kind !== 'branch') return;
  if (action === 'export' && target.kind === 'branch' && target.localHistory !== 'matching_signed_manifests') return;
  recoveryCopyPending = true;
  $('cancelRecoveryCopy').disabled = true;
  updateRecoveryChoiceControls();
  showInline('recoveryCopyResult', 'Verifying the exact selected history and data…');
  try {
    let message;
    if (action === 'copy') {
      const raw = await backend.recoverCopy(target.world, target.checkpoint, true, target.conflictDigest);
      const receipt = recoveryCopyReceipt(target, raw);
      message = receipt.name + ' created.\nOriginal: ' + receipt.source + '\nNew world: ' + receipt.world
        + '\nSelect the new world in Worlds; hosting setup is separate.';
      $('recoveryCopyTitle').textContent = 'Recovered world created';
    } else {
      const evidenceOnly = action === 'evidence';
      const raw = target.kind === 'branch'
        ? await backend.conflictExport(target.world, target.branch, destination, target.conflictDigest, evidenceOnly)
        : await backend.recoveryExport(target.world, target.checkpoint, destination, target.conflictDigest);
      const receipt = recoveryExportReceipt(target, raw, evidenceOnly);
      message = (receipt.files ? 'Verified head files and signed evidence exported.' : 'Signed evidence exported. No world files were exported.')
        + '\nDestination: ' + receipt.destination + '\nThe original world was not changed.';
      $('recoveryCopyTitle').textContent = 'Recovery export completed';
    }
    showInline('recoveryCopyResult', message, 'safe');
    showInline('recoveryCopyNotice', message, 'safe');
    setOutput(message, action === 'copy' ? 'Recovered copy' : 'Recovery export');
    if (action === 'copy') await refreshWorlds();
  } catch (error) {
    $('recoveryCopyTitle').textContent = 'Recovery outcome not confirmed';
    showInline('recoveryCopyResult', 'Could not confirm the operation. Inspect Worlds or the destination before retrying. ' + String(error), 'danger');
  } finally {
    recoveryCopyTarget = null;
    recoveryReview = null;
    recoveryCopyPending = false;
    $('recoveryReviewBody').hidden = true;
    updateRecoveryChoiceControls();
    $('cancelRecoveryCopy').disabled = false;
    $('cancelRecoveryCopy').focus();
    $('cancelRecoveryCopy').textContent = 'Done';
  }
}

async function confirmRecoveryCopy() { await performRecoveryChoice('copy'); }

async function transferHost() {
  invalidateGameRoute();
  invalidateHostReadiness();
  if (!backend.migration.capabilities.transfer) return;
  await run('Preparing host transfer…', () => backend.migration.transferAuthority(worldId()));
  await refreshMigrationState(selectedWorld());
}

async function wakeWorld() {
  invalidateGameRoute();
  invalidateHostReadiness();
  if (!backend.migration.capabilities.wake) return;
  await run('Waking world…', () => backend.migration.wakeWorld(worldId()));
  await refreshMigrationState(selectedWorld());
}

// The Svelte candidate owns Create World submissions. The installed legacy
// frontend has no ownership flag and continues using this handler as before.
// Completion-only callbacks are exposed exclusively in the mounted Svelte
// candidate. The shipping legacy webview does not receive extra globals.
if (window.__swarmcraftSvelteCreateOwned === true
    || window.__swarmcraftSvelteJoinOwned === true
    || window.__swarmcraftSvelteImportOwned === true
    || window.__swarmcraftSvelteLeaveOwned === true) {
window.__swarmcraftAfterSvelteLeave = async () => {
  invalidateGameRoute();
  invalidateHostReadiness();
  $('leaveDialog').close();
  selectedWorldId = '';
  migrationRequestGeneration += 1;
  $('world').value = '';
  await refreshWorlds();
  if (lastWorldError) throw new Error('Membership may have changed but Worlds could not be refreshed.');
};

window.__swarmcraftAfterSvelteImport = async (importedWorldId) => {
  const id = String(importedWorldId || '').trim();
  if (!/^scworld:[^\s]+$/.test(id)) throw new Error('Import did not return a canonical World ID.');
  selectedWorldId = id;
  await refreshWorlds();
  if (lastWorldError) throw new Error('Import completed but the updated world list could not be loaded.');
  showView('worlds');
  showInline(
    'worldNotice',
    'Existing world imported. Minecraft EULA acceptance and this device’s runtime setup were not changed.',
    'safe',
  );
};

window.__swarmcraftAfterSvelteJoin = async (joinedWorldId) => {
  const id = String(joinedWorldId || '').trim();
  if (id && !/^scworld:[^\s]+$/.test(id)) throw new Error('Join returned an invalid World ID.');
  if (id) selectedWorldId = id;
  await refreshWorlds();
  if (lastWorldError) throw new Error('Join completed but the updated world list could not be loaded.');
  showView('worlds');
};

window.__swarmcraftAfterSvelteCreate = async (createdWorldId) => {
  const id = String(createdWorldId || '').trim();
  if (!/^scworld:[^\s]+$/.test(id)) throw new Error('Creation did not return a canonical World ID.');
  selectedWorldId = id;
  await refreshWorlds();
  if (lastWorldError) {
    throw new Error('World was created but the updated world list could not be loaded.');
  }
  showView('worlds');
};
}

// Explicit Rust-backed daemon actions are Svelte-owned only in the candidate.
// Legacy desktop continues its original button handlers when that flag is absent.
if (window.__swarmcraftSvelteNetworkingOwned === true) {
  window.__swarmcraftAfterSvelteNetworkingChange = async (outcome) => {
    ++connectivityGeneration;
    if (outcome === 'stopped' || outcome === 'unknown') {
      invalidateGameRoute();
      invalidateHostReadiness();
      networkStoppedBySvelte = true;
      setServiceWarning(outcome === 'stopped'
        ? 'Networking is stopped for this Desktop session. Replication and automatic migration supervision may be unavailable.'
        : 'Networking outcome is unverified. Previous connectivity paths must not be treated as current.');
      structuredConnectivity = connectivityFromStatus({ state: 'nat_status_unknown' });
      renderConnectivity();
      for (const element of document.querySelectorAll('.world-row-connectivity')) {
        element.textContent = structuredConnectivity.label;
      }
      return;
    }
    if (outcome !== 'started') throw new Error('Unknown native networking outcome.');
    networkStoppedBySvelte = false;
    setServiceWarning('');
    await refreshConnectivityDiagnostics({ logFailure: false });
  };
}

// Candidate-only selection/refresh handoff. Rust data originates in Svelte;
// the rich selected-world panel remains legacy until its separate migration.
if (window.__swarmcraftSvelteWorldsOwned === true) {
  window.__swarmcraftRefreshWorldsFromSvelte = refreshWorlds;
  window.__swarmcraftSelectWorldFromSvelte = (id) => {
    const model = typeof id === 'string' ? worldCache.get(id) : null;
    if (!model) return false;
    selectWorld(model, { focusDetail: true });
    return true;
  };
}

function bindAction(id, handler, { submit = false } = {}) {
  const element = $(id);
  if (!element) return;
  element.addEventListener(submit ? 'submit' : 'click', async (event) => {
    if (submit) event.preventDefault();
    // Avoid issuing two native create_world commands when the Svelte handler
    // owns the form. The original production HTML still uses this listener.
    if (id === 'createForm' && window.__swarmcraftSvelteCreateOwned === true) return;
    if (id === 'joinForm' && window.__swarmcraftSvelteJoinOwned === true) return;
    if (id === 'importForm' && window.__swarmcraftSvelteImportOwned === true) return;
    if (id === 'createInvite' && window.__swarmcraftSvelteInviteOwned === true) return;
    if (id === 'joinWorldIdButton' && window.__swarmcraftSvelteDiscoveryOwned === true) return;
    if (id === 'confirmLeave' && window.__swarmcraftSvelteLeaveOwned === true) return;
    if (id === 'refresh' && window.__swarmcraftSvelteWorldsOwned === true) return;
    if (id === 'identityButton' && window.__swarmcraftSvelteIdentityOwned === true) return;
    if ((id === 'startDaemon' || id === 'stopDaemon')
        && window.__swarmcraftSvelteNetworkingOwned === true) return;
    if (element.dataset.busy === 'true') return;
    const busyControl = submit && event.submitter ? event.submitter : element;
    element.dataset.busy = 'true';
    busyControl.disabled = true;
    busyControl.setAttribute('aria-busy', 'true');
    try {
      await handler(event);
    } catch (error) {
      if (!error?.swarmcraftActivityLogged) setOutput(String(error), 'Action failed');
    } finally {
      element.dataset.busy = 'false';
      busyControl.disabled = false;
      busyControl.removeAttribute('aria-busy');
      updateWorldSpecificControls();
      if (id === 'importForm') importReview.sync();
    }
  });
}

const importReview = installImportReview($('importForm'), backend, importRequestFromForm,
  () => {
    if (window.__swarmcraftSvelteImportOwned === true) {
      $('importForm').dispatchEvent(new Event('importreviewstart'));
    } else showInline('importError', '');
  });
window.__swarmcraftTakeImportReview = request => importReview.take(request);
window.__swarmcraftRefreshImportReview = () => importReview.sync();

for (const nav of document.querySelectorAll('.nav-item[data-view]')) nav.addEventListener('click', () => showView(nav.dataset.view));
for (const back of document.querySelectorAll('[data-back-worlds]')) back.addEventListener('click', () => showView('worlds'));
$('quickCreate').addEventListener('click', () => showView('create'));
$('quickImport').addEventListener('click', () => showView('import'));
$('quickJoin').addEventListener('click', () => showView('join'));
$('cancelCreate').addEventListener('click', () => showView('worlds'));
$('cancelImport').addEventListener('click', () => showView('worlds'));
$('cancelJoin').addEventListener('click', () => showView('worlds'));
$('openWorldDiagnostics').addEventListener('click', () => showView('diagnostics'));
$('clearActivity').addEventListener('click', () => { $('output').textContent = 'Activity cleared.'; });
$('inviteWorld').addEventListener('click', () => { try { openInviteDialog(); } catch (error) { setOutput(String(error), 'Invite'); } });
$('sleepWorld').addEventListener('click', () => { try { openSleepDialog(); } catch (error) { setOutput(String(error), 'Stop world'); } });
$('leaveWorld').addEventListener('click', () => { try { openLeaveDialog(); } catch (error) { setOutput(String(error), 'Leave world'); } });
$('copyInvite').addEventListener('click', () => copyText($('inviteResult').value, 'Invite'));
$('copyWorldId').addEventListener('click', () => copyText(selectedWorldId, 'World ID'));

bindAction('sidebarInit', initialize);
bindAction('init', initialize);
bindAction('identityButton', () => showIdentity());
bindAction('refresh', refreshWorlds);
bindAction('createForm', createWorld, { submit: true });
bindAction('importForm', importWorld, { submit: true });
bindAction('joinForm', joinWorld, { submit: true });
bindAction('joinWorldIdButton', joinWorldId);
bindAction('createInvite', createInvite);
$('manageInvites')?.addEventListener('click', openInviteManagement);
$('refreshMembership')?.addEventListener('click', refreshMembershipStatus);
$('inspectInvite')?.addEventListener('click', inspectManagedInvite);
$('revokeInvite')?.addEventListener('click', revokeManagedInvite);
$('manageInviteToken')?.addEventListener('input', () => {
  if (inviteManagementBusy) return;
  clearInviteInspection();
  showInline('inviteManagementNotice', '');
});
$('revokeInviteConfirm')?.addEventListener('change', () => setInviteManagementBusy(inviteManagementBusy));
$('inviteManagementDialog')?.addEventListener('cancel', event => {
  if (inviteManagementBusy) event.preventDefault();
});
$('inviteManagementDialog')?.querySelector('form')?.addEventListener('submit', event => {
  if (inviteManagementBusy) event.preventDefault();
});
$('inviteManagementDialog')?.addEventListener('close', () => {
  if ($('inviteManagementDialog').open) return;
  inviteManagementWorld = '';
  $('manageInviteToken').value = '';
  clearInviteInspection();
  showInline('inviteManagementNotice', '');
});
bindAction('worldStatus', worldStatus);
bindAction('worldCompatibility', worldCompatibility);
bindAction('worldConflicts', worldConflicts);
bindAction('worldPeers', worldPeers);
bindAction('verifyWorld', verifyWorld);
bindAction('diagnosticVerify', verifyWorld);
$('retryRouteDiagnosis')?.addEventListener('click', () => { void retryConnectionDiagnosis(); });
$('copyRouteDiagnosis')?.addEventListener('click', copyConnectionDiagnosis);
bindAction('seedOn', () => setSeeding(true));
bindAction('seedOff', () => setSeeding(false));
bindAction('diagnosticSeedOn', () => setSeeding(true));
bindAction('diagnosticSeedOff', () => setSeeding(false));
bindAction('supplyRequiredMod', supplyRequiredMod);
bindAction('refreshMods', () => refreshWorldMods(selectedWorld()));
bindAction('openModsFolder', openModsFolder);
bindAction('confirmSleep', sleepWorld);
bindAction('confirmLeave', performLeaveWorld);
bindAction('playWorld', hostWorld);
bindAction('host', hostWorld);
bindAction('stopHost', stopHost);
bindAction('startDaemon', startDaemon);
bindAction('stopDaemon', stopDaemon);
bindAction('exportWorld', exportWorld);
bindAction('recoverWorld', recoverWorld);
bindAction('openRecoveryCopy', openRecoveryCopy);
$('confirmRecoveryCopy')?.addEventListener('click', confirmRecoveryCopy);
$('recoveryChoice')?.addEventListener('change', selectRecoveryChoice);
$('recoveryConflictAck')?.addEventListener('change', updateRecoveryChoiceControls);
$('recoveryExportDestination')?.addEventListener('input', updateRecoveryChoiceControls);
$('exportRecoveryChoice')?.addEventListener('click', () => performRecoveryChoice('export'));
$('exportRecoveryEvidence')?.addEventListener('click', () => performRecoveryChoice('evidence'));
$('recoveryCopyDialog')?.addEventListener('cancel', (event) => {
  if (recoveryCopyPending) event.preventDefault();
});
$('recoveryCopyDialog')?.querySelector('form')?.addEventListener('submit', (event) => {
  if (recoveryCopyPending) event.preventDefault();
});
$('recoveryCopyDialog')?.addEventListener('close', () => {
  // Native close events are queued; an immediately reopened dialog already
  // owns a different selection, which the previous close must not clear.
  if (!$('recoveryCopyDialog').open) { recoveryCopyTarget = null; recoveryReview = null; }
});
bindAction('transferHost', transferHost);
bindAction('wakeWorld', wakeWorld);
document.addEventListener('click', (event) => {
  if (!event.target?.closest?.('[data-reconnect-world]')) return;
  try { reconnectWorld(); } catch (error) { showInline('worldNotice', String(error), 'danger'); }
});

showView(currentView, { focus: false });
updateWorldSpecificControls();
renderMigration(null);
renderGameRoute();
window.swarmcraftRetryStartup = () => {
  if (startupInProgress) return;
  startup().catch((error) => {
    // The runtime retains the technical error; the startup surface avoids
    // writing raw sidecar stderr (which may include signed arguments).
    setOutput('Startup operation failed. Check the required sidecars and device data permissions.', 'Desktop startup failed');
    setNodeState('Setup needs attention', 'warning');
    window.__swarmcraftStartup?.fail('backend', error);
  });
};
window.swarmcraftRetryStartup();
document.addEventListener('visibilitychange', refreshVisibleMigration);
document.addEventListener('visibilitychange', refreshVisibleGameRoute);
document.addEventListener('visibilitychange', refreshVisibleHostReadiness);
setInterval(refreshVisibleMigration, MIGRATION_REFRESH_MS);
setInterval(refreshVisibleGameRoute, MIGRATION_REFRESH_MS);
setInterval(refreshVisibleHostReadiness, MIGRATION_REFRESH_MS);
setInterval(() => {
  if (!document.hidden) refreshConnectivityDiagnostics({ logFailure: false }).catch(() => {});
}, CONNECTIVITY_REFRESH_MS);
