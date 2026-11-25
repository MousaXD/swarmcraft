import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import net from 'node:net';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const sourceRoot = path.resolve(here, '../src');
const evidenceRoot = process.env.SWARMCRAFT_BROWSER_ARTIFACTS || path.resolve(here, '../test-artifacts/agent-7-browser');
const entryModule = 'app.js';
const candidateRoot = path.resolve(here, '../frontend/dist');
const candidateMode = process.env.SWARMCRAFT_BROWSER_VARIANT === 'candidate';

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
  // This Chrome fixture injects its own module/bridge and intentionally does
  // not load the packaged-app bootstrap watchdog. Native CI covers that path.
  const bootstrapMarker = '<script src="./bootstrap.js" defer></script>';
  const styleMarker = '<link rel="stylesheet" href="./style.css" />';
  assert.ok(html.includes(moduleMarker), 'index.html app module marker changed; browser smoke harness must be reviewed');
  assert.ok(html.includes(styleMarker), 'index.html stylesheet marker changed; browser smoke harness must be reviewed');
  assert.ok(html.includes(bootstrapMarker), 'index.html bootstrap marker changed; browser fixture must be reviewed');

  const preloadedStyles = [
    `<style data-agent7-production-style>${mainCss}</style>`,
    `<style data-agent7-player-experience-style>${playerCss}</style>`,
    `<style data-agent7-runtime-wizard-style>${runtimeCss}</style>`,
    '<link rel="stylesheet" data-player-experience href="data:text/css," />',
    '<link rel="stylesheet" data-runtime-wizard-style href="data:text/css," />',
  ].join('\n');
  return html.replace(styleMarker, preloadedStyles).replace(moduleMarker, '').replace(bootstrapMarker, '');
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
  if (typeof port === 'object') return port.request(pathname, body);
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

