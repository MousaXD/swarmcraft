const MEMBERSHIP_NOTICE = 'Discovery does not grant membership or verify a Minecraft route.';
const FAILURES = Object.freeze({
  not_found: 'No current signed announcement was found. Check the World ID or ask the owner for an invite.',
  provider_unavailable: 'Discovery directory unavailable. Check the configured discovery providers and retry.',
  network_unavailable: 'Discovery network unavailable. Check networking or relay setup and retry.',
  stale: 'Discovery announcement is stale. Ask the world owner to publish a fresh announcement and retry.',
  invalid: 'Discovery announcement could not be verified. Check the World ID and retry.',
  timeout: 'Discovery lookup timed out. Retry when networking is responsive.',
  unavailable: 'Signed discovery could not be checked. Check networking and retry.',
});

export function discoveryUnavailable(state = 'unavailable') {
  const detail = Object.hasOwn(FAILURES, state) ? FAILURES[state] : FAILURES.unavailable;
  return { tone: 'danger', message: `${detail} ${MEMBERSHIP_NOTICE}`, copy: `Directory: ${detail} ${MEMBERSHIP_NOTICE}` };
}

export function formatDiscoveryResult(report, expectedWorldId, now = Date.now()) {
  if (report?.state === 'found') {
    const world = report.world;
    if (!world || world.world_id !== expectedWorldId || typeof world.name !== 'string'
        || !world.name.trim() || world.name.length > 256
        || !['public', 'unlisted', 'private'].includes(world.visibility)
        || !Number.isSafeInteger(world.expires_unix_ms) || world.expires_unix_ms <= now) {
      return { ...discoveryUnavailable('invalid'), tone: 'warning' };
    }
    return { tone: 'safe', message: `${world.name} is ${world.visibility}. Directory returned a fresh signed announcement. ${MEMBERSHIP_NOTICE}`,
      copy: `Directory: fresh signed announcement received. ${MEMBERSHIP_NOTICE}`,
      validUntil: world.expires_unix_ms };
  }
  return { ...discoveryUnavailable(report?.state), tone: 'warning' };
}

// Even a stuck native invocation must not leave a UI indefinitely busy.
export function withDiscoveryDeadline(promise, timeoutMs = 30_000) {
  let timer;
  return Promise.race([promise, new Promise((resolve) => {
    timer = setTimeout(() => resolve({ state: 'timeout' }), timeoutMs);
  })]).finally(() => clearTimeout(timer));
}
