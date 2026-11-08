import test from 'node:test';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = async (name) => readFile(new URL(name, import.meta.url), 'utf8');
const ids = (value) => [...value.matchAll(/\bid=["']([^"']+)["']/g)].map((match) => match[1]);
const componentNames = [
  'Sidebar', 'Topbar', 'WorldsView', 'CreateWorldView', 'ImportWorldView',
  'JoinWorldView', 'ActivityView', 'DiagnosticsView', 'Dialogs', 'Workspace',
];

test('Svelte component split retains every legacy DOM binding and every view', async () => {
  const [original, app, ...parts] = await Promise.all([
    source('../../src/index.html'), source('../src/App.svelte'),
    ...componentNames.map((name) => source('../src/components/' + name + '.svelte')),
  ]);
  const expected = ids(original);
  const actual = ids([app, ...parts].join('\n'));
  assert.ok(expected.length >= 150);
  assert.equal(new Set(expected).size, expected.length, 'legacy IDs must be unique');
  assert.equal(new Set(actual).size, actual.length, 'component IDs must be unique');
  assert.deepEqual(new Set(actual), new Set(expected), 'every existing native/UI binding must survive');
  for (const view of ['worlds', 'create', 'import', 'join', 'activity', 'diagnostics']) {
    assert.ok(parts.join('\n').includes('data-view-panel="' + view + '"'), 'Missing view ' + view);
  }
  assert.ok(app.includes('onMount(() => {'));
  assert.ok(app.includes("import('../../src/app.js')"));
  for (const name of ['Sidebar', 'Workspace', 'Dialogs']) {
    assert.ok(app.includes('<' + name + ' />'), name + ' must be mounted in the app shell');
  }
});

test('PR 88 native startup watchdog stays byte-for-byte intact', async () => {
  assert.equal(await source('../public/bootstrap.js'), await source('../../src/bootstrap.js'));
});

test('native IPC invocation remains read-only and production packaging is not silently switched', async () => {
  const [bridge, tauri] = await Promise.all([
    source('../src/lib/native-bridge.ts'),
    source('../../src-tauri/tauri.conf.json'),
  ]);
  assert.doesNotMatch(bridge, /__TAURI__\s*=|\.invoke\s*=/);
  assert.equal(JSON.parse(tauri).build.frontendDist, '../src', 'Svelte pilot must not ship before Agent 00 release approval');
});


