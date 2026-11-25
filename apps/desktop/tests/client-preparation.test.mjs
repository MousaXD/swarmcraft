import assert from 'node:assert/strict';
import test from 'node:test';
import { createBackendAdapter } from '../src/backend-adapter.js';
import { normalizeClientProfile, clientPreparationInstructions } from '../src/client-preparation.js';

const world = 'scworld:' + 'a'.repeat(64);
const profile = {
  world_id: world,
  compatibility_fingerprint: 'b'.repeat(64),
  minecraft_version: '26.1.2',
  fabric_loader_version: '0.19.3',
  minimum_java_major: 25,
  required_mods: [{ mod_id: 'sharedmod', version: '1.0', artifact_hash: 'c'.repeat(64) }],
  inventory: 'not_checked',
  issues: [],
};

test('the native response is bound to one world and one explicit inspection mode', () => {
  assert.deepEqual(normalizeClientProfile(JSON.stringify(profile), world, false), profile);
  for (const value of [
    { ...profile, world_id: 'scworld:' + 'd'.repeat(64) },
    { ...profile, compatibility_fingerprint: 'unverified' },
    { ...profile, minimum_java_major: 0 },
    { ...profile, inventory: 'exact_match' },
    { ...profile, inventory: 'needs_review' },
    { ...profile, required_mods: [...profile.required_mods, ...profile.required_mods] },
    { ...profile, required_mods: [{ mod_id: 'sharedmod', version: '1.0', artifact_hash: '' }] },
  ]) {
    assert.throws(() => normalizeClientProfile(value, world, false), /mismatched/);
  }
  assert.throws(() => normalizeClientProfile(profile, world, true));
  assert.equal(normalizeClientProfile({ ...profile, inventory: 'exact_match' }, world, true).inventory, 'exact_match');
  assert.throws(() => normalizeClientProfile({ ...profile, inventory: 'exact_match', issues: [{ code: 'missing_required', mod_id: 'sharedmod' }] }, world, true));
  assert.throws(() => normalizeClientProfile({ ...profile, inventory: 'needs_review', issues: [{ code: 'unknown', mod_id: null }] }, world, true));
  assert.equal(normalizeClientProfile({ ...profile, inventory: 'needs_review', issues: [{ code: 'hash_mismatch', mod_id: 'sharedmod' }] }, world, true).inventory, 'needs_review');
});

test('launcher handoff directs an isolated instance without launching or asserting readiness', () => {
  for (const [launcher, name] of [['official', 'Minecraft Launcher'], ['prism', 'Prism Launcher'], ['other', 'Java Edition launcher']]) {
    const instructions = clientPreparationInstructions(profile, launcher);
    assert.ok(instructions.includes(name));
    assert.match(instructions, /26\.1\.2/);
    assert.match(instructions, /0\.19\.3/);
    assert.match(instructions, /Java 25/);
    assert.match(instructions, /separate/);
    assert.match(instructions, /existing account/);
    assert.match(instructions, /server-only SwarmCraft bridge/);
    assert.match(instructions, /current verified reconnect instructions/);
    assert.match(instructions, /not login, runtime, dependency or connection proof/);
  }
});

test('backend forwards only selected world and optional mods directory', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, args) => {
    calls.push({ command, args });
    return JSON.stringify(profile);
  });
  await backend.clientProfile(world);
  await backend.clientProfile(world, '/fixture/instance/mods');
  assert.deepEqual(calls, [
    { command: 'client_profile', args: { world, modsDir: null } },
    { command: 'client_profile', args: { world, modsDir: '/fixture/instance/mods' } },
  ]);
});
