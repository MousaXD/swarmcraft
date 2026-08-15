const invoke = window.__TAURI__?.core?.invoke;
const $ = (id) => document.getElementById(id);
const worldCache = new Map();

function requireTauri() {
  if (!invoke) throw new Error('Tauri runtime is unavailable');
  return invoke;
}

function setStatus(message) {
  $('status').textContent = message;
}

function setOutput(output) {
  $('output').textContent = output || 'Done.';
}

async function action(label, command, payload = {}, { quiet = false } = {}) {
  if (!quiet) setStatus(label);
  try {
    const result = await requireTauri()(command, payload);
    if (!quiet && result !== undefined && result !== null) setOutput(String(result));
    if (!quiet) setStatus('Ready.');
    return result;
  } catch (error) {
    if (!quiet) {
      setOutput(String(error));
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

function badgeClass(safety) {
  const value = String(safety || '').toLowerCase();
  if (value.includes('conflict')) return 'danger';
  if (value.includes('solo')) return 'warning';
  if (value.includes('quorum') || value.includes('canonical') || value.includes('replicated')) return 'safe';
  return 'neutral';
}

function chip(label, value) {
  const item = document.createElement('span');
  item.className = 'chip';
  const strong = document.createElement('strong');
  strong.textContent = label;
  item.append(strong, document.createTextNode(` ${value}`));
  return item;
}

function selectWorld(world) {
  $('world').value = world.id;
  $('selectedName').textContent = world.name;
  $('selectedSafety').textContent = world.status.Safety || 'Canonical state';
  $('selectedSafety').className = `badge ${badgeClass(world.status.Safety)}`;
  const authority = world.status.Authority || 'not established';
  const checkpoint = world.status['Latest snapshot'] || 'none';
  $('selectedSummary').textContent = `${world.id} · authority ${authority} · latest safe checkpoint ${checkpoint}`;
  $('selectedPanel').scrollIntoView({ behavior: 'smooth', block: 'nearest' });
}

function renderWorld(world) {
  const card = document.createElement('article');
  card.className = 'world-card';
  card.tabIndex = 0;
  card.addEventListener('click', () => selectWorld(world));
  card.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') selectWorld(world);
  });

  const top = document.createElement('div');
  top.className = 'world-card-top';
  const title = document.createElement('div');
  const name = document.createElement('h3');
  name.textContent = world.name;
  const id = document.createElement('p');
  id.className = 'world-id';
  id.textContent = world.id;
  title.append(name, id);
  const safety = document.createElement('span');
  safety.className = `badge ${badgeClass(world.status.Safety)}`;
  safety.textContent = world.status.Safety || 'Canonical';
  top.append(title, safety);

  const chips = document.createElement('div');
  chips.className = 'chips';
  chips.append(
    chip('Minecraft', world.status.Minecraft || 'unknown'),
    chip('Replicas', world.status['Authorized peers'] || 'unknown'),
    chip('Checkpoint', world.status['Latest snapshot'] || 'none'),
  );

  const compatibility = document.createElement('p');
  compatibility.className = 'compatibility';
  compatibility.textContent = world.compatibility['Authority eligibility'] || 'Compatibility not synchronized';

  const actions = document.createElement('div');
  actions.className = 'button-row';
  const play = document.createElement('button');
  play.textContent = 'Play';
  play.addEventListener('click', (event) => {
    event.stopPropagation();
    selectWorld(world);
    $('serverJar').focus();
  });
  const details = document.createElement('button');
  details.className = 'secondary';
  details.textContent = 'Details';
  details.addEventListener('click', (event) => {
    event.stopPropagation();
    selectWorld(world);
    worldStatus().catch(() => {});
  });
  actions.append(play, details);

  card.append(top, chips, compatibility, actions);
  return card;
}

async function initialize() {
  const result = await action('Initializing local identity…', 'initialize_node');
  $('identity').textContent = String(result);
  $('nodeBadge').textContent = 'Node initialized';
  $('nodeBadge').className = 'badge safe';
  await refreshWorlds();
}

async function showIdentity() {
  const result = await action('Reading local identity…', 'node_identity');
  $('identity').textContent = String(result);
}

async function refreshWorlds() {
  setStatus('Reading local worlds…');
  const raw = await action('Reading local worlds…', 'list_worlds', {}, { quiet: true });
  const worlds = parseWorldList(raw);
  const container = $('worldCards');
  container.replaceChildren();
  worldCache.clear();

  if (!worlds.length) {
    const empty = document.createElement('article');
    empty.className = 'world-card empty-card';
    const title = document.createElement('h3');
    title.textContent = 'No worlds yet';
    const body = document.createElement('p');
    body.textContent = 'Create a world or join one with a signed invite.';
    empty.append(title, body);
    container.append(empty);
    setStatus('Ready.');
    return;
  }

  for (const world of worlds) {
    let statusText = '';
    let compatibilityText = '';
    try {
      [statusText, compatibilityText] = await Promise.all([
        action('Reading world status…', 'world_status', { world: world.id }, { quiet: true }),
        action('Reading compatibility…', 'world_compatibility', { world: world.id }, { quiet: true }),
      ]);
    } catch (_) {
      // A partially synchronized replica still belongs on the dashboard.
    }
    const model = {
      ...world,
      status: parseLines(statusText),
      compatibility: parseLines(compatibilityText),
    };
    worldCache.set(world.id, model);
    container.append(renderWorld(model));
  }
  setStatus('Ready.');
}

async function startDaemon() {
  const pid = await action('Starting replication daemon…', 'start_daemon', { listen: $('daemonListen').value.trim() });
  setOutput(`Replication daemon started. PID ${pid}.`);
}

async function stopDaemon() {
  await action('Stopping replication daemon…', 'stop_daemon');
  setOutput('Replication daemon stopped.');
}

async function createWorld() {
  const result = await action('Creating signed world configuration…', 'create_world', {
    name: $('createName').value.trim(),
    minecraft: $('createMinecraft').value.trim(),
    fabricLoader: $('createLoader').value.trim(),
    compatibility: $('createCompatibility').value.trim(),
    visibility: $('createVisibility').value,
  });
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) $('world').value = match[1];
  await refreshWorlds();
  if (match && worldCache.has(match[1])) selectWorld(worldCache.get(match[1]));
}

