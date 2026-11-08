import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = async (name) => readFile(new URL(name, import.meta.url), 'utf8');
const ids = (text) => [...text.matchAll(/\\bid=["']([^"']+)["']/g)].map((m) => m[1]);

test('transitional Svelte DOM keeps every legacy element ID and all six views', async () => {
  const [original, svelte] = await Promise.all([
    source('../../src/index.html'), source('../src/App.svelte'),
  ]);
  const originalIds = ids(original);
  assert.ok(originalIds.length >= 150);
  assert.equal(new Set(originalIds).size, originalIds.length, 'legacy IDs must be unique');
  const svelteIds = ids(svelte);
  assert.deepEqual(new Set(svelteIds), new Set(originalIds));
  for (const view of ['worlds', 'create', 'import', 'join', 'activity', 'diagnostics']) {
    assert.match(svelte, new RegExp('data-view-panel=["\\\']' + view + '["\\\']'));
  }
  assert.match(svelte, /onMount\\(\\(\\) =>/);
  assert.match(svelte, /import\\('\\.\\.\\/\\.\\.\\/src\\/app\\.js'\\)/);
});

test('PR 88 native startup watchdog stays byte-for-byte intact', async () => {
  assert.equal(await source('../public/bootstrap.js'), await source('../../src/bootstrap.js'));
});

test('native IPC invocation remains read-only and production packaging is not silently switched', async () => {
  const [bridge, tauri] = await Promise.all([
    source('../src/lib/native-bridge.ts'),
    source('../../src-tauri/tauri.conf.json'),
  ]);
  assert.doesNotMatch(bridge, /__TAURI__\\s*=|\\.invoke\\s*=/);
  assert.equal(JSON.parse(tauri).build.frontendDist, '../src', 'Svelte pilot must not ship before Agent 00 release approval');
});
