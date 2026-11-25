import assert from 'node:assert/strict';
import test from 'node:test';
import { describeModAcquisition } from '../src/mod-acquisition.js';
import { createBackendAdapter } from '../src/backend-adapter.js';

const world = 'scworld:' + 'a'.repeat(64);
const artifact = {
  mod_id: 'requiredmod', version: '1.0', artifact_hash: 'b'.repeat(64),
  provider: 'curse_forge', project_id: '123', version_id: '456', file_name: 'requiredmod.jar',
  state: 'manual_required',
};
test('exact manual fallback identifies the provider file without claiming readiness', () => {
  const text = describeModAcquisition({
    world_id: world, ready: false, artifacts: [artifact], inventory_issues: ['missing_required'],
  }, world);
  assert.match(text, /Supply the exact authorized JAR manually/);
  assert.match(text, /CurseForge 123\/456/);
  assert.match(text, /requiredmod.jar/);
  assert.match(text, new RegExp(artifact.artifact_hash));
  assert.match(text, /still needs attention/);
});
test('partial verified progress and unavailable artifacts remain distinguishable', () => {
  const text = describeModAcquisition({
    world_id: world, ready: false, artifacts: [
      { ...artifact, state: 'acquired' },
      { ...artifact, mod_id: 'othermod', state: 'unavailable' },
    ], inventory_issues: ['missing_required'],
  }, world);
  assert.match(text, /Downloaded and verified/);
  assert.match(text, /Provider unavailable/);
  assert.throws(() => describeModAcquisition({
    world_id: world, ready: true, artifacts: [artifact], inventory_issues: [],
  }, world));
  assert.throws(() => describeModAcquisition({
    world_id: 'other', ready: false, artifacts: [], inventory_issues: [],
  }, world));
});
test('acquisition is explicit, world-bound, and separate from runtime setup', async () => {
  const calls = [];
  const report = { world_id: world, ready: true, artifacts: [], inventory_issues: [] };
  const backend = createBackendAdapter(async (command, args) => {
    calls.push({ command, args });
    return JSON.stringify(report);
  });
  assert.deepEqual(await backend.mods.acquire(world), report);
  assert.deepEqual(calls, [{ command: 'world_mods_acquire', args: { world } }]);
  assert.match(describeModAcquisition(report, world), /does not install the runtime or authorize hosting/);
});
