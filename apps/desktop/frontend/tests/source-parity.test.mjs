import test from 'node:test';
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
