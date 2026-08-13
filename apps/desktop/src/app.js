const invoke = window.__TAURI__?.core?.invoke;

function requireTauri() {
  if (!invoke) throw new Error('Tauri runtime is unavailable');
  return invoke;
}

const $ = (id) => document.getElementById(id);

async function initialize() {
  $('status').textContent = 'Initializing local identity…';
  try {
    const output = await requireTauri()('initialize_node');
    $('identity').textContent = output;
    $('status').textContent = 'Local node ready.';
  } catch (error) {
    $('status').textContent = String(error);
  }
}

async function refreshWorlds() {
  $('status').textContent = 'Reading local replicas…';
  try {
    $('worlds').textContent = await requireTauri()('list_worlds');
    $('status').textContent = 'World list updated.';
  } catch (error) {
    $('status').textContent = String(error);
  }
}

async function hostWorld() {
  $('status').textContent = 'Starting verified authority runtime…';
  try {
    const pid = await requireTauri()('host_world', {
      world: $('world').value.trim(),
      java: $('java').value.trim(),
      serverJar: $('serverJar').value.trim(),
      modJar: $('modJar').value.trim(),
      acceptEula: $('eula').checked,
    });
    $('status').textContent = `Authority runtime started (PID ${pid}).`;
  } catch (error) {
    $('status').textContent = String(error);
  }
}

$('init').addEventListener('click', initialize);
$('refresh').addEventListener('click', refreshWorlds);
$('host').addEventListener('click', hostWorld);
