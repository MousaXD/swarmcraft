import { createBackendAdapter, connectivityFromStatus, MIGRATION_PHASES, normalizeMigrationState } from './backend-adapter.js';

const $ = (id) => document.getElementById(id);
const invoke = window.__TAURI__?.core?.invoke;
const backend = createBackendAdapter(invoke);
const worldCache = new Map();
let selectedWorldId = '';
let currentView = 'worlds';

const viewMeta = {
  worlds: ['Worlds', 'Choose a world and play'],
  create: ['Create', 'Start a replicated Minecraft world'],
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
  $('status').textContent = message;
  $('statusDot').className = `status-dot ${tone}`;
}

function setOutput(output, label = '') {
  const console = $('output');
  const text = String(output || 'Done.');
  const stamp = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const entry = `${stamp}${label ? `  ${label}` : ''}\n${text}`;
  console.textContent = console.textContent.trim() === 'SwarmCraft is ready.' || console.textContent.trim() === 'Activity cleared.'
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
      setStatus('Ready', 'safe');
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
  if (name === 'diagnostics') updateDiagnosticsContext();
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
  if (safetyKind(world.status.Safety) === 'danger') {
    return { enabled: false, reason: 'Play is unavailable while divergent history needs review.' };
  }
  const value = String(world.compatibility?.['Authority eligibility'] || '').trim();
  if (value === 'Compatible') return { enabled: true, reason: 'This device is eligible to host this world.' };
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

function renderConnectivity(world) {
  const connectivity = connectivityFromStatus(world?.status || {});
  $('selectedConnectivity').textContent = connectivity.label;
  $('selectedConnectivityDetail').textContent = connectivity.detail;
  $('diagnosticConnectivity').textContent = connectivity.label;
  $('diagnosticConnectivity').className = `status-badge ${connectivity.kind}`;
  $('diagnosticConnectivityDetail').textContent = connectivity.detail;
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
  $('transferAvailability').textContent = migration.transfer ? '' : 'Transfer host is not available in this build.';
  $('migrationAvailability').textContent = migration.transfer || migration.wake
    ? 'Host handoff actions are enabled when the backend reports they are safe.'
    : 'Transfer and wake are unavailable in this build. Existing Play and graceful stop remain available.';
}

function clearSelection() {
  selectedWorldId = '';
  $('world').value = '';
  $('selectionContent').hidden = true;
  $('noSelection').hidden = false;
  for (const row of document.querySelectorAll('.world-row')) {
    row.classList.remove('is-selected');
    row.setAttribute('aria-pressed', 'false');
  }
  showInline('worldNotice', '');
  showInline('runtimeNotice', '');
  updateWorldSpecificControls();
  updateDiagnosticsContext();
}

function updateDiagnosticsContext() {
  const world = selectedWorld();
  if (!world) {
    $('diagnosticWorldName').textContent = 'No world selected';
    $('diagnosticWorldId').textContent = 'World-specific controls are unavailable.';
    $('diagnosticSafety').hidden = true;
    renderConnectivity(null);
    return;
  }
  $('diagnosticWorldName').textContent = world.name;
  $('diagnosticWorldId').textContent = world.id;
  const safety = world.status.Safety || 'Unknown';
  $('diagnosticSafety').textContent = safety;
  $('diagnosticSafety').className = `status-badge ${safetyKind(safety)}`;
  $('diagnosticSafety').hidden = false;
  renderConnectivity(world);
}

function renderMigration(state) {
  const migration = normalizeMigrationState(state);
  $('migrationBadge').textContent = migration.label;
  $('migrationBadge').className = `status-badge ${migration.failed ? 'danger' : migration.phase === 'ready' ? 'safe' : migration.available ? 'warning' : 'neutral'}`;
  $('migrationSummary').textContent = migration.detail || (migration.available ? migration.label : 'No host migration is active.');
  $('migrationProgress').hidden = !migration.available;
  $('migrationProgressBar').style.width = `${migration.progress}%`;

  const steps = $('migrationSteps');
  steps.replaceChildren();
  if (!migration.available) return;
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

async function refreshMigrationState(world) {
  if (!world || !backend.migration.capabilities.status) {
    renderMigration(null);
    return;
  }
  try {
    const state = await backend.migration.readState(world.id);
    renderMigration(state);
  } catch (error) {
    renderMigration({ phase: 'failed', detail: `Could not read host migration state: ${String(error)}` });
  }
}

function selectWorld(world, { focusDetail = false } = {}) {
  selectedWorldId = world.id;
  $('world').value = world.id;
  $('selectedName').textContent = world.name;
  $('selectedWorldId').textContent = world.id;
  $('selectedAuthority').textContent = world.status.Authority || 'Not reported';
  $('selectedMembers').textContent = world.status['Authorized peers'] || 'Unknown';
  $('selectedMinecraft').textContent = world.status.Minecraft || 'Unknown';
  $('selectedCheckpoint').textContent = world.status['Latest snapshot'] || (world.snapshots ? `${world.snapshots} stored` : 'None');
  $('selectedCompatibility').textContent = world.compatibility['Authority eligibility'] || 'Not synchronized';

  const safety = world.status.Safety || 'Unknown';
  const kind = safetyKind(safety);
  $('selectedSafety').textContent = safety;
  $('selectedSafety').className = `status-badge ${kind}`;
  $('safetyPanel').className = `safety-panel ${kind}`;
  $('selectedSummary').textContent = safetySummary(world);

  $('noSelection').hidden = true;
  $('selectionContent').hidden = false;
  showInline('worldNotice', '');
  showInline('runtimeNotice', '');
  for (const row of document.querySelectorAll('.world-row')) {
    const selected = row.dataset.worldId === world.id;
    row.classList.toggle('is-selected', selected);
    row.setAttribute('aria-pressed', selected ? 'true' : 'false');
  }

  renderConnectivity(world);
  updatePlayState(world);
  updateDiagnosticsContext();
  refreshMigrationState(world);
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
  connectivity.textContent = connectivityFromStatus(world.status).label;
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
  body.textContent = 'Create a world for your group or join one with a signed invite.';
  const actions = document.createElement('div');
  actions.className = 'compact-actions';
  const create = document.createElement('button');
  create.type = 'button';
  create.className = 'button button-primary';
  create.textContent = 'Create world';
  create.addEventListener('click', () => showView('create'));
  const join = document.createElement('button');
  join.type = 'button';
  join.className = 'button button-secondary';
  join.textContent = 'Join with invite';
  join.addEventListener('click', () => showView('join'));
  actions.append(create, join);
  empty.append(mark, title, body, actions);
  container.append(empty);
  container.setAttribute('aria-busy', 'false');
  $('worldCount').textContent = 'No local worlds';
  clearSelection();
}

async function refreshWorlds() {
  $('worldCards').setAttribute('aria-busy', 'true');
  showInline('worldLoadError', '');
  setStatus('Reading worlds…');
  let raw;
  try {
    raw = await run('Reading worlds…', () => backend.listWorlds(), { quiet: true });
  } catch (error) {
    renderEmptyWorlds();
    $('worldCount').textContent = 'Could not read local worlds';
    showInline('worldLoadError', `Worlds could not be loaded. ${String(error)}`, 'danger');
    setOutput(String(error), 'World refresh failed');
    setStatus('Worlds unavailable', 'danger');
    return;
  }

  const worlds = parseWorldList(raw);
  const previousSelection = selectedWorldId;
  const container = $('worldCards');
  container.replaceChildren();
  worldCache.clear();
  if (!worlds.length) {
    renderEmptyWorlds();
    setStatus('Ready', 'safe');
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
  setStatus('Ready', 'safe');
}

async function initialize() {
  const result = await run('Setting up this device…', () => backend.initializeNode());
  $('identity').textContent = String(result);
  setNodeState('Ready', 'safe');
  await refreshWorlds();
}

async function showIdentity({ quiet = false } = {}) {
  try {
    const result = await run('Reading device identity…', () => backend.nodeIdentity(), { quiet });
    $('identity').textContent = String(result);
    setNodeState('Ready', 'safe');
    return result;
  } catch (error) {
    $('identity').textContent = quiet ? 'Device is not initialized.' : String(error);
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
    result = await run('Joining world…', () => backend.joinWorld(invite));
  } catch (error) {
    showInline('joinError', `Could not join world: ${String(error)}`, 'danger');
    return;
  }
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) selectedWorldId = match[1];
  $('joinInvite').value = '';
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

function openInviteDialog() {
  const world = selectedWorld();
  if (!world) throw new Error('Choose a world first.');
  $('inviteWorldName').textContent = world.name;
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
  await run('Reading preserved conflicts…', () => backend.worldConflicts(worldId()));
  showView('activity');
}

async function worldPeers() {
  await run('Reading world members…', () => backend.worldPeers(worldId()));
  showView('activity');
}

async function verifyWorld() {
  await run('Verifying replica…', () => backend.verifyWorld(worldId()), { successMessage: 'Replica verification completed.' });
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
  await run('Requesting graceful sleep…', () => backend.stopHost(), {
    successMessage: 'World runtime stopped gracefully. Replica storage can continue separately.',
  });
  $('sleepDialog').close();
  showInline('worldNotice', 'World runtime stopped gracefully.', 'safe');
}

function openLeaveDialog() {
  const world = selectedWorld();
  if (!world) throw new Error('Choose a world first.');
  $('leaveDialogText').textContent = `This stages a signed leave request for ${world.name}. Local membership changes after the request is accepted.`;
  $('leaveDialog').showModal();
}

async function performLeaveWorld() {
  await run('Requesting membership leave…', () => backend.leaveWorld(worldId()));
  $('leaveDialog').close();
  selectedWorldId = '';
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
  const issue = runtimeValidationIssue();
  if (issue) {
    showRuntimeValidation(issue);
    return;
  }
  showInline('runtimeNotice', '');
  try {
    await run('Starting Minecraft…', () => backend.hostWorld({
      world: worldId(),
      java: $('java').value.trim(),
      serverJar: $('serverJar').value.trim(),
      modJar: $('modJar').value.trim(),
      acceptEula: $('eula').checked,
    }), { successMessage: (pid) => `Minecraft authority runtime started. PID ${pid}.` });
    showView('worlds');
    showInline('worldNotice', 'Minecraft runtime started.', 'safe');
  } catch (error) {
    showInline('runtimeNotice', `Could not start Minecraft: ${String(error)}`, 'danger');
    throw error;
  }
}

async function stopHost() {
  await run('Stopping Minecraft runtime…', () => backend.stopHost(), { successMessage: 'Minecraft runtime stopped.' });
}

async function startDaemon() {
  await run('Starting networking…', () => backend.startDaemon($('daemonListen').value.trim()), {
    successMessage: (pid) => `Networking daemon started. PID ${pid}.`,
  });
}

async function stopDaemon() {
  await run('Stopping networking…', () => backend.stopDaemon(), { successMessage: 'Networking daemon stopped.' });
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

async function transferHost() {
  if (!backend.migration.capabilities.transfer) return;
  await run('Preparing host transfer…', () => backend.migration.transferAuthority(worldId()));
  await refreshMigrationState(selectedWorld());
}

async function wakeWorld() {
  if (!backend.migration.capabilities.wake) return;
  await run('Waking world…', () => backend.migration.wakeWorld(worldId()));
  await refreshMigrationState(selectedWorld());
}

function bindAction(id, handler, { submit = false } = {}) {
  const element = $(id);
  if (!element) return;
  element.addEventListener(submit ? 'submit' : 'click', async (event) => {
    if (submit) event.preventDefault();
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
    }
  });
}

for (const nav of document.querySelectorAll('.nav-item[data-view]')) nav.addEventListener('click', () => showView(nav.dataset.view));
for (const back of document.querySelectorAll('[data-back-worlds]')) back.addEventListener('click', () => showView('worlds'));
$('quickCreate').addEventListener('click', () => showView('create'));
$('quickJoin').addEventListener('click', () => showView('join'));
$('cancelCreate').addEventListener('click', () => showView('worlds'));
$('cancelJoin').addEventListener('click', () => showView('worlds'));
$('openWorldDiagnostics').addEventListener('click', () => showView('diagnostics'));
$('clearActivity').addEventListener('click', () => { $('output').textContent = 'Activity cleared.'; });
$('inviteWorld').addEventListener('click', () => { try { openInviteDialog(); } catch (error) { setOutput(String(error), 'Invite'); } });
$('sleepWorld').addEventListener('click', () => { try { openSleepDialog(); } catch (error) { setOutput(String(error), 'Sleep world'); } });
$('leaveWorld').addEventListener('click', () => { try { openLeaveDialog(); } catch (error) { setOutput(String(error), 'Leave world'); } });
$('copyInvite').addEventListener('click', () => copyText($('inviteResult').value, 'Invite'));
$('copyWorldId').addEventListener('click', () => copyText(selectedWorldId, 'World ID'));

bindAction('sidebarInit', initialize);
bindAction('init', initialize);
bindAction('identityButton', () => showIdentity());
bindAction('refresh', refreshWorlds);
bindAction('createForm', createWorld, { submit: true });
bindAction('joinForm', joinWorld, { submit: true });
bindAction('joinWorldIdButton', joinWorldId);
bindAction('createInvite', createInvite);
bindAction('worldStatus', worldStatus);
bindAction('worldCompatibility', worldCompatibility);
bindAction('worldConflicts', worldConflicts);
bindAction('worldPeers', worldPeers);
bindAction('verifyWorld', verifyWorld);
bindAction('diagnosticVerify', verifyWorld);
bindAction('seedOn', () => setSeeding(true));
bindAction('seedOff', () => setSeeding(false));
bindAction('diagnosticSeedOn', () => setSeeding(true));
bindAction('diagnosticSeedOff', () => setSeeding(false));
bindAction('confirmSleep', sleepWorld);
bindAction('confirmLeave', performLeaveWorld);
bindAction('playWorld', hostWorld);
bindAction('host', hostWorld);
bindAction('stopHost', stopHost);
bindAction('startDaemon', startDaemon);
bindAction('stopDaemon', stopDaemon);
bindAction('exportWorld', exportWorld);
bindAction('recoverWorld', recoverWorld);
bindAction('transferHost', transferHost);
bindAction('wakeWorld', wakeWorld);

showView(currentView, { focus: false });
updateWorldSpecificControls();
renderMigration(null);
showIdentity({ quiet: true }).catch(() => {});
refreshWorlds().catch(() => {});
