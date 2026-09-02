import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, mkdir, mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const sourceRoot = path.resolve(here, '../src');
const evidenceRoot = path.resolve(here, '../test-artifacts/agent-7-browser');
const mime = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'],
]);

async function existingBrowser() {
  const candidates = [
    process.env.CHROME_PATH,
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
    '/usr/bin/google-chrome',
    '/usr/bin/google-chrome-stable',
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      await access(candidate);
      return candidate;
    } catch {}
  }
  return null;
}

function bootstrapScript() {
  return String.raw`<script>
    window.__swarmcraftBrowserErrors = [];
    window.__swarmcraftCalls = [];
    window.__swarmcraftWorldModFailures = 1;
    addEventListener('error', (event) => {
      window.__swarmcraftBrowserErrors.push(String(event.error?.stack || event.message || event.error || 'window error'));
    });
    addEventListener('unhandledrejection', (event) => {
      window.__swarmcraftBrowserErrors.push(String(event.reason?.stack || event.reason || 'unhandled rejection'));
    });
    const record = (command, args) => window.__swarmcraftCalls.push({
      command,
      args: args ? JSON.parse(JSON.stringify(args)) : null,
    });
    window.__TAURI__ = { core: { invoke: async (command, args = {}) => {
      record(command, args);
      switch (command) {
        case 'minecraft_versions':
          if (typeof args.includeSnapshots !== 'boolean' || typeof args.refresh !== 'boolean') {
            throw new Error('minecraft_versions requires includeSnapshots and refresh booleans');
          }
          return { origin: 'network', versions: [{ id: '1.21.1', type: 'release', release_time: '2024-08-08T00:00:00Z', supported: true }] };
        case 'fabric_loader_versions':
          if (!args.minecraftVersion || typeof args.refresh !== 'boolean') {
            throw new Error('fabric_loader_versions requires minecraftVersion and refresh');
          }
          return { origin: 'network', versions: [{ version: '0.16.10', stable: true, minecraft_version: args.minecraftVersion }] };
        case 'list_worlds': return [];
        case 'identity':
        case 'node_identity': return { peerId: '12D3KooWBrowserSmoke' };
        case 'migration_capabilities': return { transfer: false, wake: false, reason: 'Browser smoke fixture' };
        case 'host_readiness': return { eligible: false, reason: 'No selected world in browser smoke fixture' };
        case 'connectivity_status': return { state: 'nat_status_unknown' };
        case 'modrinth_search': return { items: [{ project_id: 'lithium', title: 'Lithium' }] };
        case 'modrinth_resolve_project':
          return { versions: [{
            project_id: 'lithium', version_id: 'lithium-1', display_name: 'Lithium', dependencies: [],
            files: [{
              primary: true,
              filename: 'lithium.jar',
              locator: { url: 'https://cdn.modrinth.com/lithium.jar' },
              retrieval: { state: 'provider_download' },
              hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
            }],
          }] };
        case 'provider_staging_dir': return '/tmp/swarmcraft-provider-smoke';
        case 'modrinth_download':
          return {
            path: '/tmp/swarmcraft-provider-smoke/lithium.jar', filename: 'lithium.jar', size: 123,
            hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
          };
        case 'inspect_mod_artifact': return { mod_id: 'lithium', version: '1.0.0', environment: 'server' };
        case 'create_canonical_world': return { worldId: 'scworld:browser-smoke', canonical: { compatibilityFingerprint: 'browser-fingerprint' } };
        case 'world_mods_add':
          if (window.__swarmcraftWorldModFailures > 0) {
            window.__swarmcraftWorldModFailures -= 1;
            throw new Error('simulated local mod installation failure');
          }
          return { status: 'ok' };
        case 'discovery_search': return { results: [] };
        case 'discovery_resolve': return { state: 'found', world: { name: 'Browser Public World', visibility: 'public' } };
        default: return null;
      }
    } } };
  </script>`;
}

function scenarioScript() {
  return String.raw`<script type="module">
    const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const waitFor = async (predicate, label, timeoutMs = 3500) => {
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
    const report = async (payload) => {
      document.documentElement.dataset.browserSmoke = payload.status;
      try {
        await fetch('/__agent7_result' + location.search, {
          method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(payload), keepalive: true,
        });
      } catch (error) {
        document.documentElement.dataset.reportError = String(error);
      }
    };

    addEventListener('DOMContentLoaded', () => setTimeout(async () => {
      try {
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
        require(window.__swarmcraftCalls.filter((call) => call.command === 'create_world').length === 0, 'Legacy create_world handler still owned the intended Create path');
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
        document.documentElement.dataset.layoutPass = 'true';
        await report({ status: 'pass', width: innerWidth, height: innerHeight, calls: window.__swarmcraftCalls.length });
      } catch (error) {
        const message = String(error?.stack || error);
        const failure = document.createElement('pre');
        failure.id = 'browserSmokeFailure';
        failure.textContent = message + '\nBrowser errors:\n' + window.__swarmcraftBrowserErrors.join('\n');
        document.body.append(failure);
        await report({ status: 'fail', message, browserErrors: window.__swarmcraftBrowserErrors });
      }
    }, 0), { once: true });
  </script>`;
}

