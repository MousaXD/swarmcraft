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
  if (focus) {
    const panel = document.querySelector(`[data-view-panel="${name}"]`);
    const target = panel?.querySelector('input:not([type="hidden"]), textarea, button, summary, [tabindex]');
    target?.focus({ preventScroll: true });
  }
}

async function action(label, command, payload = {}, { quiet = false } = {}) {
  if (!quiet) setStatus(label);
  try {
    const result = await requireTauri()(command, payload);
    if (!quiet && result !== undefined && result !== null) setOutput(result, label.replace(/…$/, ''));
    if (!quiet) setStatus('Ready.');
    return result;
  } catch (error) {
    if (!quiet) {
      setOutput(String(error), `${label.replace(/…$/, '')} failed`);
      setStatus('Action failed.');
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
  if (value.includes('quorum') || value.includes('canonical') || value.includes('replicated')) return 'safe';
  return 'neutral';
}

function safetySummary(world) {
  const label = world.status.Safety || 'Canonical state unknown';
  const kind = safetyKind(label);
  if (kind === 'danger') return 'Conflict detected. Inspect preserved branches before treating this world as canonical.';
  if (kind === 'warning') return 'This world is in a solo or degraded state. Local play may be possible, but canonical replication guarantees are reduced.';
  if (kind === 'safe') return 'Canonical state is healthy. Replication and authority information below reflects the synchronized world state.';
  return 'Canonical safety state is not available yet. Refresh or inspect full status before assuming the world is replicated.';
}

function setNodeState(text, kind = 'neutral') {
  $('nodeBadge').textContent = text;
  $('nodeStateDot').className = `state-dot ${kind}`;
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
  $('selectedAuthority').textContent = world.status.Authority || 'Not established';
  $('selectedReplicas').textContent = world.status['Authorized peers'] || 'Unknown';
  $('selectedMinecraft').textContent = world.status.Minecraft || 'Unknown';
  $('selectedCheckpoint').textContent = world.status['Latest snapshot'] || 'None';
  $('selectedCompatibility').textContent = world.compatibility['Authority eligibility'] || 'Compatibility not synchronized.';
  const secondaryActions = document.querySelector('.secondary-actions');
  if (secondaryActions) secondaryActions.open = kind === 'danger';
  $('noSelection').hidden = true;
  $('selectionContent').hidden = false;
  showInline('worldNotice', '');

  for (const row of document.querySelectorAll('.world-row')) {
    const selected = row.dataset.worldId === world.id;
    row.classList.toggle('is-selected', selected);
    row.setAttribute('aria-pressed', selected ? 'true' : 'false');
  }

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
  const replicas = document.createElement('span');
  replicas.textContent = `${world.status['Authorized peers'] || 'Unknown'} replicas`;
  const checkpoint = document.createElement('span');
  checkpoint.textContent = `Checkpoint ${world.status['Latest snapshot'] || 'none'}`;
  meta.append(minecraft, replicas, checkpoint);

  row.append(title, safety, meta);
  if (world.id === selectedWorldId) row.classList.add('is-selected');
  return row;
}

function renderEmptyWorlds() {
  const container = $('worldCards');
  container.replaceChildren();
  const empty = document.createElement('div');
  empty.className = 'empty-state';
  const symbol = document.createElement('div');
  symbol.className = 'empty-symbol';
  symbol.setAttribute('aria-hidden', 'true');
  symbol.textContent = '◇';
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
  empty.append(symbol, title, body, actions);
  container.append(empty);
  $('worldCount').textContent = 'No local worlds';
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
      // A partially synchronized replica still belongs in the world list.
    }
    const model = { ...world, status: parseLines(statusText), compatibility: parseLines(compatibilityText) };
    worldCache.set(world.id, model);
    container.append(renderWorld(model));
  }

  const nextSelection = worldCache.get(selectedWorldId) || worldCache.values().next().value;
  if (nextSelection) selectWorld(nextSelection);
  setStatus('Ready.');
}

async function startDaemon() {
  const pid = await action('Starting replication daemon…', 'start_daemon', { listen: $('daemonListen').value.trim() });
  setOutput(`Replication daemon started. PID ${pid}.`, 'Daemon');
}

async function stopDaemon() {
  await action('Stopping replication daemon…', 'stop_daemon');
  setOutput('Replication daemon stopped.', 'Daemon');
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
  await action('Reading canonical membership…', 'world_peers', { world: worldId() });
}
async function verifyWorld() {
  await action('Verifying snapshot history and blobs…', 'verify_world', { world: worldId() });
}

function openLeaveDialog() {
  const model = worldCache.get(worldId());
  $('leaveDialogText').textContent = `This stages a signed leave request for ${model?.name || 'the selected world'}. Local membership changes after the request is accepted.`;
  $('leaveDialog').showModal();
}

async function performLeaveWorld() {
  await action('Staging signed leave request…', 'leave_world', { world: worldId() });
  $('leaveDialog').close();
  selectedWorldId = '';
  $('world').value = '';
  $('selectionContent').hidden = true;
  $('noSelection').hidden = false;
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

function runtimeMissingField() {
  if (!$('serverJar').value.trim()) return ['serverJar', 'Fabric server jar is required before Play can start the authority runtime.'];
  if (!$('modJar').value.trim()) return ['modJar', 'SwarmCraft Fabric mod jar is required before Play can start the authority runtime.'];
  if (!$('eula').checked) return ['eula', 'Minecraft server EULA acceptance is required before Play can start the authority runtime.'];
  return null;
}

async function hostWorld({ fromPlay = false } = {}) {
  worldId();
  const missing = runtimeMissingField();
  if (missing) {
    const [fieldId, message] = missing;
    showInline('worldNotice', message, 'warning');
    if (fromPlay) {
      showView('diagnostics', { focus: false });
      $('runtimeSection').scrollIntoView({ block: 'start' });
      $(fieldId).focus();
    }
    return;
  }
  try {
    const pid = await action('Starting authority runtime…', 'host_world', {
      world: worldId(),
      java: $('java').value.trim(),
      serverJar: $('serverJar').value.trim(),
      modJar: $('modJar').value.trim(),
      acceptEula: $('eula').checked,
    });
    showInline('worldNotice', '');
    setOutput(`Authority runtime started. PID ${pid}.`, 'Runtime');
  } catch (error) {
    showInline('worldNotice', `Could not start authority runtime: ${String(error)}`, 'danger');
  }
}

async function stopHost(label = 'Stopping authority runtime…') {
  await action(label, 'stop_host');
  setOutput('Authority runtime stopped. The daemon may continue serving replicas.', 'Runtime');
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
      setOutput(String(error), 'Action failed');
    } finally {
      element.dataset.busy = 'false';
      busyControl.disabled = false;
      busyControl.removeAttribute('aria-busy');
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
showIdentity({ quiet: true }).catch(() => {});
refreshWorlds().catch(() => {});
