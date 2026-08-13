const invoke = window.__TAURI__?.core?.invoke;
const $ = (id) => document.getElementById(id);

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

async function action(label, command, payload = {}) {
  setStatus(label);
  try {
    const result = await requireTauri()(command, payload);
    if (result !== undefined && result !== null) setOutput(String(result));
    setStatus('Ready.');
    return result;
  } catch (error) {
    setOutput(String(error));
    setStatus('Action failed.');
    throw error;
  }
}

async function initialize() {
  const result = await action('Initializing local identity…', 'initialize_node');
  $('identity').textContent = String(result);
  await refreshWorlds();
}

async function showIdentity() {
  const result = await action('Reading local identity…', 'node_identity');
  $('identity').textContent = String(result);
}

async function refreshWorlds() {
  const result = await action('Reading local replicas…', 'list_worlds');
  $('worlds').textContent = String(result || 'No local worlds.');
}

async function createWorld() {
  const result = await action('Creating canonical world identity…', 'create_world', {
    name: $('createName').value.trim(),
    minecraft: $('createMinecraft').value.trim(),
    fabricLoader: $('createLoader').value.trim(),
    compatibility: $('createCompatibility').value.trim(),
  });
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) $('world').value = match[1];
  await refreshWorlds();
}

async function joinWorld() {
  const result = await action('Staging signed join request…', 'join_world', {
    invite: $('joinInvite').value.trim(),
  });
  const match = String(result).match(/World ID:\s*(scworld:[^\s]+)/);
  if (match) $('world').value = match[1];
  await refreshWorlds();
}

function worldId() {
  return $('world').value.trim();
}

async function worldStatus() {
  await action('Reading canonical world state…', 'world_status', { world: worldId() });
}

async function worldPeers() {
  await action('Reading canonical membership…', 'world_peers', { world: worldId() });
}

async function verifyWorld() {
  await action('Verifying snapshot history and blobs…', 'verify_world', { world: worldId() });
}

async function leaveWorld() {
  await action('Staging signed leave request…', 'leave_world', { world: worldId() });
}

async function createInvite() {
  const bootstrapAddrs = $('bootstrapAddrs').value
    .split('\n')
    .map((value) => value.trim())
    .filter(Boolean);
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
  const pid = await action('Starting verified authority runtime…', 'host_world', {
    world: worldId(),
    java: $('java').value.trim(),
    serverJar: $('serverJar').value.trim(),
    modJar: $('modJar').value.trim(),
    acceptEula: $('eula').checked,
  });
  setOutput(`Authority runtime started. PID ${pid}.`);
}

const bindings = [
  ['init', initialize],
  ['identityButton', showIdentity],
  ['refresh', refreshWorlds],
  ['createWorld', createWorld],
  ['joinWorld', joinWorld],
  ['worldStatus', worldStatus],
  ['worldPeers', worldPeers],
  ['verifyWorld', verifyWorld],
  ['leaveWorld', leaveWorld],
  ['createInvite', createInvite],
  ['exportWorld', exportWorld],
  ['recoverWorld', recoverWorld],
  ['host', hostWorld],
];
for (const [id, handler] of bindings) {
  $(id).addEventListener('click', () => handler().catch(() => {}));
}
