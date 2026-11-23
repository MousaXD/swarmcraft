/**
 * Read-only Desktop view of the native game-route proof. Native Rust enforces
 * world binding, actual loopback address and evidence lifetime; we repeat the
 * critical checks here so stale or malformed renderer data never becomes a
 * clickable/copied Minecraft address. No cached migration Ready, raw IP,
 * discovered member or local runtime setting can satisfy this contract.
 */
const ROUTE_STATES = Object.freeze({
  waiting_for_quorum: ['Waiting for quorum', 'A fresh canonical majority is required before Minecraft can reconnect.'],
  syncing_checkpoint: ['Syncing saved world', 'The current accepted world checkpoint has not yet been verified on the next host.'],
  restoring_runtime: ['Starting Minecraft host', 'The authorized host has not yet proved live Fabric server readiness.'],
  verifying_route: ['Verifying game route', 'SwarmCraft is checking the signed host route and game transport. Do not use an old server address.'],
  blocked: ['Minecraft route blocked', 'A required authority, consent, runtime or route check was refused.'],
  unavailable: ['Minecraft route not verified', 'No fresh, authenticated game-route proof is available from the local daemon.'],
});

export const UNVERIFIED_GAME_ROUTE = Object.freeze({
  state: 'unavailable',
  label: ROUTE_STATES.unavailable[0],
  detail: ROUTE_STATES.unavailable[1],
  verified: false,
  connectionTarget: null,
  automaticReconnect: false,
});

const isSafeInt = (value) => Number.isSafeInteger(value) && value >= 0;
const isHash = (value) => typeof value === 'string' && /^[a-f\d]{64}$/i.test(value);
const isLocalPort = (value) => {
  const match = typeof value === 'string' && /^127\.0\.0\.1:([1-9]\d{0,4})$/.exec(value);
  return Boolean(match && Number(match[1]) <= 65535);
};

/** Do not outlive the independent five-second authority proof. */
export function isUsableGameRoute(route, expectedWorldId, safety, now = Date.now()) {
  return Boolean(expectedWorldId && route?.worldId === expectedWorldId
    && route.verified === true && isLocalPort(route.connectionTarget)
    && Number.isSafeInteger(route.validUntil) && route.validUntil > now
    && /^canonical$/i.test(String(safety || '').trim()));
}

export function normalizeGameRouteStatus(report, expectedWorldId, now = Date.now()) {
  if (!report || typeof report !== 'object' || Array.isArray(report)) return UNVERIFIED_GAME_ROUTE;
  if (!expectedWorldId || report.world_id !== expectedWorldId) {
    throw new Error('Game-route proof did not match the selected world');
  }
  if (report.state !== 'verified') {
    if (!Object.hasOwn(ROUTE_STATES, report.state) || report.route_verified !== false) {
      throw new Error('Game-route proof reported an inconsistent state');
    }
    const [label, detail] = ROUTE_STATES[report.state];
    return Object.freeze({ state: report.state, label, detail, verified: false,
      connectionTarget: null, automaticReconnect: false });
  }

  const { verified_at_unix_ms: proofTime, valid_until_unix_ms: expiry } = report;
  if (report.route_verified !== true || report.quorum_verified !== true
      || report.hosting_consent_verified !== true || report.server_ready !== true
      || !isLocalPort(report.local_gateway)
      || !isSafeInt(report.epoch) || !isSafeInt(report.fencing_token)
      || !isHash(report.snapshot_hash)
      || typeof report.authority_peer_id !== 'string'
      || !/^scpeer:[a-f\d]{64}$/i.test(report.authority_peer_id)
      || !isSafeInt(proofTime) || !isSafeInt(expiry)
      || !Number.isFinite(now) || proofTime > now + 2000
      || now - proofTime > 5000 || expiry <= now || expiry <= proofTime
      || expiry - proofTime > 15000) {
    throw new Error('Game-route proof is missing, stale, expired or not loopback-only');
  }
  const effectiveUntil = Math.min(expiry, proofTime + 5000);
  if (effectiveUntil <= now) {
    throw new Error('The game-route proof is no longer fresh');
  }
  return Object.freeze({
    state: 'verified',
    worldId: expectedWorldId,
    label: 'Authenticated game route verified',
    detail: `Minecraft Multiplayer address: ${report.local_gateway}. Reconnect manually after a host change; the gateway verifies each new connection. This does not confirm a Minecraft login.`,
    verified: true,
    connectionTarget: report.local_gateway,
    // Never extend the live authority proof using a longer route ticket TTL.
    validUntil: effectiveUntil,
    automaticReconnect: false,
  });
}

