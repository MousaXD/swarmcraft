import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createBackendAdapter } from '../src/backend-adapter.js';
import { parseAuthority, parsePeerId, parseTransferPeers } from '../src/transfer-wizard.js';

const srcRoot = new URL('../src/', import.meta.url);
const desktopRoot = new URL('../', import.meta.url);

async function text(name) {
  return readFile(new URL(name, srcRoot), 'utf8');
}

async function desktopText(name) {
  return readFile(new URL(name, desktopRoot), 'utf8');
}

test('manual transfer capability exposes only the signed participant-specific backend stages', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload = {}) => {
    calls.push({ command, payload });
    if (command === 'migration_capabilities') return 'status,transfer,wake';
    if (command === 'list_worlds') return '';
    if (command === 'manual_transfer_step') return `${payload.action}-token`;
    return 'ok';
  });

  await backend.listWorlds();
  assert.equal(backend.migration.capabilities.transfer, true);
  assert.equal(await backend.migration.transferPrepare('scworld:test', 'peer-b'), 'prepare-token');
  assert.equal(await backend.migration.transferAccept('scworld:test', 'prepared'), 'accept-token');
  assert.equal(await backend.migration.transferCommit('scworld:test', 'accepted'), 'commit-token');
  assert.equal(await backend.migration.transferActivate('scworld:test', 'committed'), 'activate-token');
  assert.equal(await backend.migration.transferObserve('scworld:test', 'epoch'), 'observe-token');

  assert.deepEqual(
    calls.filter(({ command }) => command === 'manual_transfer_step').map(({ payload }) => payload),
    [
      { world: 'scworld:test', action: 'prepare', value: 'peer-b' },
      { world: 'scworld:test', action: 'accept', value: 'prepared' },
      { world: 'scworld:test', action: 'commit', value: 'accepted' },
      { world: 'scworld:test', action: 'activate', value: 'committed' },
      { world: 'scworld:test', action: 'observe', value: 'epoch' },
    ],
  );
});

test('transfer capability remains fail-closed when the packaged backend lacks the complete command set', async () => {
  const calls = [];
  const backend = createBackendAdapter(async (command, payload = {}) => {
    calls.push({ command, payload });
    if (command === 'migration_capabilities') return 'status,wake';
    if (command === 'list_worlds') return '';
    return 'ok';
  });

  await backend.listWorlds();
  assert.equal(backend.migration.capabilities.transfer, false);
  await assert.rejects(
    backend.migration.transferPrepare('scworld:test', 'peer-b'),
    /not available in this build/i,
  );
  assert.equal(calls.some(({ command }) => command === 'manual_transfer_step'), false);
});

test('transfer wizard derives source role and filters unsafe target membership choices', () => {
  assert.equal(parsePeerId('Peer ID: peer-a\nPublic key: deadbeef'), 'peer-a');
  assert.equal(parseAuthority('World: Demo\nAuthority: peer-a\nEpoch: 3'), 'peer-a');

  const peers = parseTransferPeers([
    'peer-a authority_eligible=true banned=false',
    'peer-b authority_eligible=true banned=false',
    'peer-c authority_eligible=false banned=false',
    'peer-d authority_eligible=true banned=true',
    'garbage line',
  ].join('\n'), 'peer-a');

  assert.deepEqual(peers, [
    { peerId: 'peer-b', authorityEligible: true, banned: false },
  ]);
});

test('Desktop transfer bridge waits for the Fabric checkpoint publication before exposing a prepared token', async () => {
  const commands = await desktopText('src-tauri/src/transfer_commands.rs');
  assert.match(commands, /"transfer-prepare"/);
  assert.match(commands, /"transfer-export"/);
  assert.match(commands, /TRANSFER_WAIT_ATTEMPTS/);
  assert.match(commands, /tokio::time::sleep\(TRANSFER_WAIT_INTERVAL\)/);
  assert.match(commands, /Minecraft was not force-killed/);
  assert.match(commands, /"transfer-accept"/);
  assert.match(commands, /"transfer-commit"/);
  assert.match(commands, /"transfer-activate"/);
  assert.match(commands, /"transfer-observe"/);
});

test('Tauri advertises transfer only when every signed backend stage is packaged', async () => {
  const main = await desktopText('src-tauri/src/main.rs');
  const commands = await desktopText('src-tauri/src/transfer_commands.rs');
  assert.match(main, /mod transfer_commands;/);
  assert.match(main, /transfer_supported\(&app\)\.await/);
  assert.match(main, /supported\.push\("transfer"\)/);
  assert.match(main, /manual_transfer_step/);
  assert.match(commands, /const TRANSFER_COMMANDS: \[&str; 6\]/);
});

test('transfer presentation delegates authority semantics to backend commands rather than recreating fencing in JavaScript', async () => {
  const wizard = await text('transfer-wizard.js');
  const adapter = await text('backend-adapter.js');

  for (const method of ['transferPrepare', 'transferAccept', 'transferCommit', 'transferActivate', 'transferObserve']) {
    assert.match(wizard, new RegExp(`backend\\.migration\\.${method}`));
  }
  assert.match(adapter, /manual_transfer_step/);
  assert.doesNotMatch(wizard, /nextEpoch|next_epoch|fencingToken|fencing_token/i);
  assert.doesNotMatch(wizard, /quorum\s*=|memberCount\s*\/\s*2|member_count\s*\/\s*2|Math\.(?:floor|ceil)\([^\n]*quorum/i);
  assert.doesNotMatch(wizard, /saveEpoch|writeEpoch|setAuthority|authorityPeerId\s*=|authority_peer_id\s*=/i);
});
