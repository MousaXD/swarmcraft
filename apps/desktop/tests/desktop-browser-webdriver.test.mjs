import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const sourceRoot = path.resolve(here, '../src');
const evidenceRoot = path.resolve(here, '../test-artifacts/agent-7-browser');
const entryModule = 'app.js';

async function existingChromeDriver() {
  const candidates = [
    process.env.CHROMEDRIVER_PATH,
    process.env.CHROMEWEBDRIVER ? path.join(process.env.CHROMEWEBDRIVER, 'chromedriver') : null,
    '/usr/local/share/chromedriver-linux64/chromedriver',
    '/usr/bin/chromedriver',
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      await access(candidate);
      return candidate;
    } catch {}
  }
  return null;
}

function moduleSpecifiers(source) {
  const found = new Set();
  const patterns = [
    /(?:\bimport\s+(?:[^'\";]*?\s+from\s*)?|\bexport\s+[^'\";]*?\s+from\s*)(['\"])(\.{1,2}\/[^'\"]+)\1/g,
    /\bimport\s*\(\s*(['\"])(\.{1,2}\/[^'\"]+)\1\s*\)/g,
  ];
  for (const pattern of patterns) {
    for (const match of source.matchAll(pattern)) found.add(match[2]);
  }
  return [...found];
}

async function collectModuleGraph(entry = entryModule) {
  const graph = {};
  const visit = async (id) => {
    if (graph[id]) return;
    const absolute = path.resolve(sourceRoot, id);
    assert.ok(absolute.startsWith(`${sourceRoot}${path.sep}`), `Module escaped Desktop source root: ${id}`);
    const source = await readFile(absolute, 'utf8');
    const deps = {};
    graph[id] = { source, deps };
    for (const specifier of moduleSpecifiers(source)) {
      const dependency = path.resolve(path.dirname(absolute), specifier);
      assert.ok(dependency.startsWith(`${sourceRoot}${path.sep}`), `Import escaped Desktop source root: ${specifier}`);
      const dependencyId = path.relative(sourceRoot, dependency).split(path.sep).join('/');
      deps[specifier] = dependencyId;
      await visit(dependencyId);
    }
  };
  await visit(entry);
  return graph;
}

async function instrumentedDocument() {
  const [html, mainCss, playerCss, runtimeCss] = await Promise.all([
    readFile(path.join(sourceRoot, 'index.html'), 'utf8'),
    readFile(path.join(sourceRoot, 'style.css'), 'utf8'),
    readFile(path.join(sourceRoot, 'player-experience.css'), 'utf8'),
    readFile(path.join(sourceRoot, 'runtime-wizard.css'), 'utf8'),
  ]);
  const moduleMarker = '<script type="module" src="./app.js"></script>';
  const styleMarker = '<link rel="stylesheet" href="./style.css" />';
  assert.ok(html.includes(moduleMarker), 'index.html app module marker changed; browser smoke harness must be reviewed');
  assert.ok(html.includes(styleMarker), 'index.html stylesheet marker changed; browser smoke harness must be reviewed');

  const preloadedStyles = [
    `<style data-agent7-production-style>${mainCss}</style>`,
    `<style data-agent7-player-experience-style>${playerCss}</style>`,
    `<style data-agent7-runtime-wizard-style>${runtimeCss}</style>`,
    '<link rel="stylesheet" data-player-experience href="data:text/css," />',
    '<link rel="stylesheet" data-runtime-wizard-style href="data:text/css," />',
  ].join('\n');
  return html.replace(styleMarker, preloadedStyles).replace(moduleMarker, '');
}

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function webdriverRequest(port, pathname, { method = 'GET', body } = {}) {
  const response = await fetch(`http://127.0.0.1:${port}${pathname}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const payload = await response.json();
  if (!response.ok || payload?.value?.error) {
    throw new Error(`WebDriver ${method} ${pathname} failed: ${JSON.stringify(payload)}`);
  }
  return payload.value;
}

async function startDriver(executable) {
  const port = await freePort();
  const child = spawn(executable, [`--port=${port}`, '--allowed-ips=127.0.0.1'], { stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });

  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`ChromeDriver exited during startup (${child.exitCode}):\n${output}`);
    try {
      const status = await webdriverRequest(port, '/status');
      if (status?.ready !== false) return { child, port, output: () => output };
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  child.kill('SIGKILL');
  throw new Error(`ChromeDriver did not become ready:\n${output}`);
}

async function createSession(port) {
  const value = await webdriverRequest(port, '/session', {
    method: 'POST',
    body: {
      capabilities: {
        alwaysMatch: {
          browserName: 'chrome',
          'goog:chromeOptions': {
            args: ['--headless=new', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--hide-scrollbars'],
          },
        },
      },
    },
  });
  const sessionId = value?.sessionId;
  assert.ok(sessionId, `ChromeDriver did not create a session: ${JSON.stringify(value)}`);
  await webdriverRequest(port, `/session/${sessionId}/timeouts`, {
    method: 'POST',
    body: { script: 20000, pageLoad: 20000, implicit: 0 },
  });
  return sessionId;
}

async function execute(port, sessionId, script, args = []) {
  return webdriverRequest(port, `/session/${sessionId}/execute/sync`, {
    method: 'POST',
    body: { script, args },
  });
}

async function executeAsync(port, sessionId, script, args = []) {
  return webdriverRequest(port, `/session/${sessionId}/execute/async`, {
    method: 'POST',
    body: { script, args },
  });
}

async function setViewport(port, sessionId, width, height) {
  await webdriverRequest(port, `/session/${sessionId}/window/rect`, {
    method: 'POST',
    body: { x: 0, y: 0, width, height },
  });
  let metrics = await execute(port, sessionId, 'return { innerWidth, innerHeight, outerWidth, outerHeight };');
  const chromeWidth = Math.max(0, metrics.outerWidth - metrics.innerWidth);
  const chromeHeight = Math.max(0, metrics.outerHeight - metrics.innerHeight);
  if (metrics.innerWidth !== width || metrics.innerHeight !== height) {
    await webdriverRequest(port, `/session/${sessionId}/window/rect`, {
      method: 'POST',
      body: { x: 0, y: 0, width: width + chromeWidth, height: height + chromeHeight },
    });
    metrics = await execute(port, sessionId, 'return { innerWidth, innerHeight, outerWidth, outerHeight };');
  }
  assert.deepEqual(
    { width: metrics.innerWidth, height: metrics.innerHeight },
    { width, height },
    `ChromeDriver could not establish requested viewport; outer=${metrics.outerWidth}x${metrics.outerHeight}`,
  );
}

const tauriFixture = String.raw`
window.__swarmcraftBrowserErrors = [];
window.__swarmcraftCalls = [];
window.__swarmcraftWorldModFailures = 1;
addEventListener('error', (event) => {
  window.__swarmcraftBrowserErrors.push(String(event.error?.stack || event.message || event.error || 'window error'));
});
addEventListener('unhandledrejection', (event) => {
  window.__swarmcraftBrowserErrors.push(String(event.reason?.stack || event.reason || 'unhandled rejection'));
});
const record = (command, args) => window.__swarmcraftCalls.push({ command, args: args ? JSON.parse(JSON.stringify(args)) : null });
window.__TAURI__ = { core: { invoke: async (command, args = {}) => {
  record(command, args);
  switch (command) {
    case 'minecraft_versions':
      if (typeof args.includeSnapshots !== 'boolean' || typeof args.refresh !== 'boolean') throw new Error('minecraft_versions requires includeSnapshots and refresh booleans');
      return { origin: 'network', versions: [{ id: '26.1.2', type: 'release', release_time: '2026-01-01T00:00:00Z', supported: true }] };
    case 'fabric_loader_versions':
      if (!args.minecraftVersion || typeof args.refresh !== 'boolean') throw new Error('fabric_loader_versions requires minecraftVersion and refresh');
      return { origin: 'network', versions: [{ version: '0.19.3', stable: true, minecraft_version: args.minecraftVersion }] };
    case 'list_worlds': return [];
    case 'identity':
    case 'node_identity': return { peerId: '12D3KooWBrowserSmoke' };
    case 'migration_capabilities': return { transfer: false, wake: false, reason: 'Browser smoke fixture' };
    case 'host_readiness': return { eligible: false, reason: 'No selected world in browser smoke fixture' };
    case 'connectivity_status': return { state: 'nat_status_unknown' };
    case 'modrinth_search': return { items: [{ project_id: 'lithium', title: 'Lithium' }] };
    case 'modrinth_resolve_project': return { versions: [{
      project_id: 'lithium', version_id: 'lithium-1', display_name: 'Lithium', dependencies: [],
      files: [{ primary: true, filename: 'lithium.jar', locator: { url: 'https://cdn.modrinth.com/lithium.jar' }, retrieval: { state: 'provider_download' }, hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' } }],
    }] };
    case 'provider_staging_dir': return '/tmp/swarmcraft-provider-smoke';
    case 'modrinth_download': return { path: '/tmp/swarmcraft-provider-smoke/lithium.jar', filename: 'lithium.jar', size: 123, hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' } };
    case 'inspect_mod_artifact': return { mod_id: 'lithium', version: '1.0.0', environment: 'server' };
    case 'create_canonical_world': return { worldId: 'scworld:browser-smoke', canonical: { compatibilityFingerprint: 'browser-fingerprint' } };
    case 'world_mods_add':
      if (window.__swarmcraftWorldModFailures > 0) { window.__swarmcraftWorldModFailures -= 1; throw new Error('simulated local mod installation failure'); }
      return { status: 'ok' };
    case 'discovery_search': return { results: [] };
    case 'discovery_resolve': return { state: 'found', world: { name: 'Browser Public World', visibility: 'public' } };
    default: return null;
  }
} } };
`;

const moduleLoaderScript = String.raw`
const done = arguments[arguments.length - 1];
const modules = arguments[0];
const entry = arguments[1];
(async () => {
  const urls = new Map();
  const building = new Set();
  const replaceSpecifier = (source, specifier, url) => source
    .split("'" + specifier + "'").join("'" + url + "'")
    .split('"' + specifier + '"').join('"' + url + '"');
  const build = async (id) => {
    if (urls.has(id)) return urls.get(id);
    if (building.has(id)) throw new Error('Desktop module graph contains unsupported static import cycle at ' + id);
    const module = modules[id];
    if (!module) throw new Error('Missing Desktop module source for ' + id);
    building.add(id);
    let source = module.source;
    for (const [specifier, dependencyId] of Object.entries(module.deps)) {
      source = replaceSpecifier(source, specifier, await build(dependencyId));
    }
    const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
    urls.set(id, url);
    building.delete(id);
    return url;
  };
  await import(await build(entry));
  window.__agent7ModuleUrls = [...urls.values()];
  return { modules: urls.size };
})().then((value) => done({ ok: true, value }), (error) => done({ ok: false, error: String(error?.stack || error) }));
`;

const scenario = String.raw`(async () => {
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const waitFor = async (predicate, label, timeoutMs = 6000) => {
    const started = performance.now();
    while (performance.now() - started < timeoutMs) {
      if (predicate()) return;
      await wait(25);
    }
    throw new Error('Timed out waiting for ' + label);
  };
  const require = (condition, message) => { if (!condition) throw new Error(message); };
  const focusable = (id) => {
    const element = document.getElementById(id);
    require(element, 'Missing keyboard control #' + id);
    const hidden = [];
    for (let current = element; current; current = current.parentElement) {
      if (current.hidden) { hidden.push(current); current.hidden = false; }
    }
    element.disabled = false;
    element.focus();
    require(document.activeElement === element, '#' + id + ' could not receive focus');
    for (const ancestor of hidden) ancestor.hidden = true;
  };

  await waitFor(() => document.getElementById('launcherMods'), 'Mods UI');
  require(document.getElementById('publicWorldDiscovery'), 'Public discovery UI did not install');
  require(document.getElementById('launcherMods').closest('#createForm'), 'Mods UI was not inserted into Create form');
  await waitFor(
    () => document.getElementById('importMinecraft')?.tagName === 'SELECT' && document.getElementById('importLoader')?.tagName === 'SELECT',
    'Import catalog hydration',
  );

  const minecraftCalls = window.__swarmcraftCalls.filter((call) => call.command === 'minecraft_versions');
  const fabricCalls = window.__swarmcraftCalls.filter((call) => call.command === 'fabric_loader_versions');
  require(minecraftCalls.length >= 2, 'Expected Create and Import Minecraft catalog calls');
  require(minecraftCalls.every((call) => typeof call.args?.includeSnapshots === 'boolean' && typeof call.args?.refresh === 'boolean'), 'A minecraft_versions call omitted required boolean payloads');
  require(fabricCalls.every((call) => call.args?.minecraftVersion && typeof call.args?.refresh === 'boolean'), 'A fabric_loader_versions call omitted required payloads');
  require(document.getElementById('createMinecraft').value === '26.1.2', 'Create did not select shipped runtime Minecraft line');
  require(document.getElementById('createLoader').value === '0.19.3', 'Create did not select minimum shipped Fabric loader');

  document.getElementById('joinWorldId').value = 'scworld:browser-public';
  document.getElementById('joinWorldIdButton').click();
  await waitFor(() => document.getElementById('joinWorldIdNotice').textContent.includes('Browser Public World'), 'local World ID feedback');
  require(!document.getElementById('joinWorldIdNotice').hidden, 'Local World ID feedback stayed hidden');
  require(!document.getElementById('publicWorldStatus').textContent.includes('Browser Public World'), 'Exact World ID feedback leaked into Public Worlds status');

  document.getElementById('openCreate').click();
  document.getElementById('modSearch').value = 'Lithium';
  document.getElementById('modSearchButton').click();
  await waitFor(() => document.querySelector('#modSearchResults button'), 'provider search result');
  document.querySelector('#modSearchResults button').click();
  document.getElementById('createName').value = 'Browser smoke world';
  document.getElementById('createForm').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await waitFor(() => document.getElementById('createError')?.dataset.createdWorldId === 'scworld:browser-smoke', 'post-create repair state');

  require(window.__swarmcraftCalls.filter((call) => call.command === 'create_canonical_world').length === 1, 'Canonical Create was not invoked exactly once');
  require(window.__swarmcraftCalls.filter((call) => call.command === 'create_world').length === 0, 'Legacy create_world handler still owned intended Create path');
  require(document.getElementById('createWorld').disabled, 'Create submit was re-enabled after canonical world already existed');
  require(document.getElementById('createRepairRetry'), 'Local mod repair action was not offered');
  require(document.getElementById('createError').textContent.includes('was created canonically'), 'Partial-success copy did not preserve canonical creation success');
  require(document.querySelector('#sleepDialog .dialog-card > div:first-child p').textContent.includes('durable sleeping state'), 'Stop dialog did not expose durable sleeping semantics');

  for (const id of [
    'createName', 'createVisibility', 'createMinecraft', 'createLoader', 'modProvider', 'modSearch', 'modSearchButton', 'createWorld',
    'joinInvite', 'joinWorld', 'joinWorldId', 'joinWorldIdButton', 'eula', 'host', 'inviteWorld', 'transferHost', 'sleepWorld', 'leaveWorld',
  ]) focusable(id);

  require(window.__swarmcraftBrowserErrors.length === 0, 'Uncaught browser errors: ' + window.__swarmcraftBrowserErrors.join(' | '));
  require(document.body.scrollWidth <= innerWidth, 'Page has horizontal overflow at ' + innerWidth + 'px');
  document.getElementById('createError').scrollIntoView({ block: 'center' });
  return { status: 'pass', width: innerWidth, height: innerHeight, calls: window.__swarmcraftCalls.length };
})()`;

const scenarioRunner = String.raw`
const done = arguments[arguments.length - 1];
Promise.resolve().then(() => eval(arguments[0])).then(
  (value) => done({ ok: true, value }),
  (error) => done({ ok: false, error: String(error?.stack || error), browserErrors: window.__swarmcraftBrowserErrors || [] }),
);
`;

function pngDimensions(buffer) {
  assert.equal(buffer.toString('ascii', 1, 4), 'PNG', 'Expected PNG screenshot');
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

async function runViewport(port, sessionId, documentHtml, graph, width, height, screenshotPath) {
  await setViewport(port, sessionId, width, height);
  await execute(port, sessionId, 'document.open(); document.write(arguments[0]); document.close(); return true;', [documentHtml]);
  await execute(port, sessionId, 'eval(arguments[0]); return true;', [tauriFixture]);

  const loaded = await executeAsync(port, sessionId, moduleLoaderScript, [graph, entryModule]);
  assert.equal(loaded?.ok, true, `Desktop module graph failed to import: ${loaded?.error || JSON.stringify(loaded)}`);
  assert.ok(loaded.value?.modules >= 2, 'Browser smoke did not load Desktop module graph');

  const result = await executeAsync(port, sessionId, scenarioRunner, [scenario]);
  assert.equal(
    result?.ok,
    true,
    `Browser scenario threw at ${width}x${height}: ${result?.error || JSON.stringify(result)}\nBrowser errors: ${(result?.browserErrors || []).join(' | ')}`,
  );
  assert.equal(result.value?.status, 'pass', `Browser smoke failed at ${width}x${height}: ${JSON.stringify(result.value)}`);
  assert.deepEqual({ width: result.value.width, height: result.value.height }, { width, height });

  const screenshotBase64 = await webdriverRequest(port, `/session/${sessionId}/screenshot`);
  assert.ok(screenshotBase64, 'ChromeDriver returned empty screenshot');
  await writeFile(screenshotPath, Buffer.from(screenshotBase64, 'base64'));
}

test('current Desktop module graph initializes, owns canonical Create, and renders supported window sizes', async () => {
  const driver = await existingChromeDriver();
  assert.ok(driver, 'ChromeDriver is required for Agent 7 real-browser smoke gate on Linux CI');
  await mkdir(evidenceRoot, { recursive: true });
  const [documentHtml, graph] = await Promise.all([instrumentedDocument(), collectModuleGraph()]);
  const running = await startDriver(driver);
  let sessionId;
  try {
    sessionId = await createSession(running.port);
    for (const [width, height] of [[980, 760], [720, 560], [1280, 900]]) {
      const screenshot = path.join(evidenceRoot, `${width}x${height}.png`);
      await runViewport(running.port, sessionId, documentHtml, graph, width, height, screenshot);
      assert.deepEqual(pngDimensions(await readFile(screenshot)), { width, height });
    }
  } finally {
    if (sessionId) {
      try { await webdriverRequest(running.port, `/session/${sessionId}`, { method: 'DELETE' }); } catch {}
    }
    running.child.kill('SIGTERM');
    await Promise.race([
      new Promise((resolve) => running.child.once('exit', resolve)),
      new Promise((resolve) => setTimeout(resolve, 1500)),
    ]);
    if (running.child.exitCode === null) running.child.kill('SIGKILL');
  }
});