async function joinWorld() {
  const result = await action('Staging signed join request…', 'join_world', { invite: $('joinInvite').value.trim() });
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) $('world').value = match[1];
  await refreshWorlds();
}

async function joinWorldId() {
  const world = $('joinWorldId').value.trim();
  if (!world) throw new Error('World ID is required');
  setOutput(`World-ID discovery for ${world} cannot bypass membership. Paste a signed invite for private worlds; public/unlisted discovery uses configured peers and remains non-authoritative until membership is accepted.`);
  setStatus('Invite or reachable discovery peer required.');
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

async function leaveWorld() {
  await action('Staging signed leave request…', 'leave_world', { world: worldId() });
  await refreshWorlds();
}

async function setSeeding(enabled) {
  await action(`${enabled ? 'Enabling' : 'Disabling'} background replica…`, 'set_background_seeding', {
    world: worldId(),
    enabled,
  });
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
  await action('Exporting latest verified snapshot…', 'export_world', {
    world: worldId(),
    destination: $('destination').value.trim(),
  });
}

async function recoverWorld() {
  await action('Recovering selected verified snapshot…', 'recover_world', {
    world: worldId(),
    snapshot: Number($('snapshotNumber').value || 1),
    destination: $('destination').value.trim(),
  });
}

async function hostWorld() {
  const pid = await action('Starting authority runtime…', 'host_world', {
    world: worldId(),
    java: $('java').value.trim(),
    serverJar: $('serverJar').value.trim(),
    modJar: $('modJar').value.trim(),
    acceptEula: $('eula').checked,
  });
  setOutput(`Authority runtime started. PID ${pid}.`);
}

async function stopHost(label = 'Stopping authority runtime…') {
  await action(label, 'stop_host');
  setOutput('Authority runtime stopped. The daemon may continue serving replicas.');
}

function scrollTo(id) {
  $(id).scrollIntoView({ behavior: 'smooth', block: 'start' });
}

const bindings = [
  ['init', initialize],
  ['identityButton', showIdentity],
  ['refresh', refreshWorlds],
  ['openCreate', () => scrollTo('createPanel')],
  ['openJoin', () => scrollTo('joinPanel')],
  ['startDaemon', startDaemon],
  ['stopDaemon', stopDaemon],
  ['createWorld', createWorld],
  ['joinWorld', joinWorld],
  ['joinWorldIdButton', joinWorldId],
  ['worldStatus', worldStatus],
  ['worldCompatibility', worldCompatibility],
  ['worldConflicts', worldConflicts],
  ['worldPeers', worldPeers],
  ['verifyWorld', verifyWorld],
  ['leaveWorld', leaveWorld],
  ['seedOn', () => setSeeding(true)],
  ['seedOff', () => setSeeding(false)],
  ['createInvite', createInvite],
  ['exportWorld', exportWorld],
  ['recoverWorld', recoverWorld],
  ['playWorld', hostWorld],
  ['host', hostWorld],
  ['gracefulSleep', () => stopHost('Requesting graceful sleep…')],
  ['stopHost', () => stopHost()],
];
for (const [id, handler] of bindings) {
  $(id).addEventListener('click', () => Promise.resolve(handler()).catch((error) => setOutput(String(error))));
}

refreshWorlds().catch(() => {});