function instrumentIndex(html) {
  const marker = '<script type="module" src="./app.js"></script>';
  assert.ok(html.includes(marker), 'index.html app module marker changed; browser smoke harness must be reviewed');
  return html.replace(marker, `${bootstrapScript()}\n${marker}\n${scenarioScript()}`);
}

async function readBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Buffer.concat(chunks).toString('utf8');
}

async function startServer() {
  const index = instrumentIndex(await readFile(path.join(sourceRoot, 'index.html'), 'utf8'));
  const waiters = new Map();
  const results = new Map();
  const server = http.createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://127.0.0.1');
      if (url.pathname === '/__agent7_result' && request.method === 'POST') {
        const token = url.searchParams.get('token') || '';
        const payload = JSON.parse(await readBody(request));
        results.set(token, payload);
        waiters.get(token)?.(payload);
        response.writeHead(204, { 'cache-control': 'no-store' });
        response.end();
        return;
      }
      const requested = url.pathname === '/' || url.pathname === '/index.html' ? '/index.html' : url.pathname;
      if (requested === '/index.html') {
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        response.end(index);
        return;
      }
      const relative = requested.replace(/^\/+/, '');
      const file = path.resolve(sourceRoot, relative);
      if (!file.startsWith(`${sourceRoot}${path.sep}`)) throw new Error('invalid path');
      const content = await readFile(file);
      response.writeHead(200, { 'content-type': mime.get(path.extname(file)) || 'application/octet-stream', 'cache-control': 'no-store' });
      response.end(content);
    } catch {
      response.writeHead(404);
      response.end('not found');
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  return {
    server,
    urlFor(token) { return `http://127.0.0.1:${port}/index.html?token=${encodeURIComponent(token)}`; },
    resultFor(token, timeoutMs = 12000) {
      if (results.has(token)) return Promise.resolve(results.get(token));
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          waiters.delete(token);
          reject(new Error(`Browser did not report a smoke result for ${token}`));
        }, timeoutMs);
        waiters.set(token, (payload) => {
          clearTimeout(timeout);
          waiters.delete(token);
          resolve(payload);
        });
      });
    },
  };
}

async function waitForScreenshot(file, timeoutMs = 12000) {
  const started = Date.now();
  let lastSize = -1;
  let stableCount = 0;
  while (Date.now() - started < timeoutMs) {
    try {
      const info = await stat(file);
      if (info.size > 1000 && info.size === lastSize) {
        stableCount += 1;
        if (stableCount >= 2) return info;
      } else {
        stableCount = 0;
        lastSize = info.size;
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Chromium did not produce screenshot ${file}`);
}

async function runBrowser(browser, server, width, height, screenshotPath) {
  const profile = await mkdtemp(path.join(os.tmpdir(), 'swarmcraft-agent7-chrome-'));
  const token = `${width}x${height}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  const child = spawn(browser, [
    '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage', '--hide-scrollbars',
    '--virtual-time-budget=6000', `--user-data-dir=${profile}`, `--window-size=${width},${height}`,
    `--screenshot=${screenshotPath}`, server.urlFor(token),
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const exited = new Promise((resolve) => child.once('exit', resolve));

  try {
    const [payload] = await Promise.all([server.resultFor(token), waitForScreenshot(screenshotPath)]);
    assert.equal(payload.status, 'pass', `Browser smoke failed at ${width}x${height}: ${payload.message || JSON.stringify(payload)}\nChromium stderr:\n${stderr}`);
  } finally {
    if (child.exitCode === null) child.kill('SIGTERM');
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 1000))]);
    if (child.exitCode === null) child.kill('SIGKILL');
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 1000))]);
    await rm(profile, { recursive: true, force: true });
  }
}

function pngDimensions(buffer) {
  assert.equal(buffer.toString('ascii', 1, 4), 'PNG', 'Expected PNG screenshot');
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

test('current Desktop module graph initializes, owns canonical Create, and renders supported window sizes', async (t) => {
  const browser = await existingBrowser();
  assert.ok(browser, 'A Chromium/Chrome binary is required for the Agent 7 browser smoke gate on Linux CI');
  await mkdir(evidenceRoot, { recursive: true });
  const server = await startServer();
  t.after(() => new Promise((resolve) => server.server.close(resolve)));

  for (const [width, height] of [[980, 760], [720, 560], [1280, 900]]) {
    const screenshot = path.join(evidenceRoot, `${width}x${height}.png`);
    await runBrowser(browser, server, width, height, screenshot);
    const dimensions = pngDimensions(await readFile(screenshot));
    assert.deepEqual(dimensions, { width, height });
  }
});
