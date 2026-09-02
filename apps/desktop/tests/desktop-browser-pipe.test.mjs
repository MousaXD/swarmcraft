import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
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

function smokeBootstrap() {
  return String.raw`<script>
    window.__swarmcraftBrowserErrors = [];
    window.__swarmcraftCalls = [];
    window.__swarmcraftWorldModFailures = 1;
    window.addEventListener('error', (event) => {
      window.__swarmcraftBrowserErrors.push(String(event.error?.stack || event.message || event.error || 'window error'));
    });
    window.addEventListener('unhandledrejection', (event) => {
      window.__swarmcraftBrowserErrors.push(String(event.reason?.stack || event.reason || 'unhandled rejection'));
    });
    const record = (command, args) => {
      window.__swarmcraftCalls.push({ command, args: args ? JSON.parse(JSON.stringify(args)) : null });
    };
    window.__TAURI__ = { core: { invoke: async (command, args = {}) => {
      record(command, args);
      switch (command) {
        case 'minecraft_versions':
          if (typeof args.includeSnapshots !== 'boolean' || typeof args.refresh !== 'boolean') {
            throw new Error('minecraft_versions requires includeSnapshots and refresh booleans');
          }
          return {
            origin: 'network',
            versions: [{ id: '1.21.1', type: 'release', release_time: '2024-08-08T00:00:00Z', supported: true }],
          };
        case 'fabric_loader_versions':
          if (!args.minecraftVersion || typeof args.refresh !== 'boolean') {
            throw new Error('fabric_loader_versions requires minecraftVersion and refresh');
          }
          return {
            origin: 'network',
            versions: [{ version: '0.16.10', stable: true, minecraft_version: args.minecraftVersion }],
          };
        case 'list_worlds':
          return [];
        case 'identity':
        case 'node_identity':
          return { peerId: '12D3KooWBrowserSmoke' };
        case 'migration_capabilities':
          return { transfer: false, wake: false, reason: 'Browser smoke fixture' };
        case 'host_readiness':
          return { eligible: false, reason: 'No selected world in browser smoke fixture' };
        case 'connectivity_status':
          return { state: 'nat_status_unknown' };
        case 'modrinth_search':
          return { items: [{ project_id: 'lithium', title: 'Lithium' }] };
        case 'modrinth_resolve_project':
          return {
            versions: [{
              project_id: 'lithium',
              version_id: 'lithium-1',
              display_name: 'Lithium',
              dependencies: [],
              files: [{
                primary: true,
                filename: 'lithium.jar',
                locator: { url: 'https://cdn.modrinth.com/lithium.jar' },
                retrieval: { state: 'provider_download' },
                hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
              }],
            }],
          };
        case 'provider_staging_dir':
          return '/tmp/swarmcraft-provider-smoke';
        case 'modrinth_download':
          return {
            path: '/tmp/swarmcraft-provider-smoke/lithium.jar',
            filename: 'lithium.jar',
            size: 123,
            hashes: { sha512: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
          };
        case 'inspect_mod_artifact':
          return { mod_id: 'lithium', version: '1.0.0', environment: 'server' };
        case 'create_canonical_world':
          return { worldId: 'scworld:browser-smoke', canonical: { compatibilityFingerprint: 'browser-fingerprint' } };
        case 'world_mods_add':
          if (window.__swarmcraftWorldModFailures > 0) {
            window.__swarmcraftWorldModFailures -= 1;
            throw new Error('simulated local mod installation failure');
          }
          return { status: 'ok' };
        case 'discovery_search':
          return { results: [] };
        case 'discovery_resolve':
          return { state: 'found', world: { name: 'Browser Public World', visibility: 'public' } };
        default:
          return null;
      }
    } } };
  </script>`;
}

