const MEMBERSHIP_NOTICE = 'Discovery does not grant membership or verify a Minecraft route.';
const MAX_ANNOUNCEMENT_LIFETIME_MS = 600_000;
const CLOCK_SKEW_MS = 60_000;
const FAILURES = Object.freeze({
  not_found: 'No current signed announcement was found. Check the World ID or ask the owner for an invite.',
  provider_unavailable: 'Discovery directory unavailable. Check the configured discovery providers and retry.',
  network_unavailable: 'Discovery network unavailable. Check networking or relay setup and retry.',
  stale: 'Discovery announcement is stale. Ask the world owner to publish a fresh announcement and retry.',
  invalid: 'Discovery announcement could not be verified. Check the World ID and retry.',
  timeout: 'Discovery lookup timed out. Retry when networking is responsive.',
  unavailable: 'Signed discovery could not be checked. Check networking and retry.',
});

export function normalizeWorldId(value) {
  const id = String(value ?? '').trim();
  return /^scworld:[0-9a-fA-F]{64}$/.test(id) ? id.toLowerCase() : '';
}

export function discoveryUnavailable(state = 'unavailable') {
  const detail = Object.hasOwn(FAILURES, state) ? FAILURES[state] : FAILURES.unavailable;
  return { tone: 'danger', message: `${detail} ${MEMBERSHIP_NOTICE}`, copy: `Directory: ${detail} ${MEMBERSHIP_NOTICE}` };
}

// Rust is the cryptographic verifier. The renderer checks the shape and
// time of its projection, never signatures, grants, or routes itself.
export function normalizeDiscoveryCard(world, now = Date.now(), expected = null) {
  if (!world || !normalizeWorldId(world.world_id) || world.world_id !== normalizeWorldId(world.world_id)
      || (expected !== null && world.world_id !== expected)
      || !['public', 'unlisted'].includes(world.visibility)
      || world.membership_policy !== 'invite_only' || world.join_action !== 'invite_required'
      || typeof world.name !== 'string' || !world.name.trim() || world.name.length > 4096
      || !['issued_unix_ms', 'expires_unix_ms', 'verified_unix_ms'].every(key =>
        Number.isSafeInteger(world[key]) && world[key] >= 0)
      || world.expires_unix_ms <= now || world.expires_unix_ms <= world.issued_unix_ms
      || world.expires_unix_ms - world.issued_unix_ms > MAX_ANNOUNCEMENT_LIFETIME_MS
      || world.verified_unix_ms > now || world.verified_unix_ms >= world.expires_unix_ms
      || world.issued_unix_ms > world.verified_unix_ms + CLOCK_SKEW_MS
      || now - world.verified_unix_ms > MAX_ANNOUNCEMENT_LIFETIME_MS + CLOCK_SKEW_MS
      || !['minecraft_version', 'loader_id', 'loader_version'].every(key =>
        typeof world[key] === 'string' && world[key].trim() && world[key].length <= 128)) return null;
  const display = (value, max) => {
    const text = typeof value === 'string' ? value.replace(/[\u0000-\u001f\u007f]/g, ' ').trim() : '';
    return text.length > max ? text.slice(0, max) + '…' : text;
  };
  return Object.freeze({
    world_id: world.world_id, name: display(world.name, 256) || 'Unnamed listing',
    visibility: world.visibility, description: display(world.description, 320),
    tags: Array.isArray(world.tags) ? world.tags.slice(0, 6).map(tag => display(tag, 40)).filter(Boolean) : [],
    approximate_region: display(world.approximate_region, 64),
    minecraft_version: display(world.minecraft_version, 64),
    loader_id: display(world.loader_id, 64), loader_version: display(world.loader_version, 64),
    issued_unix_ms: world.issued_unix_ms, verified_unix_ms: world.verified_unix_ms,
    expires_unix_ms: world.expires_unix_ms,
  });
}

export function discoveryTime(ms) {
  return new Date(ms).toLocaleTimeString([], {hour: '2-digit', minute: '2-digit', second: '2-digit'});
}

export function formatDiscoveryResult(report, expectedWorldId, now = Date.now()) {
  if (report?.state === 'found') {
    const world = normalizeDiscoveryCard(report.world, now, expectedWorldId);
    if (!world) return { ...discoveryUnavailable('invalid'), tone: 'warning' };
    return {
      tone: 'neutral',
      message: `${world.name} is ${world.visibility}. Verified signed announcement at ${discoveryTime(world.verified_unix_ms)}; listing expires at ${discoveryTime(world.expires_unix_ms)}. Minecraft ${world.minecraft_version} · ${world.loader_id} ${world.loader_version}. Signed invite required. ${MEMBERSHIP_NOTICE}`,
      copy: `Directory: signed announcement verified at lookup. ${MEMBERSHIP_NOTICE}`,
      validUntil: Math.min(world.expires_unix_ms, world.verified_unix_ms + 30_000),
    };
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
