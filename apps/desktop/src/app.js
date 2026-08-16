const invoke = window.__TAURI__?.core?.invoke;
const $ = (id) => document.getElementById(id);
const worldCache = new Map();
let selectedWorldId = '';
let currentView = 'worlds';

const viewMeta = {
  worlds: ['Worlds', 'Manage playable replicated worlds'],
  create: ['Create world', 'Start a new signed world configuration'],
  join: ['Join world', 'Membership and discovery'],
  activity: ['Activity', 'Command results and operational detail'],
  diagnostics: ['Diagnostics', 'Advanced networking, verification, and runtime controls'],
};

function requireTauri() {
  if (!invoke) throw new Error('Tauri runtime is unavailable');
  return invoke;
}

function setStatus(message) {
  $('status').textContent = message;
}

function setOutput(output, label = '') {
  const console = $('output');
  const text = String(output || 'Done.');
  const stamp = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const entry = `${stamp}${label ? `  ${label}` : ''}\n${text}`;
  console.textContent = console.textContent.trim() === 'SwarmCraft is ready.' ? entry : `${console.textContent.trim()}\n\n${entry}`;
  console.scrollTop = console.scrollHeight;
}

function showInline(id, message, tone = 'neutral') {
  const element = $(id);
  if (!element) return;
  element.textContent = message;
  element.hidden = !message;
  element.dataset.tone = tone;
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
    const target = panel?.querySelector('input:not([type="hidden"]):not(:disabled), textarea:not(:disabled), button:not(:disabled), summary, [tabindex]');
    target?.focus({ preventScroll: true });
  }
}

function activityLabel(label) {
  return label.replace(/…$/, '');
}

