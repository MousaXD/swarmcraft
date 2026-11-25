import assert from 'node:assert/strict';
import test from 'node:test';
import { validateInviteToken, inspectInviteReceipt, revocationRequest, revokeInviteReceipt, pendingMembershipMessage } from '../src/invite-lifecycle.js';
import { createBackendAdapter } from '../src/backend-adapter.js';

const world = 'scworld:' + 'a'.repeat(64);
const other = 'scworld:' + 'b'.repeat(64);
const id = 'c'.repeat(64);
const token = 'scinvite:abcd';
const response = {
  world_id: world, invite_id: id, issuer_peer_id: 'scpeer:' + 'd'.repeat(64),
  expires_unix_ms: 1800000000000, reuse: 'reusable', status: 'lifetime_valid', locally_issued: true,
};

test('invitation management rejects malformed, unbounded and different-world inputs', () => {
  assert.equal(validateInviteToken('  ' + token + '  '), token);
  for (const invalid of ['', 'scinvite:a', 'scinvite:zz', '--help', 'scinvite:' + 'ab'.repeat(32768)]) {
    assert.throws(() => validateInviteToken(invalid));
  }
  const receipt = inspectInviteReceipt(world, JSON.stringify(response));
  assert.equal(receipt.canRequestRevocation, true);
  assert.throws(() => inspectInviteReceipt(other, response));
  for (const change of [
    { invite_id: 'not-a-hash' }, { issuer_peer_id: 'not-a-peer' }, { expires_unix_ms: Infinity },
    { locally_issued: 'true' }, { reuse: 'one-use' }, { status: 'admitted' },
    { status: 'revoked', locally_issued: false },
  ]) assert.throws(() => inspectInviteReceipt(world, { ...response, ...change }));
  assert.equal(inspectInviteReceipt(world, { ...response, locally_issued: false }).canRequestRevocation, false);
  assert.equal(inspectInviteReceipt(world, { ...response, status: 'revoked' }).canRequestRevocation, false);
});

test('revocation pins exact token, world, grant and explicit confirmation', async () => {
  const receipt = inspectInviteReceipt(world, response);
  const selection = { token, receipt };
  assert.throws(() => revocationRequest(selection, token, false));
  assert.throws(() => revocationRequest(selection, 'scinvite:eeee', true));
  const payload = revocationRequest(selection, token, true);
  assert.deepEqual(payload, { world, invite: token, inviteId: id, confirmRevocation: true });
  const calls = [];
  const backend = createBackendAdapter(async (command, args) => { calls.push({command,args}); return 'receipt'; });
  await assert.rejects(backend.revokeInvite({ ...payload, confirmRevocation: false }));
  assert.equal(calls.length, 0);
  await backend.inspectInvite(token);
  await backend.revokeInvite(payload);
  assert.deepEqual(calls, [{command:'inspect_invite',args:{invite:token}}, {command:'revoke_invite',args:payload}]);
  assert.equal(revokeInviteReceipt(receipt, { world_id:world, invite_id:id, revoked:true, newly_revoked:true }), true);
  assert.equal(revokeInviteReceipt(receipt, { world_id:world, invite_id:id, revoked:true, newly_revoked:false }), false);
  for (const bad of [{world_id:other,invite_id:id,revoked:true,newly_revoked:true},
      {world_id:world,invite_id:id,revoked:false,newly_revoked:true}]) {
    assert.throws(() => revokeInviteReceipt(receipt,bad));
  }
});

test('pending membership messages do not claim authority or play readiness', () => {
  assert.match(pendingMembershipMessage('pending; issuer reported invitation revoked'), /not approved/);
  assert.match(pendingMembershipMessage('pending; invitation expired, no new admission can begin'), /expired/);
  assert.match(pendingMembershipMessage('committed admission; local synchronization pending'), /verified world synchronization/);
  assert.match(pendingMembershipMessage('pending; waiting for signed membership approval'), /required voters/);
  assert.equal(pendingMembershipMessage(undefined), '');
});