function smokeScenario() {
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
    const require = (condition, message) => {
      if (!condition) throw new Error(message);
    };
    const focusable = (id) => {
      const element = document.getElementById(id);
      require(element, 'Missing keyboard control #' + id);
      const hiddenAncestors = [];
      for (let current = element; current; current = current.parentElement) {
        if (current.hidden) {
          hiddenAncestors.push(current);
          current.hidden = false;
        }
      }
      element.disabled = false;
      element.focus();
      require(document.activeElement === element, '#' + id + ' could not receive focus');
      for (const ancestor of hiddenAncestors) ancestor.hidden = true;
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
        require(
          minecraftCalls.every((call) => typeof call.args?.includeSnapshots === 'boolean' && typeof call.args?.refresh === 'boolean'),
          'A minecraft_versions call omitted required boolean payloads',
        );
        require(
          fabricCalls.every((call) => call.args?.minecraftVersion && typeof call.args?.refresh === 'boolean'),
          'A fabric_loader_versions call omitted required payloads',
        );

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

        const createCanonicalCalls = window.__swarmcraftCalls.filter((call) => call.command === 'create_canonical_world');
        const legacyCreateCalls = window.__swarmcraftCalls.filter((call) => call.command === 'create_world');
        require(createCanonicalCalls.length === 1, 'Canonical Create was not invoked exactly once');
        require(legacyCreateCalls.length === 0, 'Legacy create_world handler still owned the intended Create path');
        require(document.getElementById('createWorld').disabled, 'Create submit was re-enabled after canonical world already existed');
        require(document.getElementById('createRepairRetry'), 'Local mod repair action was not offered');
        require(document.getElementById('createError').textContent.includes('was created canonically'), 'Partial-success copy did not preserve canonical creation success');
        require(document.querySelector('#sleepDialog .dialog-card > div:first-child p').textContent.includes('durable sleeping state'), 'Stop dialog did not expose durable sleeping semantics');

        const keyboardIds = [
          'createName', 'createVisibility', 'createMinecraft', 'createLoader', 'modProvider', 'modSearch', 'modSearchButton', 'createWorld',
          'joinInvite', 'joinWorld', 'joinWorldId', 'joinWorldIdButton',
          'eula', 'host', 'inviteWorld', 'transferHost', 'sleepWorld', 'leaveWorld',
        ];
        for (const id of keyboardIds) focusable(id);

        require(window.__swarmcraftBrowserErrors.length === 0, 'Uncaught browser errors: ' + window.__swarmcraftBrowserErrors.join(' | '));
        require(document.body.scrollWidth <= innerWidth, 'Page has horizontal overflow at ' + innerWidth + 'px');
        document.getElementById('createError').scrollIntoView({ block: 'center' });
        document.documentElement.dataset.browserSmoke = 'pass';
        document.documentElement.dataset.layoutPass = 'true';
      } catch (error) {
        document.documentElement.dataset.browserSmoke = 'fail';
        const failure = document.createElement('pre');
        failure.id = 'browserSmokeFailure';
        failure.textContent = String(error?.stack || error) + '\nBrowser errors:\n' + window.__swarmcraftBrowserErrors.join('\n');
        document.body.append(failure);
      }
    }, 0), { once: true });
  </script>`;
}

function instrumentIndex(html) {
  const marker = '<script type="module" src="./app.js"></script>';
  assert.ok(html.includes(marker), 'index.html app module marker changed; browser smoke harness must be reviewed');
  return html.replace(marker, `${smokeBootstrap()}\n${marker}\n${smokeScenario()}`);
}

async function startServer() {
  const index = instrumentIndex(await readFile(path.join(sourceRoot, 'index.html'), 'utf8'));
  const server = http.createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://127.0.0.1');
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
  return { server, url: `http://127.0.0.1:${port}/index.html` };
}

class PipeDevToolsClient {
  constructor(readable, writable) {
    this.readable = readable;
    this.writable = writable;
    this.buffer = Buffer.alloc(0);
    this.nextId = 1;
    this.pending = new Map();
    this.closed = false;

    readable.on('data', (chunk) => this.onData(chunk));
    readable.on('error', (error) => this.close(error));
    readable.on('close', () => this.close(new Error('DevTools pipe closed')));
  }

  onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (true) {
      const delimiter = this.buffer.indexOf(0);
      if (delimiter < 0) return;
      const frame = this.buffer.subarray(0, delimiter).toString('utf8');
      this.buffer = this.buffer.subarray(delimiter + 1);
      if (!frame) continue;
      const message = JSON.parse(frame);
      if (!message.id) continue;
      const pending = this.pending.get(message.id);
      if (!pending) continue;
      this.pending.delete(message.id);
      clearTimeout(pending.timeout);
      if (message.error) pending.reject(new Error(`${pending.method}: ${message.error.message}`));
      else pending.resolve(message.result || {});
    }
  }

  send(method, params = {}, sessionId = null, timeoutMs = 5000) {
    if (this.closed) return Promise.reject(new Error(`DevTools pipe is closed before ${method}`));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`DevTools command timed out: ${method}`));
      }, timeoutMs);
      this.pending.set(id, { method, resolve, reject, timeout });
      const message = { id, method, params };
      if (sessionId) message.sessionId = sessionId;
      this.writable.write(`${JSON.stringify(message)}\0`);
    });
  }

  close(error = new Error('DevTools client closed')) {
    if (this.closed) return;
    this.closed = true;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.pending.clear();
    try { this.writable.end(); } catch {}
  }
}

async function attachPage(client) {
  const { targetInfos = [] } = await client.send('Target.getTargets');
  let page = targetInfos.find((target) => target.type === 'page');
  if (!page) {
    const created = await client.send('Target.createTarget', { url: 'about:blank' });
    page = { targetId: created.targetId };
  }
  const { sessionId } = await client.send('Target.attachToTarget', { targetId: page.targetId, flatten: true });
  assert.ok(sessionId, 'DevTools did not return a page session');
  return sessionId;
}

async function evaluate(client, sessionId, expression) {
  const result = await client.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, sessionId);
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text || 'Browser evaluation failed');
  return result.result?.value;
}

async function waitForSmokeResult(client, sessionId, timeoutMs = 10000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const status = await evaluate(client, sessionId, `document.documentElement?.dataset?.browserSmoke || ''`);
    if (status === 'pass' || status === 'fail') return status;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return 'timeout';
}

async function runBrowser(browser, url, width, height, screenshotPath) {
  const profile = await mkdtemp(path.join(os.tmpdir(), 'swarmcraft-agent7-chrome-'));
  const child = spawn(browser, [
    '--headless=new',
    '--disable-gpu',
    '--no-sandbox',
    '--disable-dev-shm-usage',
    '--remote-debugging-pipe',
    `--user-data-dir=${profile}`,
    `--window-size=${width},${height}`,
    'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'] });
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const exited = new Promise((resolve) => child.once('exit', resolve));
  const client = new PipeDevToolsClient(child.stdio[4], child.stdio[3]);

  try {
    const sessionId = await attachPage(client);
    await client.send('Page.enable', {}, sessionId);
    await client.send('Runtime.enable', {}, sessionId);
    await client.send('Emulation.setDeviceMetricsOverride', {
      width,
      height,
      deviceScaleFactor: 1,
      mobile: false,
      screenWidth: width,
      screenHeight: height,
    }, sessionId);
    await client.send('Page.navigate', { url }, sessionId);
    const status = await waitForSmokeResult(client, sessionId);
    if (status !== 'pass') {
      const failure = await evaluate(
        client,
        sessionId,
        `document.getElementById('browserSmokeFailure')?.textContent || 'Browser smoke did not complete before timeout'`,
      );
      assert.fail(`Browser smoke ${status} at ${width}x${height}: ${failure}\nChromium stderr:\n${stderr}`);
    }
    assert.equal(await evaluate(client, sessionId, `document.documentElement.dataset.layoutPass`), 'true');
    const capture = await client.send('Page.captureScreenshot', {
      format: 'png',
      fromSurface: true,
      captureBeyondViewport: false,
    }, sessionId);
    await writeFile(screenshotPath, Buffer.from(capture.data, 'base64'));
  } finally {
    try {
      if (!client.closed) await client.send('Browser.close', {}, null, 1000);
    } catch {}
    if (child.exitCode === null) {
      await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 1500))]);
    }
    if (child.exitCode === null) child.kill('SIGKILL');
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 1500))]);
    client.close();
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
  const { server, url } = await startServer();
  t.after(() => new Promise((resolve) => server.close(resolve)));

  const viewports = [
    [980, 760],
    [720, 560],
    [1280, 900],
  ];
  for (const [width, height] of viewports) {
    const screenshot = path.join(evidenceRoot, `${width}x${height}.png`);
    await runBrowser(browser, url, width, height, screenshot);
    const info = await stat(screenshot);
    assert.ok(info.size > 1000, `Screenshot ${width}x${height} is unexpectedly small`);
    const dimensions = pngDimensions(await readFile(screenshot));
    assert.deepEqual(dimensions, { width, height });
  }
});