async function action(label, command, payload = {}, { quiet = false, successMessage = null } = {}) {
  if (!quiet) setStatus(label);
  try {
    const result = await requireTauri()(command, payload);
    if (!quiet) {
      const message = typeof successMessage === 'function' ? successMessage(result) : successMessage;
      if (message) setOutput(message, activityLabel(label));
      else if (result !== undefined && result !== null && String(result).trim()) setOutput(result, activityLabel(label));
      setStatus('Ready.');
    }
    return result;
  } catch (error) {
    if (!quiet) {
      setOutput(String(error), `${activityLabel(label)} failed`);
      setStatus('Action failed.');
      try { error.swarmcraftActivityLogged = true; } catch (_) {}
    }
    throw error;
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
  if (value.includes('canonical')) return 'safe';
  return 'neutral';
}

function safetySummary(world) {
  const label = world.status.Safety || 'Safety state unknown';
  const kind = safetyKind(label);
  if (kind === 'danger') {
    return 'Preserved divergent solo history exists. Inspect conflicts and preserve both branches rather than treating either as safe to overwrite.';
  }
  if (kind === 'warning') {
    return 'Solo advancement is recorded for this world. Safety guarantees are reduced until canonical history is re-established; this does not by itself indicate data loss.';
  }
  if (kind === 'safe') {
    return 'No solo-history conflict is currently recorded. Canonical metadata is available; live member availability and replication health are separate concerns.';
  }
  return 'Safety state is unavailable. Refresh or inspect full status before making assumptions about canonical history.';
}

function playEligibility(world) {
  const value = String(world?.compatibility?.['Authority eligibility'] || '').trim();
  if (value === 'Compatible') {
    return { enabled: true, reason: 'This node is authority eligible under the synchronized compatibility manifest.' };
  }
  if (value.toLowerCase().includes('not authority eligible') || value.toLowerCase().startsWith('replica only')) {
    return { enabled: false, reason: 'Play unavailable on this node. This replica is not authority eligible under the current compatibility manifest.' };
  }
  return { enabled: false, reason: 'Play unavailable until compatibility and authority eligibility are known.' };
}

function setNodeState(text, kind = 'neutral') {
  $('nodeBadge').textContent = text;
  $('nodeStateDot').className = `state-dot ${kind}`;
}

function selectedWorld() {
  return worldCache.get(selectedWorldId) || null;
}

function updatePlayState(world) {
  const eligibility = playEligibility(world);
  $('playWorld').disabled = !eligibility.enabled;
  $('playAvailability').textContent = eligibility.reason;
  $('playAvailability').dataset.tone = eligibility.enabled ? 'safe' : 'warning';
  $('runtimeEligibilityHint').textContent = eligibility.reason;
  updateWorldSpecificControls();
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
  updateDiagnosticsContext();
  updateWorldSpecificControls();
}

function updateDiagnosticsContext() {
  const world = selectedWorld();
  if (!world) {
    $('diagnosticWorldName').textContent = 'Select a world first';
    $('diagnosticWorldId').textContent = 'World-specific controls are unavailable.';
    $('diagnosticSafety').hidden = true;
    return;
  }
  $('diagnosticWorldName').textContent = world.name;
  $('diagnosticWorldId').textContent = world.id;
  const safety = world.status.Safety || 'Unknown';
  $('diagnosticSafety').textContent = safety;
  $('diagnosticSafety').className = `status-badge ${safetyKind(safety)}`;
  $('diagnosticSafety').hidden = false;
}

function updateWorldSpecificControls() {
  const world = selectedWorld();
  const hasWorld = Boolean(world);
  for (const control of document.querySelectorAll('.world-required')) {
    control.disabled = !hasWorld;
  }
  const eligibility = playEligibility(world);
  for (const control of document.querySelectorAll('.authority-required')) {
    control.disabled = !hasWorld || !eligibility.enabled;
  }
  if ($('playWorld')) $('playWorld').disabled = !hasWorld || !eligibility.enabled;
}

function selectWorld(world, { focusDetail = false } = {}) {
  selectedWorldId = world.id;
  $('world').value = world.id;
  $('selectedName').textContent = world.name;
  $('selectedWorldId').textContent = world.id;

  const safety = world.status.Safety || 'Unknown';
  const kind = safetyKind(safety);
  $('selectedSafety').textContent = safety;
  $('selectedSafety').className = `status-badge ${kind}`;
  $('selectedSummary').textContent = safetySummary(world);
  $('selectedSummary').className = `safety-summary ${kind}`;
  $('selectedAuthority').textContent = world.status.Authority || 'No snapshot authority recorded';
  $('selectedMembers').textContent = world.status['Authorized peers'] || 'Unknown';
  $('selectedMinecraft').textContent = world.status.Minecraft || 'Unknown';
  $('selectedCheckpoint').textContent = world.status['Latest snapshot'] || 'None';
  $('selectedCompatibility').textContent = world.compatibility['Authority eligibility'] || 'Compatibility not synchronized.';

  const secondaryActions = document.querySelector('.secondary-actions');
  if (secondaryActions) secondaryActions.open = kind === 'danger';
  $('noSelection').hidden = true;
  $('selectionContent').hidden = false;
  showInline('worldNotice', '');
  showInline('runtimeNotice', '');

  for (const row of document.querySelectorAll('.world-row')) {
    const selected = row.dataset.worldId === world.id;
    row.classList.toggle('is-selected', selected);
    row.setAttribute('aria-pressed', selected ? 'true' : 'false');
  }

  updatePlayState(world);
  updateDiagnosticsContext();
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
  const members = document.createElement('span');
  members.textContent = `${world.status['Authorized peers'] || 'Unknown'} members`;
  const checkpoint = document.createElement('span');
  checkpoint.textContent = `Checkpoint ${world.status['Latest snapshot'] || 'none'}`;
  meta.append(minecraft, members, checkpoint);

  row.append(title, safety, meta);
  if (world.id === selectedWorldId) row.classList.add('is-selected');
  return row;
}

function renderEmptyWorlds() {
  const container = $('worldCards');
  container.replaceChildren();
  const empty = document.createElement('div');
  empty.className = 'empty-state';
  const title = document.createElement('h3');
  title.textContent = 'No worlds yet';
  const body = document.createElement('p');
  body.textContent = 'Create a new world or join one with a signed invitation.';
  const actions = document.createElement('div');
  actions.className = 'button-row';
  const create = document.createElement('button');
  create.type = 'button';
  create.className = 'button button-primary';
  create.textContent = 'Create world';
  create.addEventListener('click', () => showView('create'));
  const join = document.createElement('button');
  join.type = 'button';
  join.className = 'button button-secondary';
  join.textContent = 'Join world';
  join.addEventListener('click', () => showView('join'));
  actions.append(create, join);
  empty.append(title, body, actions);
  container.append(empty);
  $('worldCount').textContent = 'No local worlds';
  clearSelection();
}

async function initialize() {
  const result = await action('Initializing local identity…', 'initialize_node');
  $('identity').textContent = String(result);
  setNodeState('Initialized', 'safe');
  await refreshWorlds();
}

async function showIdentity({ quiet = false } = {}) {
  try {
    const result = await action('Reading local identity…', 'node_identity', {}, { quiet });
    $('identity').textContent = String(result);
    setNodeState('Initialized', 'safe');
    return result;
  } catch (error) {
    setNodeState('Not initialized', 'neutral');
    if (!quiet) throw error;
    return null;
  }
}

async function refreshWorlds() {
  setStatus('Reading local worlds…');
  let raw;
  try {
    raw = await action('Reading local worlds…', 'list_worlds', {}, { quiet: true });
  } catch (error) {
    renderEmptyWorlds();
    $('worldCount').textContent = 'Unable to read local worlds';
    setStatus('Could not read local worlds.');
    setOutput(String(error), 'World refresh failed');
    return;
  }

  const worlds = parseWorldList(raw);
  const previousSelection = selectedWorldId;
  const container = $('worldCards');
  container.replaceChildren();
  worldCache.clear();

  if (!worlds.length) {
    renderEmptyWorlds();
    setStatus('Ready.');
    return;
  }

  $('worldCount').textContent = `${worlds.length} local world${worlds.length === 1 ? '' : 's'}`;
  for (const world of worlds) {
    let statusText = '';
    let compatibilityText = '';
    try {
      [statusText, compatibilityText] = await Promise.all([
        action('Reading world status…', 'world_status', { world: world.id }, { quiet: true }),
        action('Reading compatibility…', 'world_compatibility', { world: world.id }, { quiet: true }),
      ]);
    } catch (_) {
      // A partially synchronized local copy still belongs in the world list.
    }
    const model = { ...world, status: parseLines(statusText), compatibility: parseLines(compatibilityText) };
    worldCache.set(world.id, model);
    container.append(renderWorld(model));
  }

  const nextSelection = worldCache.get(previousSelection) || worldCache.values().next().value;
  if (nextSelection) selectWorld(nextSelection);
  else clearSelection();
  setStatus('Ready.');
}

async function startDaemon() {
  await action('Starting replication daemon…', 'start_daemon', { listen: $('daemonListen').value.trim() }, {
    successMessage: (pid) => `Replication daemon started. PID ${pid}.`,
  });
}

async function stopDaemon() {
  await action('Stopping replication daemon…', 'stop_daemon', {}, { successMessage: 'Replication daemon stopped.' });
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
    result = await action('Creating signed world configuration…', 'create_world', {
      name: $('createName').value.trim(),
      minecraft: $('createMinecraft').value.trim(),
      fabricLoader: $('createLoader').value.trim(),
      compatibility: $('createCompatibility').value.trim(),
      visibility: $('createVisibility').value,
    });
  } catch (error) {
    showInline('createError', `Could not create world: ${String(error)}`, 'danger');
    return;
  }
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) selectedWorldId = match[1];
  await refreshWorlds();
  if (match && worldCache.has(match[1])) selectWorld(worldCache.get(match[1]));
  showView('worlds');
}