// Explicit local alternative when Chrome is installed but a matching
// ChromeDriver is not. The default CI/real WebDriver path remains unchanged.
// One temporary profile and one browser are reused for all viewport checks.
async function startLocalCdp() {
  const profile = await mkdtemp(path.join(os.tmpdir(), 'swarmcraft-local-cdp-'));
  const child = spawn(process.env.SWARMCRAFT_CHROME_BINARY || 'google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--hide-scrollbars',
    '--disable-background-networking', '--disable-component-update', '--no-first-run',
    '--remote-debugging-port=0', '--remote-debugging-address=127.0.0.1',
    '--user-data-dir=' + profile, 'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  let output = '';
  let socket;
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', chunk => { output = (output + chunk).slice(-4000); });
  const cleanup = async () => {
    socket?.close();
    if (child.exitCode === null) child.kill('SIGTERM');
    await Promise.race([new Promise(resolve => child.once('exit', resolve)), new Promise(resolve => setTimeout(resolve, 1500))]);
    if (child.exitCode === null) child.kill('SIGKILL');
    await rm(profile, { recursive: true, force: true, maxRetries: 3 });
  };
  try {
    let target;
    for (let attempt = 0; attempt < 120; attempt += 1) {
      if (child.exitCode !== null) throw new Error('Chrome exited: ' + output);
      try {
        const port = Number((await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]);
        const pages = await (await fetch('http://127.0.0.1:' + port + '/json/list')).json();
        target = pages.find(page => page.type === 'page');
        if (target?.webSocketDebuggerUrl) break;
      } catch {}
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    assert.ok(target?.webSocketDebuggerUrl, 'Chrome DevTools unavailable: ' + output);
    socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', reject, { once: true });
    });
    const pending = new Map();
    let next = 0;
    socket.addEventListener('message', event => {
      const message = JSON.parse(event.data);
      if (!pending.has(message.id)) return;
      const { resolve, reject, timeout } = pending.get(message.id);
      pending.delete(message.id);
      clearTimeout(timeout);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
    });
    const send = (method, params = {}) => new Promise((resolve, reject) => {
      const id = ++next;
      const timeout = setTimeout(() => { pending.delete(id); reject(new Error('CDP timeout: ' + method)); }, 30000);
      pending.set(id, { resolve, reject, timeout });
      socket.send(JSON.stringify({ id, method, params }));
    });
    await send('Page.enable');
    await send('Runtime.enable');
    const evaluate = async expression => {
      const result = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
      if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
      return result.result.value;
    };
    return {
      close: cleanup,
      async request(pathname, body) {
        if (pathname.endsWith('/url')) {
          const result = await send('Page.navigate', { url: body.url });
          assert.ok(!result.errorText, 'Candidate browser navigation failed');
          return null;
        }
        if (pathname.endsWith('/window/rect')) {
          return send('Emulation.setDeviceMetricsOverride', { width: body.width, height: body.height, deviceScaleFactor: 1, mobile: false });
        }
        if (pathname.endsWith('/execute/sync')) {
          return evaluate('(function(){' + body.script + '}).apply(null,' + JSON.stringify(body.args) + ')');
        }
        if (pathname.endsWith('/execute/async')) {
          return evaluate('new Promise(resolve => (function(){' + body.script + '}).apply(null,[...' + JSON.stringify(body.args) + ',resolve]))');
        }
        if (pathname.endsWith('/screenshot')) return (await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false })).data;
        throw new Error('Unsupported local CDP request: ' + pathname);
      },
    };
  } catch (error) {
    await cleanup();
    throw error;
  }
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
  let outerWidth = width;
  let outerHeight = height;
  let metrics;

  // ChromeDriver's window rect and inner viewport can settle at different times
  // under Linux CI load. Calibrate against the measured inner viewport, not a
  // possibly stale outer-minus-inner decoration estimate. Exact size is still
  // mandatory: never pass a responsive-layout check with the wrong viewport.
  for (let attempt = 0; attempt < 5; attempt += 1) {
    await webdriverRequest(port, `/session/${sessionId}/window/rect`, {
      method: 'POST',
      body: { x: 0, y: 0, width: outerWidth, height: outerHeight },
    });
    for (let settle = 0; settle < 5; settle += 1) {
      metrics = await execute(port, sessionId, 'return { innerWidth, innerHeight, outerWidth, outerHeight };');
      if (metrics.innerWidth === width && metrics.innerHeight === height) {
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 80));
    }
    outerWidth = Math.max(1, outerWidth + width - metrics.innerWidth);
    outerHeight = Math.max(1, outerHeight + height - metrics.innerHeight);
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
window.__swarmcraftRecoveryWorld = null;
window.__swarmcraftPendingMembership = null;
window.__swarmcraftInviteRevoked = false;
window.__swarmcraftImportPicker = '/tmp/QA Minecraft Save';
window.__swarmcraftImportBlocked = false;
window.__swarmcraftImportDelay = false;
window.__swarmcraftImportedWorld = null;
window.__swarmcraftClientDelay = false;
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
    case 'client_profile': {
      if (window.__swarmcraftClientDelay) await new Promise(resolve => setTimeout(resolve, 180));
      return JSON.stringify({
        world_id: args.world, compatibility_fingerprint: 'b'.repeat(64),
        minecraft_version: '26.1.2', fabric_loader_version: '0.19.3', minimum_java_major: 25,
        required_mods: [{ mod_id: 'sharedmod', version: '1.0', artifact_hash: 'c'.repeat(64) }],
        inventory: !args.modsDir ? 'not_checked' : args.modsDir === '/fixture/matching/mods' ? 'exact_match' : 'needs_review',
        issues: args.modsDir && args.modsDir !== '/fixture/matching/mods' ? [{ code: 'missing_required', mod_id: 'sharedmod' }] : [],
      });
    }
    case 'pick_import_folder': return window.__swarmcraftImportPicker;
    case 'preview_import_world': {
      if (window.__swarmcraftImportDelay) await new Promise(resolve => setTimeout(resolve, 180));
      if (window.__swarmcraftImportBlocked) return JSON.stringify({ state: 'blocked', blocker: 'source_not_quiescent' });
      return JSON.stringify({
        state: 'ready', minecraft_version: args.minecraft, fabric_loader_version: args.fabricLoader,
        minimum_java_major: 25, source_files: 12, source_bytes: 2097152,
        required_server_mods: [], compatibility_fingerprint: 'a'.repeat(64),
      });
    }
    case 'import_world':
      if (args.expectedCompatibility !== 'a'.repeat(64) || args.source !== '/tmp/QA Minecraft Save'
          || args.noServerMods !== true || Object.hasOwn(args, 'acceptEula')) throw new Error('Unexpected reviewed import');
      window.__swarmcraftImportedWorld = 'scworld:' + '1'.repeat(64);
      return JSON.stringify({ world_id: window.__swarmcraftImportedWorld });
    case 'minecraft_versions':
      if (typeof args.includeSnapshots !== 'boolean' || typeof args.refresh !== 'boolean') throw new Error('minecraft_versions requires includeSnapshots and refresh booleans');
      return { origin: 'network', versions: [
        { id: '26.3', type: 'release', release_time: '2026-10-01T00:00:00Z', supported: true, adapter_supported: false },
        { id: '26.1.2', type: 'release', release_time: '2026-01-01T00:00:00Z', supported: true, adapter_supported: true },
      ] };
    case 'fabric_loader_versions':
      if (!args.minecraftVersion || typeof args.refresh !== 'boolean') throw new Error('fabric_loader_versions requires minecraftVersion and refresh');
      return { origin: 'network', versions: [{ version: '0.19.3', stable: true, minecraft_version: args.minecraftVersion, adapter_supported: args.minecraftVersion === '26.1.2' }] };
    case 'list_worlds': return window.__swarmcraftImportedWorld
      ? window.__swarmcraftImportedWorld + '  Reviewed import  snapshots=1'
      : window.__swarmcraftRecoveryWorld
      ? window.__swarmcraftRecoveryWorld + '  Recovery source  snapshots=4'
      : 'No local worlds.';
    case 'game_route_status': return { world_id: args.world, state: 'unavailable', route_verified: false, diagnostic: 'peer_link_unavailable', recent_session: { state: 'stream_admission_failed', age_ms: 100 } };
    case 'world_status':
      if (args.world === window.__swarmcraftImportedWorld) {
        return ['World: Reviewed import', 'World ID: ' + args.world, 'Safety: Canonical', 'Latest snapshot: 1'].join('\n');
      }
      if (args.world === window.__swarmcraftRecoveryWorld) {
        return ['World: Recovery source', 'World ID: ' + args.world, 'Safety: Canonical',
          'Latest snapshot: 4', 'Checkpoint manifest: ' + 'c'.repeat(64),
          ...(window.__swarmcraftPendingMembership ? ['Membership: ' + window.__swarmcraftPendingMembership] : [])].join('\n');
      }
      return '';
    case 'recovery_review':
      return JSON.stringify({
        world_id: args.world, evidence: 'local_signed_history_not_live_authority',
        checkpoint_history: 'verified_history_files_checked_on_action', checkpoint_error: null,
        conflict_digest: '9'.repeat(64),
        checkpoints: [
          { checkpoint_hash: 'c'.repeat(64), snapshot_number: '4', epoch: '2', sequence: '4', state_root: '1'.repeat(64), signer: 'scpeer:' + 'd'.repeat(64), files: 12, latest: true },
          { checkpoint_hash: 'e'.repeat(64), snapshot_number: '3', epoch: '1', sequence: '3', state_root: '2'.repeat(64), signer: 'scpeer:' + 'd'.repeat(64), files: 8, latest: false },
        ],
        conflicts: [
          { branch_hash: 'f'.repeat(64), base_checkpoint: 'e'.repeat(64), head_checkpoint: '7'.repeat(64), head_epoch: '2', head_sequence: '4', state_root: '3'.repeat(64), signer: 'scpeer:' + 'a'.repeat(64), local_history: 'evidence_only' },
          { branch_hash: '8'.repeat(64), base_checkpoint: 'e'.repeat(64), head_checkpoint: 'c'.repeat(64), head_epoch: '2', head_sequence: '4', state_root: '1'.repeat(64), signer: 'scpeer:' + 'd'.repeat(64), local_history: 'matching_signed_manifests' },
        ],
      });
    case 'conflict_export':
      if (args.world !== window.__swarmcraftRecoveryWorld || args.branch !== 'f'.repeat(64)
          || args.acknowledgedConflicts !== '9'.repeat(64) || args.evidenceOnly !== true
          || args.destination !== '/tmp/swarmcraft-evidence') throw new Error('Unexpected branch evidence selection');
      return JSON.stringify({
        world_id: args.world, branch_hash: args.branch, checkpoint_hash: '7'.repeat(64),
        destination: args.destination, evidence: 'signed_branch_evidence_only', files_exported: false, manifest_count: 0,
      });
    case 'inspect_invite':
      return JSON.stringify({
        world_id: args.invite === 'scinvite:ffff' ? 'scworld:' + 'e'.repeat(64) : window.__swarmcraftRecoveryWorld,
        invite_id: 'c'.repeat(64), issuer_peer_id: 'scpeer:' + 'd'.repeat(64),
        expires_unix_ms: 1800000000000, reuse: 'reusable',
        locally_issued: true, status: window.__swarmcraftInviteRevoked ? 'revoked' : 'lifetime_valid',
      });
    case 'revoke_invite':
      if (args.world !== window.__swarmcraftRecoveryWorld || args.invite !== 'scinvite:abcd'
          || args.inviteId !== 'c'.repeat(64) || args.confirmRevocation !== true) throw new Error('Unexpected revocation target');
      window.__swarmcraftInviteRevoked = true;
      return JSON.stringify({ world_id: args.world, invite_id: args.inviteId, revoked: true, newly_revoked: true });
    case 'recover_copy':
      if (args.world !== window.__swarmcraftRecoveryWorld || args.checkpoint !== 'e'.repeat(64)
          || args.acknowledgedConflicts !== '9'.repeat(64)
          || args.confirmNewIdentity !== true) throw new Error('Unexpected recovery selection');
      return JSON.stringify({
        source_world_id: args.world, source_snapshot_hash: args.checkpoint, source_snapshot_number: 3,
        source_conflict_digest: args.acknowledgedConflicts,
        world_id: 'scworld:' + 'b'.repeat(64), snapshot_hash: 'd'.repeat(64),
        display_name: 'Recovered: Recovery source',
      });
    case 'initialize_node': return 'SwarmCraft initialized';
    case 'identity':
    case 'node_identity': return 'Peer ID: scpeer:' + 'd'.repeat(64);
    case 'ensure_daemon_running': return 12345;
    case 'hosting_contribution_get': return {
      world_id: args.world, consented: false, selected_heap_mib: 0, may_host_now: false,
      reason: 'Player only', available_mib: 8192, physical_mib: 16384,
      hard_process_memory_cap_guaranteed: false, checkpoint_state: 'no_pending_checkpoint',
    };
    case 'migration_capabilities': return { transfer: false, wake: false, reason: 'Browser smoke fixture' };
    case 'host_readiness': return { eligible: false, reason: 'No selected world in browser smoke fixture' };
    case 'connectivity_status': return { state: 'nat_status_unknown' };
    case 'modrinth_search': return { items: [{ project_id: 'lithium', title: 'Lithium' }] };
    case 'curseforge_search': return { status: 'ok', data: { projects: [{ project_id: '123', name: 'Restricted mod' }] } };
    case 'curseforge_resolve_project': return { status: 'ok', data: { packages: [{
      project_id: '123', file_id: '456', version_id: '456', file_name: 'restricted.jar', dependencies: [],
    }] } };
    case 'curseforge_download': return { status: 'manual_artifact_required', data: {
      project_id: '123', version_id: '456', file_name: 'restricted.jar', file_size: 42,
      hashes: [{ algorithm: 'sha1', digest: 'b'.repeat(40) }],
    } };
    case 'modrinth_resolve_project': return { versions: [{
      project_id: 'lithium', version_id: 'lithium-1', display_name: 'Lithium', dependencies: [],
      files: [{ primary: true, filename: 'lithium.jar', locator: { url: 'https://cdn.modrinth.com/lithium.jar' }, retrieval: { state: 'provider_download' }, hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' } }],
    }] };
    case 'provider_staging_dir': return '/tmp/swarmcraft-provider-smoke';
    case 'modrinth_download': return { path: '/tmp/swarmcraft-provider-smoke/lithium.jar', filename: 'lithium.jar', size: 123, hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' } };
    case 'inspect_mod_artifact': return args.path === '/fixture/restricted.jar'
      ? { mod_id: 'restricted', version: '1.0', environment: 'server' }
      : { mod_id: 'lithium', version: '1.0.0', environment: 'server' };
    case 'create_canonical_world': return { worldId: 'scworld:browser-smoke', canonical: { compatibilityFingerprint: 'browser-fingerprint' } };
    case 'world_mods_add':
      if (window.__swarmcraftWorldModFailures > 0) { window.__swarmcraftWorldModFailures -= 1; throw new Error('simulated local mod installation failure'); }
      return { status: 'ok' };
    case 'world_mods_acquire': return JSON.stringify({
      world_id: args.world, ready: false, inventory_issues: ['missing_required'],
      artifacts: [
        { mod_id: 'downloaded', version: '1', artifact_hash: 'a'.repeat(64), provider: 'modrinth',
          project_id: 'modrinth-id', version_id: 'exact-version', file_name: 'downloaded.jar', state: 'acquired' },
        { mod_id: 'restricted', version: '2', artifact_hash: 'b'.repeat(64), provider: 'curse_forge',
          project_id: '123', version_id: '456', file_name: 'restricted.jar', state: 'manual_required' },
      ],
    });
    case 'discovery_search': return { results: [] };
    case 'discovery_resolve': return { state: 'found', world: { world_id: args.world, name: 'Browser Public World', visibility: 'public', expires_unix_ms: Date.now() + 60_000 } };
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
    const closedDetails = [];
    for (let current = element; current; current = current.parentElement) {
      if (current.hidden) { hidden.push(current); current.hidden = false; }
      if (current.tagName === 'DETAILS' && !current.open) { closedDetails.push(current); current.open = true; }
    }
    element.disabled = false;
    element.focus();
    require(document.activeElement === element, '#' + id + ' could not receive focus');
    for (const details of closedDetails) details.open = false;
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
  require(!document.getElementById('copyDiscoveryDiagnosis').disabled, 'Safe directory diagnosis unavailable');
  document.getElementById('joinWorldId').value = 'scworld:changed-public';
  document.getElementById('joinWorldId').dispatchEvent(new Event('input', { bubbles: true }));
  await waitFor(() => document.getElementById('copyDiscoveryDiagnosis').disabled, 'Changed lookup invalidates stale diagnosis');
  require(!document.getElementById('publicWorldStatus').textContent.includes('Browser Public World'), 'Exact World ID feedback leaked into Public Worlds status');

  document.getElementById('openCreate').click();
  document.getElementById('modSearch').value = 'Lithium';
  document.getElementById('modSearchButton').click();
  await waitFor(() => document.querySelector('#modSearchResults button'), 'provider search result');
  document.querySelector('#modSearchResults button').click();
  document.getElementById('modProvider').value = 'curseforge';
  document.getElementById('modSearch').value = 'Restricted mod';
  document.getElementById('modSearchButton').click();
  await waitFor(() => document.getElementById('modSearchResults').textContent.includes('Restricted mod'), 'restricted provider search result');
  document.querySelector('#modSearchResults button').click();
  document.getElementById('createName').value = 'Browser smoke world';
  document.getElementById('createForm').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await waitFor(() => document.getElementById('manualProviderArtifact0') && document.getElementById('createError').textContent.includes('Manual artifact required'), 'exact manual file selection');
  require(!window.__swarmcraftCalls.some(call => call.command === 'create_canonical_world'), 'World created before restricted artifact was supplied');
  document.getElementById('manualProviderArtifact0').value = '/fixture/restricted.jar';
  document.getElementById('createForm').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await waitFor(() => document.getElementById('createError')?.dataset.createdWorldId === 'scworld:browser-smoke', 'post-create repair state');

  require(window.__swarmcraftCalls.filter((call) => call.command === 'create_canonical_world').length === 1, 'Canonical Create was not invoked exactly once');
  const createdPack = window.__swarmcraftCalls.find(call => call.command === 'create_canonical_world').args.request.modpack.packages;
  require(createdPack.length === 2, 'Both provider artifacts were not retained');
  require(createdPack.some(pkg => pkg.provider === 'curseforge' && pkg.retrieval === 'manual_required' && pkg.artifactPath === '/fixture/restricted.jar'), 'Manual restriction was lost during canonicalization');
  require(createdPack.some(pkg => pkg.provider === 'modrinth' && pkg.retrieval === 'provider_download'), 'Permitted provider artifact was not retained');
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

const recoveryScenario = String.raw`(async () => {
  const waitFor = async (predicate, label) => {
    for (let i = 0; i < 160; i += 1) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error('Timed out waiting for ' + label);
  };
  const require = (condition, message) => { if (!condition) throw new Error(message); };
  const source = 'scworld:' + 'a'.repeat(64);
  window.__swarmcraftRecoveryWorld = source;
  document.querySelector('[data-view="worlds"]').click();
  document.getElementById('refresh').click();
  await waitFor(() => document.getElementById('world').value === source, 'source selection');
  document.querySelector('[data-view="diagnostics"]').click();
  const button = document.getElementById('openRecoveryCopy');
  button.click();
  const dialog = document.getElementById('recoveryCopyDialog');
  await waitFor(() => dialog.open, 'confirmation dialog');
  require(document.getElementById('recoveryCopySource').textContent.includes(source), 'Original identity missing');
  require(document.getElementById('recoveryCopySource').textContent.includes('c'.repeat(64)), 'Exact checkpoint missing');
  require(!window.__swarmcraftCalls.some(call => call.command === 'recover_copy'), 'Copy dispatched without confirmation');
  document.getElementById('cancelRecoveryCopy').click();
  await waitFor(() => !dialog.open, 'cancel dialog');
  require(!window.__swarmcraftCalls.some(call => call.command === 'recover_copy'), 'Canceled recovery dispatched');
  button.click();
  await waitFor(() => dialog.open, 'reopen confirmation');
  const confirm = document.getElementById('confirmRecoveryCopy');
  const choice = document.getElementById('recoveryChoice');
  const ack = document.getElementById('recoveryConflictAck');
  require(choice.options.length === 4, 'Missing historical checkpoint or branch choice');
  choice.value = 'checkpoint:' + 'e'.repeat(64);
  choice.dispatchEvent(new Event('change', { bubbles: true }));
  require(document.getElementById('recoveryCopySource').textContent.includes('earlier checkpoint'), 'Earlier checkpoint not displayed');
  require(confirm.disabled, 'Conflict acknowledgement was bypassed');
  ack.click();
  require(!confirm.disabled, 'Explicit acknowledgement did not enable copy');
  choice.focus();
  require(document.activeElement === choice, 'Recovery choice is not keyboard focusable');
  ack.focus();
  require(document.activeElement === ack, 'Conflict acknowledgement is not keyboard focusable');
  confirm.focus();
  require(document.activeElement === confirm, 'Confirmation is not keyboard focusable');
  confirm.click();
  confirm.click();
  await waitFor(() => document.getElementById('cancelRecoveryCopy').textContent === 'Done',
    'copy receipt (state: ' + document.getElementById('recoveryCopyResult').textContent
      + ', dispatched: ' + window.__swarmcraftCalls.filter(call => call.command === 'recover_copy').length + ')');
  require(window.__swarmcraftCalls.filter(call => call.command === 'recover_copy').length === 1, 'Recovery was not dispatched exactly once');
  const result = document.getElementById('recoveryCopyResult');
  require(result.textContent.includes(source) && result.textContent.includes('scworld:' + 'b'.repeat(64)), 'Receipt lacks both world IDs');
  require(result.dataset.tone === 'safe', 'Confirmed receipt did not show success');
  const bounds = dialog.getBoundingClientRect();
  require(bounds.left >= 0 && bounds.top >= 0 && bounds.right <= innerWidth && bounds.bottom <= innerHeight, 'Recovery dialog exceeds viewport');
  require(dialog.scrollWidth <= dialog.clientWidth + 2, 'Recovery dialog has horizontal overflow');
  const done = document.getElementById('cancelRecoveryCopy').getBoundingClientRect();
  require(done.top >= bounds.top && done.bottom <= bounds.bottom, 'Recovery Done control clipped');
  require(window.__swarmcraftBrowserErrors.length === 0, 'Recovery browser errors: ' + window.__swarmcraftBrowserErrors.join(' | '));
  return { status: 'pass' };
})()`;

const selectionScenario = String.raw`(async () => {
  const $ = id => document.getElementById(id);
  const waitFor = async predicate => {
    for (let i = 0; i < 160; i += 1) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error('Timed out opening recovery review');
  };
  $('cancelRecoveryCopy').click();
  await waitFor(() => !$('recoveryCopyDialog').open);
  $('openRecoveryCopy').click();
  await waitFor(() => $('recoveryCopyDialog').open);
  const dialog = $('recoveryCopyDialog').getBoundingClientRect();
  for (const id of ['cancelRecoveryCopy', 'confirmRecoveryCopy', 'exportRecoveryChoice']) {
    const bounds = $(id).getBoundingClientRect();
    if (bounds.top < dialog.top || bounds.bottom > dialog.bottom || bounds.left < dialog.left || bounds.right > dialog.right) {
      throw new Error('Recovery action clipped: ' + id);
    }
  }
  return { status: 'pass' };
})()`;

const branchScenario = String.raw`(async () => {
  const waitFor = async (predicate, label) => {
    for (let i = 0; i < 160; i += 1) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error('Timed out waiting for ' + label);
  };
  const require = (condition, message) => { if (!condition) throw new Error(message); };
  const $ = id => document.getElementById(id);
  $('cancelRecoveryCopy').click();
  await waitFor(() => !$('recoveryCopyDialog').open, 'copy dialog close');
  $('openRecoveryCopy').click();
  await waitFor(() => $('recoveryCopyDialog').open, 'branch review');
  $('recoveryChoice').value = 'branch:' + 'f'.repeat(64);
  $('recoveryChoice').dispatchEvent(new Event('change', { bubbles: true }));
  require($('confirmRecoveryCopy').hidden && $('exportRecoveryChoice').hidden, 'Missing branch files offered as available');
  require($('recoveryCopySource').textContent.includes('Evidence only here'), 'Evidence-only warning missing');
  $('recoveryExportDestination').value = '/tmp/swarmcraft-evidence';
  $('recoveryExportDestination').dispatchEvent(new Event('input', { bubbles: true }));
  require($('exportRecoveryEvidence').disabled, 'Missing exact conflict acknowledgement');
  $('recoveryConflictAck').click();
  require(!$('exportRecoveryEvidence').disabled, 'Evidence export unavailable after confirmation');
  $('exportRecoveryEvidence').focus();
  require(document.activeElement === $('exportRecoveryEvidence'), 'Evidence export not keyboard focusable');
  $('exportRecoveryEvidence').click();
  $('exportRecoveryEvidence').click();
  await waitFor(() => $('cancelRecoveryCopy').textContent === 'Done', 'evidence receipt');
  require(window.__swarmcraftCalls.filter(call => call.command === 'conflict_export').length === 1, 'Duplicate evidence export');
  require($('recoveryCopyResult').textContent.includes('No world files were exported.'), 'Evidence-only receipt misrepresents files');
  const bounds = $('recoveryCopyDialog').getBoundingClientRect();
  require(bounds.left >= 0 && bounds.top >= 0 && bounds.right <= innerWidth && bounds.bottom <= innerHeight, 'Branch dialog exceeds viewport');
  require($('recoveryCopyDialog').scrollWidth <= $('recoveryCopyDialog').clientWidth + 2, 'Branch dialog has horizontal overflow');
  require(window.__swarmcraftBrowserErrors.length === 0, 'Branch browser errors: ' + window.__swarmcraftBrowserErrors.join(' | '));
  return { status: 'pass' };
})()`;

const inviteScenario = String.raw`(async () => {
  const waitFor = async (predicate, label) => {
    for (let i = 0; i < 160; i += 1) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error('Timed out waiting for ' + label);
  };
  const require = (condition, message) => { if (!condition) throw new Error(message); };
  const $ = id => document.getElementById(id);
  $('cancelRecoveryCopy').click();
  await waitFor(() => !$('recoveryCopyDialog').open, 'recovery close');
  document.querySelector('[data-view="worlds"]').click();
  $('manageInvites').click();
  const dialog = $('inviteManagementDialog');
  await waitFor(() => dialog.open, 'invite management');
  const changeToken = value => {
    $('manageInviteToken').value = value;
    $('manageInviteToken').dispatchEvent(new Event('input', { bubbles: true }));
  };
  changeToken('scinvite:ffff');
  $('inspectInvite').click();
  await waitFor(() => !$('inviteManagementNotice').hidden, 'wrong world refusal');
  require($('revokeInvite').disabled, 'Wrong-world receipt enabled revocation');
  changeToken('scinvite:abcd');
  $('inspectInvite').click();
  await waitFor(() => !$('inviteInspection').hidden, 'verified inspection');
  require($('inviteInspection').textContent.includes('reusable bearer grant'), 'Reuse policy not visible');
  require($('inviteInspection').textContent.includes('c'.repeat(64)), 'Signed grant hash missing');
  require($('revokeInvite').disabled, 'Revocation enabled without explicit confirmation');
  $('revokeInviteConfirm').click();
  require(!$('revokeInvite').disabled, 'Explicit confirmation did not enable revoke');
  changeToken('scinvite:beef');
  require($('revokeInvite').disabled, 'Changed token retained stale confirmation');
  require(!window.__swarmcraftCalls.some(call => call.command === 'revoke_invite'), 'Revocation dispatched before confirmation');
  changeToken('scinvite:abcd');
  $('inspectInvite').click();
  await waitFor(() => !$('inviteInspection').hidden, 're-inspection');
  $('revokeInviteConfirm').click();
  const bounds = dialog.getBoundingClientRect();
  require(bounds.left >= 0 && bounds.top >= 0 && bounds.right <= innerWidth && bounds.bottom <= innerHeight, 'Invite dialog exceeds viewport');
  require(dialog.scrollWidth <= dialog.clientWidth + 2, 'Invite dialog has horizontal overflow');
  $('revokeInvite').click();
  $('revokeInvite').click();
  await waitFor(() => $('inviteManagementNotice').dataset.tone === 'safe', 'confirmed revocation');
  require(window.__swarmcraftCalls.filter(call => call.command === 'revoke_invite').length === 1, 'Revocation dispatched more than once');
  require($('inviteManagementNotice').textContent.includes('retain membership'), 'Membership retention boundary absent');
  require(!$('output').textContent.includes('scinvite:abcd'), 'Bearer token leaked to Activity');
  $('closeInviteManagement').click();
  await waitFor(() => !dialog.open, 'invite dialog close');
  window.__swarmcraftPendingMembership = 'pending; issuer reported invitation revoked';
  $('refresh').click();
  await waitFor(() => $('membershipNotice').textContent.includes('revoked'), 'pending admission notice');
  require($('playWorld').disabled, 'Pending invitation became playable');
  require(!$('refreshMembership').hidden, 'Missing membership refresh control');
  window.__swarmcraftPendingMembership = 'committed admission; local synchronization pending';
  $('refreshMembership').click();
  await waitFor(() => $('membershipNotice').textContent.includes('Membership approved'), 'synchronization pending notice');
  require($('playWorld').disabled, 'Unverified sync became playable');
  document.querySelector('[data-view="diagnostics"]').click();
  await waitFor(() => $('routeDiagnosis').textContent.includes('Host peer link unavailable'), 'typed peer-link diagnosis');
  require($('routeDiagnosis').textContent.includes('stream admission'), 'Actual stream failure stage missing');
  require(!$('copyRouteDiagnosis').disabled, 'Safe route diagnosis unavailable');
  const count = window.__swarmcraftCalls.filter(call => call.command === 'game_route_select').length;
  $('retryRouteDiagnosis').click();
  await waitFor(() => window.__swarmcraftCalls.filter(call => call.command === 'game_route_select').length > count, 'retry selected gateway');
  require($('playWorld').disabled, 'Failed route enabled play');
  require(window.__swarmcraftBrowserErrors.length === 0, 'Invite browser errors: ' + window.__swarmcraftBrowserErrors.join(' | '));
  return { status: 'pass' };
})()`;

const importScenario = String.raw`(async () => {
  const $ = id => document.getElementById(id);
  const require = (condition, message) => { if (!condition) throw new Error(message); };
  const waitFor = async (predicate, label) => {
    for (let i = 0; i < 200; i += 1) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error('Timed out waiting for ' + label);
  };
  const edit = (id, value) => {
    $(id).value = value;
    $(id).dispatchEvent(new Event('input', { bubbles: true }));
  };
  const calls = name => window.__swarmcraftCalls.filter(call => call.command === name);
  $('openImport').click();
  await waitFor(() => !$('importPanel').hidden, 'import view');
  $('importForm').reset();
  require($('importWorld').disabled, 'Unreviewed import enabled');
  $('pickImportFolder').click();
  await waitFor(() => $('importSource').value === '/tmp/QA Minecraft Save', 'native folder selection result');
  window.__swarmcraftImportPicker = null;
  $('pickImportFolder').click();
  await waitFor(() => !$('pickImportFolder').disabled, 'picker cancellation');
  require($('importSource').value === '/tmp/QA Minecraft Save', 'Cancel discarded the source');
  edit('importName', 'Reviewed import');
  edit('importMinecraft', '26.1.2');
  edit('importLoader', '0.19.3');
  $('importNoServerMods').click();
  require($('importNoServerMods').checked, 'No-mod declaration missing');
  window.__swarmcraftImportBlocked = true;
  $('checkImport').click();
  await waitFor(() => $('importReview').dataset.state === 'blocked', 'source lock blocker');
  require($('importWorld').disabled, 'Blocked source enabled import');
  window.__swarmcraftImportBlocked = false;
  window.__swarmcraftImportDelay = true;
  $('checkImport').click();
  edit('importName', 'Changed during review');
  await waitFor(() => !$('checkImport').disabled, 'stale review completion');
  require($('importWorld').disabled, 'Stale preview enabled commit');
  window.__swarmcraftImportDelay = false;
  edit('importName', 'Reviewed import');
  $('checkImport').click();
  await waitFor(() => !$('importWorld').disabled, 'valid native review');
  require($('importReview').textContent.includes('Java 25'), 'Runtime requirements absent');
  require($('importReview').textContent.includes('EULA'), 'EULA boundary absent');
  edit('importVisibility', 'unlisted');
  require($('importWorld').disabled, 'Changed visibility retained review');
  edit('importVisibility', 'private');
  $('checkImport').click();
  await waitFor(() => !$('importWorld').disabled, 'fresh review');
  require(calls('import_world').length === 0, 'Review committed a world');
  $('importWorld').click();
  $('importWorld').click();
  await waitFor(() => $('importPanel').hidden, 'reviewed import completion');
  require(calls('import_world').length === 1, 'Import executed more than once');
  require(calls('import_world')[0].args.expectedCompatibility === 'a'.repeat(64), 'Reviewed fingerprint missing');
  require(!calls('runtime_install').length && !calls('hosting_contribution_set').length, 'Import changed runtime or consent');
  require(window.__swarmcraftBrowserErrors.length === 0, 'Import browser errors: ' + window.__swarmcraftBrowserErrors.join(' | '));
  return { status: 'pass' };
})()`;

const clientProfileScenario = String.raw`(async () => {
  const $ = id => document.getElementById(id);
  const require = (condition, message) => { if (!condition) throw new Error(message); };
  const waitFor = async (predicate, label) => {
    for (let i = 0; i < 200; i += 1) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error('Timed out waiting for ' + label);
  };
  const edit = value => {
    $('clientModsFolder').value = value;
    $('clientModsFolder').dispatchEvent(new Event('input', { bubbles: true }));
  };
  const before = window.__swarmcraftCalls.length;
  await waitFor(() => !$('selectionContent').hidden && !$('checkClientProfile').disabled, 'selected imported world');
  $('clientProfilePanel').querySelector('summary').click();
  await waitFor(() => !$('clientProfileRequirements').hidden, 'signed requirements');
  require($('clientProfileRequirements').textContent.includes('26.1.2'), 'Minecraft requirement missing');
  require($('clientProfileRequirements').textContent.includes('0.19.3'), 'Fabric requirement missing');
  require($('clientProfileRequirements').textContent.includes('sharedmod'), 'Client mod missing');
  require($('clientInventoryStatus').textContent.includes('not checked'), 'Unchecked folder represented as verified');
  $('clientLauncher').value = 'prism';
  $('clientLauncher').dispatchEvent(new Event('change', { bubbles: true }));
  require($('clientLauncherInstructions').textContent.includes('Prism Launcher'), 'Launcher instructions did not change');
  edit('/fixture/missing/mods');
  $('checkClientProfile').click();
  await waitFor(() => $('clientInventoryStatus').dataset.state === 'warning', 'missing mod diagnosis');
  require($('clientInventoryStatus').textContent.includes('Required client mod is missing'), 'Missing mod not reported');
  edit('/fixture/matching/mods');
  window.__swarmcraftClientDelay = true;
  $('checkClientProfile').click();
  edit('/fixture/changed/mods');
  await new Promise(resolve => setTimeout(resolve, 260));
  require($('clientInventoryStatus').dataset.state !== 'checked', 'Stale folder reply enabled success');
  window.__swarmcraftClientDelay = false;
  edit('/fixture/matching/mods');
  $('checkClientProfile').click();
  await waitFor(() => $('clientInventoryStatus').dataset.state === 'checked', 'matching client JAR inventory');
  require($('clientInventoryStatus').textContent.includes('login have not been verified'), 'Client inventory confused with runtime or login');
  const calls = window.__swarmcraftCalls.slice(before).filter(call => call.command === 'client_profile');
  require(calls.length >= 4, 'Expected native profile reads');
  require(calls.every(call => call.args.world === window.__swarmcraftImportedWorld), 'Profile read for wrong world');
  require(!window.__swarmcraftCalls.slice(before).some(call => ['runtime_install', 'runtime_repair', 'hosting_contribution_set', 'launch_client'].includes(call.command)), 'Client inspection mutated runtime or account');
  require([...document.querySelectorAll('[data-reconnect-world]')].every(button => button.disabled), 'Client match enabled unverified reconnect');
  edit('/fixture/changed/mods');
  require($('clientInventoryStatus').dataset.state !== 'checked', 'Path edit retained success');
  const acquired = window.__swarmcraftCalls.length;
  $('acquireRequiredMods').click();
  await waitFor(() => $('modAcquisitionReport').textContent.includes('Supply the exact authorized JAR manually'), 'provider acquisition report');
  require($('modAcquisitionReport').textContent.includes('Downloaded and verified'), 'Verified acquisition absent');
  require($('modAcquisitionReport').textContent.includes('CurseForge 123/456'), 'Exact manual fallback absent');
  require(window.__swarmcraftCalls.slice(acquired).filter(call => call.command === 'world_mods_acquire').length === 1, 'Unexpected acquisition count');
  require(!window.__swarmcraftCalls.slice(acquired).some(call => ['runtime_install', 'runtime_repair', 'hosting_contribution_set'].includes(call.command)), 'Acquisition changed runtime or consent');
  require(window.__swarmcraftBrowserErrors.length === 0, 'Client preparation browser errors: ' + window.__swarmcraftBrowserErrors.join(' | '));
  return { status: 'pass' };
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
  const recovery = await executeAsync(port, sessionId, scenarioRunner, [recoveryScenario]);
  assert.equal(recovery?.ok, true, `Recovery browser scenario failed at ${width}x${height}: ${recovery?.error || JSON.stringify(recovery)}`);
  assert.equal(recovery.value?.status, 'pass');
  const recoveryScreenshot = await webdriverRequest(port, `/session/${sessionId}/screenshot`);
  await writeFile(screenshotPath.replace(/\.png$/, '-recovery.png'), Buffer.from(recoveryScreenshot, 'base64'));
  const selection = await executeAsync(port, sessionId, scenarioRunner, [selectionScenario]);
  assert.equal(selection?.ok, true, `Selection browser scenario failed at ${width}x${height}: ${selection?.error || JSON.stringify(selection)}`);
  const selectionScreenshot = await webdriverRequest(port, `/session/${sessionId}/screenshot`);
  await writeFile(screenshotPath.replace(/\.png$/, '-selection.png'), Buffer.from(selectionScreenshot, 'base64'));
  const branch = await executeAsync(port, sessionId, scenarioRunner, [branchScenario]);
  assert.equal(branch?.ok, true, `Branch browser scenario failed at ${width}x${height}: ${branch?.error || JSON.stringify(branch)}`);
  assert.equal(branch.value?.status, 'pass');
  const branchScreenshot = await webdriverRequest(port, `/session/${sessionId}/screenshot`);
  await writeFile(screenshotPath.replace(/\.png$/, '-branch.png'), Buffer.from(branchScreenshot, 'base64'));
  const invitation = await executeAsync(port, sessionId, scenarioRunner, [inviteScenario]);
  assert.equal(invitation?.ok, true, `Invite browser scenario failed at ${width}x${height}: ${invitation?.error || JSON.stringify(invitation)}`);
  assert.equal(invitation.value?.status, 'pass');
  const inviteScreenshot = await webdriverRequest(port, `/session/${sessionId}/screenshot`);
  await writeFile(screenshotPath.replace(/\.png$/, '-invitation.png'), Buffer.from(inviteScreenshot, 'base64'));
  const imported = await executeAsync(port, sessionId, scenarioRunner, [importScenario]);
  assert.equal(imported?.ok, true, `Import browser scenario failed at ${width}x${height}: ${imported?.error || JSON.stringify(imported)}`);
  assert.equal(imported.value?.status, 'pass');
  const clientProfile = await executeAsync(port, sessionId, scenarioRunner, [clientProfileScenario]);
  assert.equal(clientProfile?.ok, true, `Client preparation browser scenario failed at ${width}x${height}: ${clientProfile?.error || JSON.stringify(clientProfile)}`);
  assert.equal(clientProfile.value?.status, 'pass');
}

// Exercise the actual compiled Svelte module graph, including its shared
// controllers and original watchdog, in a real browser. Only the native IPC
// boundary is mocked. This server is local test-only, never a Desktop asset.
async function startCandidateFixture() {
  const html = await readFile(path.join(candidateRoot, 'index.html'), 'utf8');
  assert.match(html, /<script\b[^>]*type="module"[^>]*src="\/assets\//,
    'Build the pinned Svelte candidate before running compiled parity');
  assert.ok(html.includes('/bootstrap.js'), 'Candidate must retain startup watchdog');
  const types = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.png': 'image/png', '.svg': 'image/svg+xml' };
  const server = http.createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://127.0.0.1');
      if (url.pathname === '/fixture-ipc.js') {
        response.writeHead(200, { 'Content-Type': 'text/javascript', 'Cache-Control': 'no-store' });
        response.end(tauriFixture);
        return;
      }
      if (url.pathname === '/') {
        response.writeHead(200, { 'Content-Type': 'text/html', 'Cache-Control': 'no-store' });
        response.end(html.replace('<head>', '<head><script src="/fixture-ipc.js"></script>'));
        return;
      }
      // Serve only built static assets; never arbitrary repo/user files.
      if (!/^\/(?:assets\/[A-Za-z0-9_.-]+|[A-Za-z0-9_.-]+)$/.test(url.pathname)
          || url.pathname.includes('..')) {
        response.writeHead(404).end(); return;
      }
      const type = types[path.extname(url.pathname)];
      if (!type) { response.writeHead(404).end(); return; }
      const bytes = await readFile(path.join(candidateRoot, url.pathname.slice(1)));
      response.writeHead(200, { 'Content-Type': type, 'Cache-Control': 'no-store' });
      response.end(bytes);
    } catch {
      response.writeHead(404).end();
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  return {
    url: 'http://127.0.0.1:' + server.address().port + '/',
    close: () => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }),
  };
}

async function runCandidateViewport(port, sessionId, url, width, height, screenshotPath) {
  await setViewport(port, sessionId, width, height);
  await webdriverRequest(port, `/session/${sessionId}/url`, { method: 'POST', body: { url } });
  let state;
  for (let attempt = 0; attempt < 200; attempt += 1) {
    state = await execute(port, sessionId, `
      return { mounted: window.__swarmcraftSvelteCreateOwned === true,
        startup: document.documentElement.dataset.swarmcraftStartup,
        controller: typeof window.__swarmcraftAfterSvelteCreate === 'function',
        errors: window.__swarmcraftBrowserErrors || [] };
    `);
    if (state?.mounted && state.controller && state.startup === 'ready') break;
    if (state?.startup === 'failed') break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.ok(state?.mounted && state.controller && state.startup === 'ready',
    'Compiled candidate did not finish real controller boot: ' + JSON.stringify(state));
  for (const [name, script] of [
    ['create-provider', scenario], ['recovery', recoveryScenario],
    ['selection', selectionScenario], ['branch', branchScenario], ['invitation', inviteScenario], ['import', importScenario],
    ['client-preparation', clientProfileScenario],
  ]) {
    const result = await executeAsync(port, sessionId, scenarioRunner, [script]);
    assert.equal(result?.ok, true, `Compiled candidate ${name} failed at ${width}x${height}: ${result?.error || JSON.stringify(result)}`);
    assert.equal(result.value?.status, 'pass');
    const screenshot = Buffer.from(await webdriverRequest(port, `/session/${sessionId}/screenshot`), 'base64');
    assert.deepEqual(pngDimensions(screenshot), { width, height });
    await writeFile(screenshotPath.replace(/\.png$/, `-${name}.png`), screenshot);
  }
}

test('current Desktop module graph initializes, owns canonical Create, and renders supported window sizes', async () => {
  await mkdir(evidenceRoot, { recursive: true });
  if (candidateMode) {
    // Reuse one isolated browser for all three viewports. Never dispatch
    // another hosted workflow merely to exercise this explicit local gate.
    const fixture = await startCandidateFixture();
    let local;
    try {
      local = await startLocalCdp();
      for (const [width, height] of [[980, 760], [720, 560], [1280, 900]]) {
        await runCandidateViewport(local, 'local', fixture.url, width, height,
          path.join(evidenceRoot, `candidate-${width}x${height}.png`));
      }
    } finally {
      try { if (local) await local.close(); } finally { await fixture.close(); }
    }
    return;
  }
  const [documentHtml, graph] = await Promise.all([instrumentedDocument(), collectModuleGraph()]);
  if (process.env.SWARMCRAFT_LOCAL_CDP === '1') {
    const local = await startLocalCdp();
    try {
      for (const [width, height] of [[980, 760], [720, 560], [1280, 900]]) {
        const screenshot = path.join(evidenceRoot, `${width}x${height}.png`);
        await runViewport(local, 'local', documentHtml, graph, width, height, screenshot);
        assert.deepEqual(pngDimensions(await readFile(screenshot)), { width, height });
      }
    } finally { await local.close(); }
    return;
  }
  const driver = await existingChromeDriver();
  assert.ok(driver, 'ChromeDriver is required for Agent 7 real-browser smoke gate on Linux CI');
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