/** Presence, local verification and live authority are deliberately separate. */
export function gameReadinessCheckpoints({ world = null, route = UNVERIFIED_GAME_ROUTE,
  localVerification = null } = {}, now = Date.now()) {
  const snapshots = world?.snapshots;
  const safety = String(world?.status?.Safety || '').trim();
  const conflict = /conflict|divergen/i.test(safety);
  const degraded = /solo|degraded/i.test(safety);
  const canonical = /^canonical$/i.test(safety);
  const live = isUsableGameRoute(route, world?.id, safety, now);
  const localCheckCompleted = localVerification?.worldId === world?.id
    && localVerification?.checkpoint === world?.status?.['Latest snapshot'];

  const checkpoint = Number.isSafeInteger(snapshots) && snapshots > 0
    ? `${snapshots} stored · not proof of current head`
    : 'No stored copy reported';
  return Object.freeze([
    { label: 'Saved copy', value: checkpoint, kind: snapshots > 0 ? 'neutral' : 'warning' },
    { label: 'Local integrity', value: localCheckCompleted ? 'Local check completed · exact head not independently bound' : 'Not checked this session', kind: 'neutral' },
    { label: 'Canonical quorum', value: conflict ? 'Conflicting history' : degraded ? 'Reduced safety' : live ? 'Fresh quorum verified' : canonical ? 'History reported · live quorum unverified' : 'Not verified', kind: conflict ? 'danger' : degraded ? 'warning' : live ? 'safe' : 'neutral' },
    { label: 'Hosting consent', value: live ? 'Verified for current host' : 'Current host consent unverified', kind: live ? 'safe' : 'neutral' },
    { label: 'Live Fabric server', value: live ? 'Fresh runtime proof verified' : 'Not verified by live IPC', kind: live ? 'safe' : 'neutral' },
    { label: 'Minecraft route', value: live ? `Verified · ${route.connectionTarget}` : conflict ? 'Not verified · conflicting history' : degraded ? 'Not verified · reduced safety' : route?.verified ? 'Not verified for this world safety state' : route?.label || 'Not verified', kind: live ? 'safe' : route?.state === 'blocked' || conflict ? 'danger' : degraded ? 'warning' : 'neutral' },
  ]);
}

/**
 * Present the *same* fail-closed native decision in the primary player flow
 * and the advanced handoff details, for both shipped HTML and staged Svelte.
 * Controls are merely display affordances: the click handler independently
 * validates world identity, canonical safety and proof expiry again.
 */
