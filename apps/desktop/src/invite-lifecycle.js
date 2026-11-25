// Native results are display receipts, not permission to admit or play.
const worldId = value => typeof value === 'string' && /^scworld:[a-f0-9]{64}$/.test(value);
const grantId = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const peerId = value => typeof value === 'string' && /^scpeer:[a-f0-9]{64}$/.test(value);

export function validateInviteToken(value) {
  const token = String(value ?? '').trim();
  if (token.length > 32768 || !/^scinvite:(?:[a-fA-F0-9]{2})+$/.test(token)) {
    throw new Error('Paste a complete signed invitation.');
  }
  return token;
}

export function inspectInviteReceipt(selectedWorld, raw) {
  const value = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (!value || !worldId(selectedWorld) || value.world_id !== selectedWorld
      || !grantId(value.invite_id) || !peerId(value.issuer_peer_id)
      || !Number.isSafeInteger(value.expires_unix_ms) || value.expires_unix_ms < 1
      || value.expires_unix_ms > 8640000000000000
      || typeof value.locally_issued !== 'boolean' || value.reuse !== 'reusable'
      || !['revoked', 'expired', 'lifetime_valid'].includes(value.status)
      || (value.status === 'revoked' && !value.locally_issued)) {
    throw new Error('Invitation receipt is unavailable or does not match the selected world.');
  }
  return Object.freeze({
    world: value.world_id, id: value.invite_id, issuer: value.issuer_peer_id,
    expires: value.expires_unix_ms, status: value.status,
    locallyIssued: value.locally_issued,
    canRequestRevocation: value.locally_issued && value.status !== 'revoked',
  });
}

export function revocationRequest(selection, token, confirmed) {
  if (!selection?.receipt?.canRequestRevocation || confirmed !== true
      || validateInviteToken(token) !== selection.token) {
    throw new Error('Inspect this exact invitation and confirm revocation first.');
  }
  return {
    world: selection.receipt.world,
    invite: selection.token,
    inviteId: selection.receipt.id,
    confirmRevocation: true,
  };
}

export function revokeInviteReceipt(expected, raw) {
  const value = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (!value || value.world_id !== expected.world || value.invite_id !== expected.id
      || value.revoked !== true || typeof value.newly_revoked !== 'boolean') {
    throw new Error('Revocation could not be confirmed. Inspect the invitation before trying again.');
  }
  return value.newly_revoked;
}

export function pendingMembershipMessage(status) {
  if (typeof status !== 'string') return '';
  if (status === 'committed admission; local synchronization pending') {
    return 'Membership approved. Waiting for verified world synchronization.';
  }
  if (status === 'pending; issuer reported invitation revoked') {
    return 'The issuer reported that this invitation was revoked. Membership is not approved. Ask the issuer for a fresh invite and join again with the same role.';
  }
  if (status === 'pending; issuer reported invitation expired'
      || status === 'pending; invitation expired, no new admission can begin') {
    return 'This invitation has expired. No new admission can begin; a previously prepared membership decision may still finish.';
  }
  if (status === 'pending; waiting for signed membership approval') {
    return 'Membership pending. Waiting for signed approval from the current authority and required voters.';
  }
  if (status.startsWith('pending;')) return 'Membership pending. Refresh the world to check approval.';
  return '';
}
