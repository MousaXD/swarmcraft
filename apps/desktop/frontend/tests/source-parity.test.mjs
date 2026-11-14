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

test('candidate Diagnostics navigation is visible despite the legacy wizard rule', async () => {
  const [legacyCss, overrideCss, entry, sidebar, nativeAcceptance] = await Promise.all([
    source('../../src/runtime-wizard.css'),
    source('../src/candidate-navigation.css'),
    source('../src/main.ts'),
    source('../src/components/Sidebar.svelte'),
    source('../scripts/visual-window-acceptance.py'),
  ]);
  assert.match(legacyCss, /#navDiagnostics\s*\{\s*display:\s*none\s*;/);
  assert.match(overrideCss, /#navDiagnostics\s*\{\s*display:\s*flex\s*;/);
  assert.match(overrideCss, /#advancedNavigation\[open\]\s+#navDiagnostics\s*\{\s*display:\s*flex\s*;/,
    'Expanded Tools must override the legacy hidden Diagnostics rule with greater specificity');
  assert.ok(nativeAcceptance.includes('identityNavDisplayVisible') &&
    nativeAcceptance.includes('identityNavHasGeometry') &&
    nativeAcceptance.includes('identityNavInsideViewport'),
    'Installed native failures must distinguish hidden navigation from clipping without exposing private DOM text');
  const legacyIndex = entry.indexOf("import '../../src/runtime-wizard.css';");
  const overrideIndex = entry.indexOf("import './candidate-navigation.css';");
  assert.ok(legacyIndex >= 0 && overrideIndex > legacyIndex,
    'Svelte candidate override must load AFTER the legacy rule, never change shipping styling');
  assert.match(sidebar, /id="navDiagnostics"/);
  assert.ok(nativeAcceptance.includes('app.click("#navDiagnostics")'),
    'Installed WebKitGTK must click the real visible Diagnostics navigation');
  assert.ok(nativeAcceptance.includes('identityNavigationVisible'),
    'Native acceptance must prove navigation is visible before clicking it');
  const hybridExperience = await source('../../src/player-experience.js');
  assert.ok(hybridExperience.includes("details.id = 'advancedNavigation'"),
    'Hybrid player controller currently owns the Tools disclosure');
  assert.ok(hybridExperience.includes('body.append(activity, diagnostics)'),
    'Diagnostics still moves under the Tools disclosure before full Svelte cutover');
  assert.ok(nativeAcceptance.includes('app.click("#advancedNavigation > summary")'),
    'Native acceptance must open the real Tools disclosure, not click a hidden child');
  assert.ok(nativeAcceptance.includes("assert opened['identityToolsOpen']"),
    'Native acceptance must prove the Tools disclosure actually expanded');
  assert.ok(nativeAcceptance.indexOf('app.click("#advancedNavigation > summary")') <
    nativeAcceptance.indexOf('app.click("#navDiagnostics")'),
    'Expand Tools before clicking the real Diagnostics control');
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
  assert.ok(dialog.includes('onsubmit={preventInviteSubmitWhilePending}'), 'guard must be wired to real invite form submission');
  assert.ok(dialog.includes("window.__swarmcraftResetSvelteInvite = resetInvite"));
  assert.ok(app.includes("id === 'createInvite' && window.__swarmcraftSvelteInviteOwned === true"));
  assert.ok(root.includes("window.__swarmcraftSvelteInviteOwned = true"));
  assert.doesNotMatch(dialog, /append\(\s*signed|setOutput\(\s*signed/);
});

test('Svelte World-ID hint never impersonates signed membership or issues duplicate legacy actions', async () => {
  const [join, app, root] = await Promise.all([
    source('../src/components/JoinWorldView.svelte'),
    source('../../src/app.js'),
    source('../src/App.svelte'),
  ]);
  assert.ok(join.includes('onclick={checkWorldId}'));
  assert.ok(join.includes('hidden={!worldIdNotice}>{worldIdNotice}'));
  assert.ok(join.includes('resolveSignedDiscovery(world)'));
  assert.ok(join.includes('disabled={resolvingWorldId}'));
  assert.doesNotMatch(join.slice(join.indexOf('async function checkWorldId()')), /nativeWorldActions\(\)\.join/);
  assert.ok(app.includes("id === 'joinWorldIdButton' && window.__swarmcraftSvelteDiscoveryOwned === true"));
  assert.ok(root.includes('window.__swarmcraftSvelteDiscoveryOwned = true;'));
});

test('World ID lookup preserves actual Rust signed discovery and capture-phase legacy fallback', async () => {
  const [controller, discovery, script] = await Promise.all([
    source('../../src/launcher-controller.js'),
    source('../src/lib/discovery.ts'),
    source('../scripts/visual-window-acceptance.py'),
  ]);
  assert.ok(controller.includes('window.__swarmcraftSvelteDiscoveryOwned === true) return'));
  assert.ok(controller.includes("await call('discovery_resolve', { world })"), 'Shipping lookup is retained');
  assert.ok(discovery.includes("nativeBridge().invoke('discovery_resolve', { world })"), 'Svelte performs a real lookup');
  assert.ok(script.includes('app.click("#joinPanel details.task-secondary summary")'));
});

test('Svelte signed Leave World remains a deliberate confirmation and never double-invokes Rust', async () => {
  const [dialog, app, root, bridge] = await Promise.all([
    source('../src/components/Dialogs.svelte'),
    source('../../src/app.js'), source('../src/App.svelte'),
    source('../src/lib/native-world-actions.ts'),
  ]);
  assert.ok(dialog.includes('onclick={confirmLeave}'));
  assert.ok(dialog.includes('disabled={leaving}'));
  assert.ok(dialog.includes('await requestLeaveWorld(id)'));
  assert.ok(dialog.includes('await window.__swarmcraftAfterSvelteLeave()'));
  assert.ok(app.includes("id === 'confirmLeave' && window.__swarmcraftSvelteLeaveOwned === true"));
  assert.ok(root.includes('window.__swarmcraftSvelteLeaveOwned = true;'));
  assert.ok(bridge.includes("nativeBridge().invoke('leave_world', { world: id })"));
  assert.ok(dialog.includes('if (busy || leaving) event.preventDefault();'));
});

test('native completion and subsequent refresh failures are distinguished without exposing signed invite secrets', async () => {
  const [create, importer, join] = await Promise.all([
    source('../src/components/CreateWorldView.svelte'),
    source('../src/components/ImportWorldView.svelte'),
    source('../src/components/JoinWorldView.svelte'),
  ]);
  for (const component of [create, importer, join]) {
    assert.ok(component.includes('let nativeSucceeded = false;'));
    assert.ok(component.includes('nativeSucceeded = true;'));
    assert.ok(component.includes('error = nativeSucceeded'), 'post-success UI failure needs an honest separate status');
    assert.doesNotMatch(component, /error\s*=\s*String\(failure\)|error\s*=\s*failure\.message\s*\|\|/, 'raw native command errors may contain secret arguments');
  }
  const clearInvite = join.indexOf("if (inviteInput) inviteInput.value = '';");
  const refresh = join.indexOf('await window.__swarmcraftAfterSvelteJoin(');
  assert.ok(clearInvite >= 0 && refresh > clearInvite, 'successful signed tokens must clear before any potentially failing UI refresh');
});

test('Svelte identity diagnostics use genuine typed Rust IPC without duplicate legacy binding', async () => {
  const [diagnostics, app, root, identity, types] = await Promise.all([
    source('../src/components/DiagnosticsView.svelte'),
    source('../../src/app.js'),
    source('../src/App.svelte'),
    source('../src/lib/device-identity.ts'),
    source('../src/lib/native-bridge.ts'),
  ]);
  assert.ok(diagnostics.includes("nativeBridge().invoke('node_identity', {})"));
  assert.ok(diagnostics.includes('onclick={refreshIdentity}'));
  assert.ok(diagnostics.includes('disabled={$identityState.busy}'));
  assert.ok(diagnostics.includes('{$identityState.text}'));
  assert.ok(diagnostics.includes('data-identity-phase={$identityState.phase}'));
  assert.ok(diagnostics.includes('data-native-verified-refresh-count={$identityState.nativeVerifiedRefreshCount}'));
  assert.ok(identity.includes('if (text && fromNativeRefresh) nativeVerifiedRefreshCount += 1;'));
  assert.ok(identity.includes('if (version === generation) state.set(present(value, true));'));
  assert.ok(identity.includes('nativeVerifiedRefreshCount: 0'));

  assert.ok(diagnostics.includes('aria-busy={$identityState.busy}'));
  assert.ok(root.includes('window.__swarmcraftSvelteIdentityOwned = true;'));
  assert.ok(app.includes("id === 'identityButton' && window.__swarmcraftSvelteIdentityOwned === true"));
  assert.ok(app.includes('window.__swarmcraftSvelteIdentity?.publish('));
  assert.ok(identity.includes('if (request) return request;'), 'duplicate native reads must be coalesced');
  assert.ok(identity.includes('if (version === generation)'), 'stale native completions must not overwrite a newer identity');
  assert.ok(types.includes('node_identity: { args: Record<string, never>; result: string }'));
  assert.doesNotMatch(identity, /error:\s*String\(error\)|error:\s*error\.message/, 'never expose raw native errors');
});

test('native visual failures emit only stage, class and allowlisted UI observations', async () => {
  const [script, workflow, regression] = await Promise.all([
    source('../scripts/visual-window-acceptance.py'),
    source('../../../../.github/workflows/svelte-candidate-native-gui.yml'),
    source('../scripts/test_visual_failure_evidence.py'),
  ]);
  assert.ok(script.includes("'failure.json'"));
  assert.ok(script.includes("mark_stage('native-identity-refresh')"));
  assert.ok(script.includes("identityVerified: output?.dataset.identityPhase === 'ready'"));
  assert.ok(script.includes("identityRefreshFinished: !!button && !button.disabled"));
  assert.ok(script.includes('identityNativeConfirmed:'));
  assert.ok(script.includes('nativeVerifiedRefreshCount ?? -1'));
  assert.ok(script.includes('app.click("#identityButton")'));
  assert.ok(script.includes('type(native_before) is int'));

  assert.ok(script.includes("'identityOwned', 'identityVerified', 'identityNotEmpty', 'identityRefreshFinished'"));
  assert.ok(script.includes("'stage': CURRENT_STAGE"));
  assert.ok(script.includes("'category': category"));
  assert.ok(script.includes("'ui': CURRENT_UI_STATE"));
  assert.ok(script.includes('SAFE_BOOL_KEYS'));
  assert.ok(script.includes('SAFE_INT_KEYS'));
  assert.ok(script.includes('failure_category(error)'));
  assert.doesNotMatch(script, /failure\.json[^\n]*String\(error\)|write_text\(str\(error\)\)/);
  assert.ok(workflow.includes("test_visual_failure_evidence.py"));
  assert.ok(regression.includes('scinvite:secret-private-token'));
});

test('installed Svelte leave-cancel exercises the real membership disclosure before hidden destructive action', async () => {
  const [view, nativeAcceptance] = await Promise.all([
    source('../src/components/WorldsView.svelte'),
    source('../scripts/visual-window-acceptance.py'),
  ]);
  assert.match(view, /<details class="danger-zone">[\s\S]*?<button id="leaveWorld"/);
  assert.ok(nativeAcceptance.includes("summary.scrollIntoView({ behavior: 'auto', block: 'center', inline: 'nearest' })"),
    'native GUI must scroll the real offscreen membership control into view before clicking it');
  assert.ok(nativeAcceptance.includes("assert summary_on_screen['leaveSummaryOnScreen']"),
    'native GUI must reject clipped or unreachable membership controls');
  const open = nativeAcceptance.indexOf('app.click("#selectionContent details.danger-zone > summary")');
  const leave = nativeAcceptance.indexOf('app.click("#leaveWorld")');
  assert.ok(open >= 0 && leave > open, 'real native WebDriver must expand Membership before clicking Leave');
  assert.ok(nativeAcceptance.includes("assert expanded['leaveDisclosureOpen'] and expanded['leaveButtonVisible']"),
    'native leave test must not bypass collapsed or hidden membership controls');
  assert.ok(nativeAcceptance.includes("assert selected['leaveSelectionVisible']"),
    'native test must not mask losing selected world before leave');
  assert.ok(nativeAcceptance.includes('app.click("#cancelLeave")'),
    'native cancellation must remain a real click, not a programmatic dialog close');
});