export function updateGameRouteControls(doc, route, expectedWorldId, safety, now = Date.now()) {
  const verified = isUsableGameRoute(route, expectedWorldId, safety, now);
  const safeState = verified ? route
    : route?.verified ? UNVERIFIED_GAME_ROUTE : route || UNVERIFIED_GAME_ROUTE;
  const target = verified ? route.connectionTarget : 'No verified address';
  const hint = verified
    ? 'Open Minecraft Java Multiplayer and join this exact loopback address. A new login is required after a host change.'
    : 'A saved checkpoint or a running host does not prove a playable connection. Wait for a fresh signed game-route proof.';
  const primaryLabel = verified ? `Verified · ${route.connectionTarget}` : safeState.label;
  const primaryDetail = verified
    ? 'Use Reconnect instructions to start a new Minecraft session. An actual successful Minecraft login has not yet been observed.'
    : safeState.detail;
  const updateText = (selector, value) => {
    for (const element of doc.querySelectorAll(selector)) {
      if (element.textContent !== value) element.textContent = value;
    }
  };
  for (const button of doc.querySelectorAll('[data-reconnect-world]')) {
    if (button.disabled !== !verified) button.disabled = !verified;
  }
  updateText('[data-verified-gateway-address]', target);
  updateText('[data-verified-gateway-hint]', hint);
  updateText('[data-primary-route-state]', primaryLabel);
  updateText('[data-primary-route-detail]', primaryDetail);
  const overview = doc.getElementById('playerJourneyOverview');
  if (overview && overview.dataset.routeVerified !== String(verified)) {
    overview.dataset.routeVerified = String(verified);
  }
  return verified;
}

/** The same DOM helper works in shipped HTML and staged Svelte compatibility. */
export function updateGameReadinessChecklist(doc, checkpoints) {
  const card = doc.getElementById('migrationCard');
  if (!card) return false;
  let list = card.querySelector('[data-game-readiness-checkpoints]');
  if (!list) {
    const statusPanel = card.querySelector('.safety-panel');
    if (!statusPanel?.parentNode) return false;
    list = doc.createElement('dl');
    list.className = 'game-readiness-checkpoints';
    list.dataset.gameReadinessCheckpoints = 'true';
    list.setAttribute('aria-label', 'World verification checkpoints');
    statusPanel.parentNode.insertBefore(list, statusPanel.nextSibling);
  }
  list.replaceChildren();
  for (const checkpoint of checkpoints) {
    const row = doc.createElement('div');
    row.className = 'game-readiness-checkpoint';
    const key = doc.createElement('dt');
    key.textContent = checkpoint.label;
    const value = doc.createElement('dd');
    value.textContent = checkpoint.value;
    value.dataset.tone = checkpoint.kind;
    row.append(key, value);
    list.append(row);
  }
  return true;
}

/**
 * The shipping HTML has this control; staged Svelte intentionally retains the
 * old element IDs without hardcoding new agent-specific markup. Supply the
 * same presentation dynamically when Svelte mounts, with no duplicate IDs,
 * hidden endpoint fallback or second controller.
 */
export function ensureGameReconnectActions(doc) {
  const card = doc.getElementById('migrationCard');
  if (!card) return false;
  const existingButton = card.querySelector('[data-reconnect-world]');
  const existingAddress = card.querySelector('[data-verified-gateway-address]');
  const existingHint = card.querySelector('[data-verified-gateway-hint]');
  if (existingButton && existingAddress && existingHint) return true;
  if (existingButton || existingAddress || existingHint) return false;

  const after = card.querySelector('[data-game-readiness-checkpoints]') || card.querySelector('.safety-panel');
  if (!after?.parentNode) return false;
  const actions = doc.createElement('div');
  actions.className = 'game-route-actions';
  actions.setAttribute('aria-label', 'Verified Minecraft reconnect');
  const information = doc.createElement('div');
  const label = doc.createElement('span');
  label.className = 'section-label';
  label.textContent = 'Local Minecraft address';
  const address = doc.createElement('strong');
  address.className = 'machine-line';
  address.dataset.verifiedGatewayAddress = 'true';
  address.textContent = 'No verified address';
  const hint = doc.createElement('p');
  hint.className = 'field-help';
  hint.dataset.verifiedGatewayHint = 'true';
  hint.textContent = 'A signed game-route proof is required before connecting.';
  information.append(label, address, hint);
  const button = doc.createElement('button');
  button.type = 'button';
  button.className = 'button button-secondary';
  button.dataset.reconnectWorld = 'true';
  button.textContent = 'Reconnect instructions';
  button.disabled = true;
  actions.append(information, button);
  after.parentNode.insertBefore(actions, after.nextSibling);
  return true;
}