async function joinWorld() {
  const invite = $('joinInvite').value.trim();
  if (!invite) {
    showInline('joinError', 'Signed invite is required.', 'danger');
    $('joinInvite').focus();
    return;
  }
  showInline('joinError', '');
  let result;
  try {
    result = await action('Staging signed join request…', 'join_world', { invite });
  } catch (error) {
    showInline('joinError', `Could not join world: ${String(error)}`, 'danger');
    return;
  }
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) selectedWorldId = match[1];
  await refreshWorlds();
  if (match && worldCache.has(match[1])) selectWorld(worldCache.get(match[1]));
  showView('worlds');
}

async function joinWorldId() {
  const world = $('joinWorldId').value.trim();
  if (!world) {
    showInline('joinWorldIdNotice', 'Enter a World ID first.', 'danger');
    $('joinWorldId').focus();
    return;
  }
  const message = `World-ID discovery for ${world} cannot bypass membership. Private worlds still require a signed invite; public or unlisted discovery remains non-authoritative until membership is accepted.`;
  showInline('joinWorldIdNotice', message);
  setOutput(message, 'Discovery');
  setStatus('Signed membership still required.');
}

function worldId() {
  const value = $('world').value.trim();
  if (!value) throw new Error('Choose a world first');
  return value;
}

