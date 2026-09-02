import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '../../..');

function inputValue(html, id) {
  const match = html.match(new RegExp(`<input[^>]+id=["']${id}["'][^>]+value=["']([^"']+)["']`, 'i'));
  return match?.[1] || '';
}

test('Desktop compatibility defaults match the Fabric adapter contract Agent 6 enforces', async () => {
  const metadata = JSON.parse(await readFile(
    path.join(repoRoot, 'minecraft/fabric/src/main/resources/fabric.mod.json'),
    'utf8',
  ));
  const html = await readFile(path.join(repoRoot, 'apps/desktop/src/index.html'), 'utf8');

  assert.equal(metadata.depends.minecraft, '~26.1.2');
  assert.equal(metadata.depends.fabricloader, '>=0.19.3');
  assert.equal(metadata.depends.java, '>=25');

  for (const id of ['createMinecraft', 'importMinecraft']) {
    assert.equal(inputValue(html, id), '26.1.2', `${id} drifted from the shipped Minecraft adapter line`);
  }
  for (const id of ['createLoader', 'importLoader']) {
    assert.equal(inputValue(html, id), '0.19.3', `${id} drifted below the shipped Fabric Loader minimum`);
  }
});