test('typed frontend IPC table exactly matches every Rust Tauri-registered command', async () => {
  const [main, bridge] = await Promise.all([
    source('../../src-tauri/src/main.rs'),
    source('../src/lib/native-bridge.ts'),
  ]);
  const handler = main.match(/\.invoke_handler\(tauri::generate_handler!\[([\s\S]*?)\]\)/);
  assert.ok(handler, 'Rust Tauri invoke registry must be present');
  const commands = handler[1].split(',').map(x => x.trim()).filter(Boolean);
  const table = bridge.match(/export interface NativeCommandTable \{([\s\S]*?)\n\}/);
  assert.ok(table, 'Native command table must be present');
  const typed = [...table[1].matchAll(/^\s+([a-z_]+): \{ args:/gm)].map(x => x[1]);
  assert.equal(commands.length, 61, 'A Rust registry addition requires deliberate typed contract review');
  assert.deepEqual(new Set(typed), new Set(commands));
  assert.equal(new Set(typed).size, typed.length, 'No duplicate command definitions');
});


test('candidate native Tauri overlay is limited to the tested frontend directory', async () => {
  const [override, native] = await Promise.all([
    source('../tauri.candidate.conf.json'),
    source('../../src-tauri/tauri.conf.json'),
  ]);
  assert.deepEqual(JSON.parse(override), { build: { frontendDist: '../frontend/dist' } });
  const config = JSON.parse(native);
  assert.equal(config.build.frontendDist, '../src');
  assert.equal(config.app.withGlobalTauri, true);
  assert.equal(config.bundle.externalBin.length, 4);
  assert.ok(config.app.security.csp.includes("connect-src ipc:"));
});

test('legacy activity writers preserve fallback while candidate uses the Svelte store', async () => {
  const [legacy, runtime, component, activity] = await Promise.all([
    source('../../src/app.js'), source('../../src/runtime-wizard.js'),
    source('../src/components/ActivityView.svelte'), source('../src/lib/activity.ts'),
  ]);
  assert.ok(legacy.includes('window.__swarmcraftSvelteActivity?.append(entry)'));
  assert.ok(runtime.includes('window.__swarmcraftSvelteActivity?.append(entry)'));
  assert.ok(legacy.includes('console.textContent ='), 'original legacy path remains available');
  assert.ok(runtime.includes('output.textContent ='), 'wizard fallback remains available');
  assert.ok(component.includes('{$activityText}'), 'Svelte activity display subscribes reactively');
  assert.ok(component.includes('event.stopImmediatePropagation()'), 'clear cannot race old listener');
  assert.ok(activity.includes("activityText.set('Activity cleared.')"));
});


test('native responsiveness guards prevent horizontal scrollers and min-window regressions', async () => {
  const [styles, player, workflow] = await Promise.all([
    source('../../src/style.css'),
    source('../../src/player-experience.css'),
    source('../../../../.github/workflows/svelte-candidate-native-gui.yml'),
  ]);
  assert.ok(styles.includes('overflow-x: hidden;'), 'world-detail must not force page-width overflow');
  assert.ok(player.includes('overflow-wrap: anywhere;'), 'paths and IDs must be allowed to wrap');
  assert.match(player, /flex-wrap: wrap;/);
  assert.ok(workflow.includes('visual-window-acceptance.py'));
});

test('Svelte create journey invokes exactly one native create without legacy double submit', async () => {
  const [create, app, root, bridge] = await Promise.all([
    source('../src/components/CreateWorldView.svelte'),
    source('../../src/app.js'),
    source('../src/App.svelte'),
    source('../src/lib/native-world-actions.ts'),
  ]);
  assert.ok(create.includes('data-journey-owner="svelte"'));
  assert.ok(create.includes('onsubmit={create}'));
  assert.ok(create.includes('disabled={busy}'));
  assert.ok(create.includes('hidden={!error}>{error}'));
  assert.ok(root.includes('window.__swarmcraftSvelteCreateOwned = true;'));
  assert.ok(app.includes("id === 'createForm' && window.__swarmcraftSvelteCreateOwned === true"));
  assert.ok(app.includes('window.__swarmcraftAfterSvelteCreate = async (createdWorldId) =>'));
  assert.ok(bridge.includes("native.invoke('create_world', request)"));
  assert.doesNotMatch(bridge, /__TAURI__\s*=|\.invoke\s*=/);
});

test('Svelte Join World owns signed-invite submit and the legacy handler cannot double-join', async () => {
  const [join, root, app] = await Promise.all([
    source('../src/components/JoinWorldView.svelte'), source('../src/App.svelte'),
    source('../../src/app.js'),
  ]);
  assert.ok(join.includes('data-journey-owner="svelte"'));
  assert.ok(join.includes('onsubmit={join}'));
  assert.ok(join.includes('nativeWorldActions().join('));
  assert.ok(join.includes('hidden={!error}>{error}'));
  assert.ok(join.includes('disabled={busy}'));
  assert.ok(root.includes('window.__swarmcraftSvelteJoinOwned = true;'));
  assert.ok(app.includes("id === 'joinForm' && window.__swarmcraftSvelteJoinOwned === true"));
  assert.ok(app.includes('window.__swarmcraftAfterSvelteJoin = async (joinedWorldId) =>'));
});

test('Svelte Import World owns native form submission without implicit EULA or duplicate import', async () => {
  const [importer, app, root] = await Promise.all([
    source('../src/components/ImportWorldView.svelte'),
    source('../../src/app.js'), source('../src/App.svelte'),
  ]);
  assert.ok(importer.includes('data-journey-owner="svelte"'));
  assert.ok(importer.includes('onsubmit={importWorld}'));
  assert.ok(importer.includes('nativeWorldActions().import('));
  assert.ok(importer.includes('noServerMods: noServerMods?.checked === true'));
  assert.ok(importer.includes('hidden={!error}>{error}'));
  assert.ok(importer.includes('disabled={busy}'));
  assert.doesNotMatch(importer, /acceptEula|eula:\s*true/);
  assert.ok(root.includes('window.__swarmcraftSvelteImportOwned = true;'));
  assert.ok(app.includes("id === 'importForm' && window.__swarmcraftSvelteImportOwned === true"));
  assert.ok(app.includes('window.__swarmcraftAfterSvelteImport = async (importedWorldId) =>'));
});


test('deliberately broken Svelte entry must retain classic watchdog and use a separate Tauri package override', async () => {
  const [badOverlay, nativeWorkflow] = await Promise.all([
    source('../tauri.candidate.broken.conf.json'),
    source('../../../../.github/workflows/svelte-candidate-native-gui.yml'),
  ]);
  assert.deepEqual(JSON.parse(badOverlay), {build: {frontendDist: '../frontend/broken-dist'}});
  assert.ok(nativeWorkflow.includes('required-entry-deliberately-missing.js'));
  assert.ok(nativeWorkflow.includes('svelte-broken-installed'));
  assert.ok(nativeWorkflow.includes('svelte-evidence/broken-bootstrap/last-state.json'));
  assert.ok(nativeWorkflow.includes("state['startup'] == 'failed'"));
  assert.ok(nativeWorkflow.includes("state['retryVisible']"));
});


test('candidate bootstrap retains a visible fail-closed status without loading Svelte', async () => {
  const [html, guard, css, entry] = await Promise.all([
    source('../index.html'),
    source('../public/candidate-startup-guard.js'),
    source('../public/candidate-startup.css'),
    source('../src/main.ts'),
  ]);
  assert.match(html, /id="bootstrapFallback"/);
  assert.match(html, /src="\/bootstrap.js"/);
  assert.match(html, /src="\/candidate-startup-guard.js"/);
  assert.match(html, /href="\/candidate-startup.css"/);
  assert.ok(guard.includes("document.documentElement.dataset.swarmcraftStartup !== 'failed'"));
  assert.ok(guard.includes("document.querySelector('script[type=\"module\"]')"));
  assert.ok(guard.includes("status.textContent = 'Startup failed'"));
  assert.ok(guard.includes("fallback.replaceChildren(status)"));
  assert.ok(guard.includes("startup.fail('module-load'"));
  assert.ok(entry.includes("document.getElementById('bootstrapFallback')?.remove()"));
  assert.match(css, /#startupFailure #retryStartup/);
});


test('candidate classic guard visibly fails closed before any Svelte component mounts', async () => {
  const src = await source('../public/candidate-startup-guard.js');
  let observerCallback;
  const callbacks = {};
  let status;
  const fallback = {
    hidden: false,
    children: [],
    attrs: {},
    replaceChildren(...items) {
      this.children = items;
      status = items[0];
    },
    setAttribute(k, v) { this.attrs[k] = v; },
  };
  const html = { dataset: { swarmcraftStartup: 'loading' } };
  let nativeFailureKind;
  const sandbox = {
    document: {
      documentElement: html,
      getElementById: (id) => id === 'bootstrapFallback' ? fallback
        : id === 'status' ? status : null,
      createElement: (tag) => ({ tag }),
      querySelector: (selector) => {
        assert.equal(selector, 'script[type="module"]');
        return { addEventListener: (name, handler) => { callbacks[name] = handler; } };
      },
    },
    MutationObserver: class {
      constructor(callback) { observerCallback = callback; }
      observe(target, options) {
        assert.equal(target, html);
        assert.deepEqual([...options.attributeFilter], ['data-swarmcraft-startup']);
      }
    },
    window: {
      __swarmcraftStartup: {
        failed: false,
        fail(kind) {
          nativeFailureKind = kind;
          this.failed = true;
          html.dataset.swarmcraftStartup = 'failed';
          observerCallback();
        },
      },
    },
  };
  vm.runInNewContext(src, sandbox, { filename: 'candidate-startup-guard.js' });
  assert.equal(status, undefined, 'No duplicate status during normal loading');
  callbacks.error();
  assert.equal(nativeFailureKind, 'module-load');
  assert.equal(status.id, 'status', 'Native GUI source must have status element without Svelte');
  assert.equal(status.textContent, 'Startup failed');
  assert.equal(fallback.attrs.role, 'alert');
  assert.equal(fallback.hidden, false);
});

test('candidate guard leaves an already mounted native status untouched', async () => {
  const src = await source('../public/candidate-startup-guard.js');
  const realStatus = { id: 'status', textContent: 'Startup failed' };
  const fallback = { replaceChildren() { throw new Error('Duplicate status!'); } };
  const html = { dataset: { swarmcraftStartup: 'failed' } };
  const sandbox = {
    document: {
      documentElement: html,
      getElementById: (id) => id === 'status' ? realStatus
        : id === 'bootstrapFallback' ? fallback : null,
      querySelector: () => null,
    },
    MutationObserver: class { constructor(fn) { this.callback = fn; } observe() {} },
    window: {},
  };
  vm.runInNewContext(src, sandbox);
  assert.equal(realStatus.textContent, 'Startup failed');
});

test('Svelte invite dialog owns signed creation without duplicate legacy calls or token logging', async () => {
  const [dialog, app, root] = await Promise.all([
    source('../src/components/Dialogs.svelte'),
    source('../../src/app.js'),
    source('../src/App.svelte'),
  ]);
  assert.ok(dialog.includes('onclick={createInvite}'));
  assert.ok(dialog.includes('await nativeWorldActions().invite('));
  assert.ok(dialog.includes('hidden={!showSigned}'));
  assert.ok(dialog.includes('value={signed}'));
  assert.ok(dialog.includes('aria-busy={busy}'));
  assert.ok(dialog.includes('if (busy) event.preventDefault();'));
  assert.ok(dialog.includes("window.__swarmcraftResetSvelteInvite = resetInvite"));
  assert.ok(app.includes("id === 'createInvite' && window.__swarmcraftSvelteInviteOwned === true"));
  assert.ok(root.includes("window.__swarmcraftSvelteInviteOwned = true"));
  assert.doesNotMatch(dialog, /append\(\s*signed|setOutput\(\s*signed/);
});