async function worldStatus() {
  await action('Reading canonical world state…', 'world_status', { world: worldId() });
}
async function worldCompatibility() {
  await action('Checking execution compatibility…', 'world_compatibility', { world: worldId() });
}
async function worldConflicts() {
  await action('Reading preserved solo branches…', 'world_conflicts', { world: worldId() });
}
async function worldPeers() {
  await action('Reading authorized membership…', 'world_peers', { world: worldId() });
}
async function verifyWorld() {
  await action('Verifying snapshot history and blobs…', 'verify_world', { world: worldId() });
}

function openLeaveDialog() {
  const model = selectedWorld();
  if (!model) throw new Error('Choose a world first');
  $('leaveDialogText').textContent = `This stages a signed leave request for ${model.name}. Local membership changes after the request is accepted.`;
  $('leaveDialog').showModal();
}

async function performLeaveWorld() {
  await action('Staging signed leave request…', 'leave_world', { world: worldId() });
  $('leaveDialog').close();
  selectedWorldId = '';
  $('world').value = '';
  await refreshWorlds();
}

async function setSeeding(enabled) {
  await action(`${enabled ? 'Enabling' : 'Disabling'} background replica…`, 'set_background_seeding', { world: worldId(), enabled });
  await refreshWorlds();
}

async function createInvite() {
  const bootstrapAddrs = $('bootstrapAddrs').value.split('\n').map((value) => value.trim()).filter(Boolean);
  await action('Creating signed authority invite…', 'create_invite', {
    world: worldId(),
    expiresMinutes: Number($('inviteMinutes').value || 60),
    bootstrapAddrs,
  });
}

async function exportWorld() {
  await action('Exporting latest verified snapshot…', 'export_world', { world: worldId(), destination: $('destination').value.trim() });
}

async function recoverWorld() {
  await action('Recovering selected verified snapshot…', 'recover_world', {
    world: worldId(),
    snapshot: Number($('snapshotNumber').value || 1),
    destination: $('destination').value.trim(),
  });
}

function runtimeValidationIssue() {
  const world = selectedWorld();
  if (!world) return [null, 'Select a world first before starting an authority runtime.'];
  const eligibility = playEligibility(world);
  if (!eligibility.enabled) return [null, eligibility.reason];
  if (!$('serverJar').value.trim()) return ['serverJar', 'Fabric server jar is required before the authority runtime can start.'];
  if (!$('modJar').value.trim()) return ['modJar', 'SwarmCraft Fabric mod jar is required before the authority runtime can start.'];
  if (!$('eula').checked) return ['eula', 'Minecraft server EULA acceptance is required before the authority runtime can start.'];
  return null;
}

function showRuntimeValidation(issue, { fromPlay = false } = {}) {
  const [fieldId, message] = issue;
  if (fromPlay) showView('diagnostics', { focus: false });
  showInline('runtimeNotice', message, 'warning');
  $('runtimeSection').scrollIntoView({ block: 'start' });
  if (fieldId) $(fieldId).focus();
  else $('runtimeNotice').focus?.();
}

async function hostWorld({ fromPlay = false } = {}) {
  const issue = runtimeValidationIssue();
  if (issue) {
    showRuntimeValidation(issue, { fromPlay });
    return;
  }
  showInline('runtimeNotice', '');
  try {
    await action('Starting authority runtime…', 'host_world', {
      world: worldId(),
      java: $('java').value.trim(),
      serverJar: $('serverJar').value.trim(),
      modJar: $('modJar').value.trim(),
      acceptEula: $('eula').checked,
    }, { successMessage: (pid) => `Authority runtime started. PID ${pid}.` });
  } catch (error) {
    showInline('runtimeNotice', `Could not start authority runtime: ${String(error)}`, 'danger');
    throw error;
  }
}

async function stopHost(label = 'Stopping authority runtime…') {
  await action(label, 'stop_host', {}, {
    successMessage: 'Authority runtime stopped. The replication daemon may continue serving stored snapshots.',
  });
}

function bindAction(id, handler, { submit = false } = {}) {
  const element = $(id);
  const eventName = submit ? 'submit' : 'click';
  element.addEventListener(eventName, async (event) => {
    if (submit) event.preventDefault();
    if (element.dataset.busy === 'true') return;
    const busyControl = submit && event.submitter ? event.submitter : element;
    element.dataset.busy = 'true';
    busyControl.disabled = true;
    busyControl.setAttribute('aria-busy', 'true');
    try {
      await handler();
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

for (const nav of document.querySelectorAll('.nav-item[data-view]')) {
  nav.addEventListener('click', () => showView(nav.dataset.view));
}

$('emptyCreate')?.addEventListener('click', () => showView('create'));
$('emptyJoin')?.addEventListener('click', () => showView('join'));
$('cancelCreate').addEventListener('click', () => showView('worlds'));
$('cancelJoin').addEventListener('click', () => showView('worlds'));
$('clearActivity').addEventListener('click', () => { $('output').textContent = 'Activity cleared.'; });
$('leaveWorld').addEventListener('click', () => {
  try { openLeaveDialog(); } catch (error) { setOutput(String(error), 'Leave world'); }
});

bindAction('sidebarInit', initialize);
bindAction('init', initialize);
bindAction('identityButton', () => showIdentity());
bindAction('refresh', refreshWorlds);
bindAction('startDaemon', startDaemon);
bindAction('stopDaemon', stopDaemon);
bindAction('createForm', createWorld, { submit: true });
bindAction('joinForm', joinWorld, { submit: true });
bindAction('joinWorldIdButton', joinWorldId);
bindAction('worldStatus', worldStatus);
bindAction('worldCompatibility', worldCompatibility);
bindAction('worldConflicts', worldConflicts);
bindAction('worldPeers', worldPeers);
bindAction('verifyWorld', verifyWorld);
bindAction('confirmLeave', performLeaveWorld);
bindAction('seedOn', () => setSeeding(true));
bindAction('seedOff', () => setSeeding(false));
bindAction('createInvite', createInvite);
bindAction('exportWorld', exportWorld);
bindAction('recoverWorld', recoverWorld);
bindAction('playWorld', () => hostWorld({ fromPlay: true }));
bindAction('host', () => hostWorld());
bindAction('gracefulSleep', () => stopHost('Requesting graceful sleep…'));
bindAction('stopHost', () => stopHost());

showView(currentView, { focus: false });
updateWorldSpecificControls();
showIdentity({ quiet: true }).catch(() => {});
refreshWorlds().catch(() => {});
