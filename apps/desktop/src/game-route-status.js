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

const DIAGNOSTIC_STATES = Object.freeze({
  gateway_unavailable: ['Local gateway unavailable', 'Retry the selected world. Check that networking is running and the reserved local port is available.'],
  peer_link_unavailable: ['Host peer link unavailable', 'The accepted remote authority has no authenticated peer connection. Check that the host is online and retry networking or relay setup.'],
  host_proof_unavailable: ['Host proof unavailable', 'A complete current signed host, quorum, consent and Fabric proof could not be verified. Check the host and the world safety state.'],
  route_check_timed_out: ['Route check timed out', 'The bounded signed-route check did not finish. Retry when the host and network are responsive.'],
  authority_changed: ['Authority changed during verification', 'The previous proof is no longer current. Retry for the newly accepted host and checkpoint.'],
});

const SESSION_STATES = Object.freeze({
  checking_route: 'A recent Minecraft connection attempt was checking the signed route.',
  opening_stream: 'A recent Minecraft connection attempt was opening the authenticated game stream.',
  route_refused: 'The last Minecraft connection attempt was refused before stream admission.',
  route_timed_out: 'The last Minecraft connection attempt timed out during route verification.',
  stream_admission_failed: 'The last Minecraft connection attempt failed during authenticated stream admission or server connection.',
  stream_interrupted: 'The last admitted game stream was interrupted. Reconnect in Minecraft after a fresh route check.',
  session_ended: 'The last admitted game stream ended. This does not confirm a successful Minecraft login.',
  overloaded: 'The local game-stream limit rejected the last connection attempt. Retry after other attempts finish.',
});

function normalizeRecentSession(report, now) {
  const session = report.recent_session;
  if (session == null) return null;
  if (!session || !Object.hasOwn(SESSION_STATES, session.state)
      || !Number.isSafeInteger(session.age_ms) || session.age_ms < 0) {
    throw new Error('Invalid recent game-stream observation');
  }
  if (session.age_ms >= 30_000) return null;
  return Object.freeze({ label: SESSION_STATES[session.state], validUntil: now + 30_000 - session.age_ms });
}

/** Diagnostics are read-only, world-bound and time-limited. Copy only
 * allowlisted local descriptions, never raw native errors or addresses. */
export function connectionDiagnosis(route, worldId, now = Date.now()) {
  const current = Boolean(worldId && route?.worldId === worldId
    && Number.isSafeInteger(route.diagnosticValidUntil) && route.diagnosticValidUntil > now);
  const currentSession = current && route.recentSession?.validUntil > now;
  return Object.freeze({
    route: current ? route.label : 'Current connection not yet checked',
    detail: current ? route.verified ? 'Signed route proof was verified; actual game login is not confirmed.' : route.detail
      : 'Retry to obtain a fresh signed-route diagnostic.',
    session: currentSession ? route.recentSession.label : 'No recent Minecraft connection attempt observed.',
  });
}

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
  const recentSession = normalizeRecentSession(report, now);
  if (report.state !== 'verified') {
    if (!Object.hasOwn(ROUTE_STATES, report.state) || report.route_verified !== false) {
      throw new Error('Game-route proof reported an inconsistent state');
    }
    if (report.diagnostic != null && !Object.hasOwn(DIAGNOSTIC_STATES, report.diagnostic)) {
      throw new Error('Unknown game-route diagnostic');
    }
    const [label, detail] = DIAGNOSTIC_STATES[report.diagnostic] || ROUTE_STATES[report.state];
    return Object.freeze({ state: report.state, worldId: expectedWorldId, label, detail, verified: false,
      diagnosticValidUntil: now + 10_000, recentSession, connectionTarget: null, automaticReconnect: false });
  }

  const { verified_at_unix_ms: proofTime, valid_until_unix_ms: expiry } = report;
  if (report.diagnostic != null || report.route_verified !== true || report.quorum_verified !== true
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
    diagnosticValidUntil: effectiveUntil,
    recentSession,
    automaticReconnect: false,
  });
}

function checkpointDiagnostics(world, readiness, now) {
  const fresh = readiness?.worldId === world?.id && isSafeInt(readiness?.validUntil)
    && readiness.validUntil > now;
  const report = fresh ? readiness.raw?.checkpoint : null;
  const cachedNumber = world?.status?.['Latest snapshot'];
  const cachedBytes = world?.status?.['Checkpoint bytes'];
  const cachedHash = world?.status?.['Checkpoint manifest'];
  const parseCounter = (value) => /^\d+$/.test(String(value ?? '')) && isSafeInt(Number(value)) ? Number(value) : null;
  const current = report && isHash(report.manifest_hash) && isSafeInt(report.snapshot_number)
    ? report : null;
  const hash = current?.manifest_hash || (isHash(cachedHash) ? cachedHash : null);
  const number = current?.snapshot_number ?? parseCounter(cachedNumber);
  const bytes = current ? current.logical_bytes : parseCounter(cachedBytes);
  const ack = current?.last_replica_ack;
  const knownAck = ack && isHash(ack.manifest_hash) && isSafeInt(ack.snapshot_number)
    && isSafeInt(ack.age_ms) && /^scpeer:[a-f\d]{64}$/i.test(ack.peer_id);
  const exact = knownAck && number === ack.snapshot_number && hash === ack.manifest_hash;
  let gap = 'Replica lag unknown';
  if (exact) gap = 'Latest checkpoint acknowledged';
  else if (knownAck && number > ack.snapshot_number) gap = `${number - ack.snapshot_number} checkpoint numbers ahead of last acknowledgment`;
  else if (knownAck) gap = 'Acknowledgment does not match the current head';
  const age = knownAck ? Math.floor((ack.age_ms + Math.max(0, now - readiness.raw.generated_unix_ms)) / 1000) : null;
  return {
    hash,
    fresh: Boolean(current),
    rows: [
      { label: 'Accepted checkpoint', value: hash && number !== null
        ? `#${number} · ${hash.slice(0, 12)}…${isSafeInt(bytes) ? ` · ${(bytes / 1048576).toFixed(2)} MiB` : ' · size unknown'}${current ? '' : ' · last local read'}`
        : 'Exact checkpoint unavailable', kind: 'neutral', title: hash || undefined },
      { label: 'Replica acknowledgment', value: knownAck
        ? `#${ack.snapshot_number} · ${ack.manifest_hash.slice(0, 12)}… · ${Number.isFinite(age) ? `${age}s ago` : 'age unknown'} · ${gap}. Historical receipt, not current recovery proof.`
        : 'No exact acknowledgment observed in the current daemon session', kind: exact ? 'neutral' : 'warning', title: knownAck ? `${ack.peer_id} · ${ack.manifest_hash}` : undefined },
      { label: 'Progress at risk', value: knownAck
        ? `${gap}. Changes after the last replica-acknowledged checkpoint may be lost; unsaved gameplay progress is not measured.`
        : 'No replica-acknowledged recovery point is known. Local-only saves and unsaved gameplay may be at risk.', kind: 'warning' },
    ],
  };
}

/** Presence, local verification and live authority are deliberately separate. */
export function gameReadinessCheckpoints({ world = null, route = UNVERIFIED_GAME_ROUTE,
  localVerification = null, readiness = null } = {}, now = Date.now()) {
  const snapshots = world?.snapshots;
  const safety = String(world?.status?.Safety || '').trim();
  const conflict = /conflict|divergen/i.test(safety);
  const degraded = /solo|degraded/i.test(safety);
  const canonical = /^canonical$/i.test(safety);
  const live = isUsableGameRoute(route, world?.id, safety, now);
  const diagnostics = checkpointDiagnostics(world, readiness, now);
  const localCheckCompleted = Boolean(world?.id && localVerification?.worldId === world.id
    && !localVerification?.manifestHash && localVerification?.checkpoint === world.status?.['Latest snapshot']);
  const exactLocalCheck = Boolean(world?.id && localVerification?.worldId === world.id
    && diagnostics.hash && localVerification?.manifestHash === diagnostics.hash);

  const checkpoint = Number.isSafeInteger(snapshots) && snapshots > 0
    ? `${snapshots} stored · not proof of current head`
    : 'No stored copy reported';
  return Object.freeze([
    { label: 'Saved copy', value: checkpoint, kind: snapshots > 0 ? 'neutral' : 'warning' },
    { label: 'Local integrity', value: exactLocalCheck ? `${diagnostics.fresh ? 'Exact' : 'Last-read'} checkpoint checked locally · not live recovery proof` : localCheckCompleted ? 'Local check completed · exact head not independently bound' : 'Current checkpoint not checked this session', kind: 'neutral' },
    { label: 'Canonical quorum', value: conflict ? 'Conflicting history' : degraded ? 'Reduced safety' : live ? 'Fresh quorum verified' : canonical ? 'History reported · live quorum unverified' : 'Not verified', kind: conflict ? 'danger' : degraded ? 'warning' : live ? 'safe' : 'neutral' },
    { label: 'Hosting consent', value: live ? 'Verified for current host' : 'Current host consent unverified', kind: live ? 'safe' : 'neutral' },
    { label: 'Live Fabric server', value: live ? 'Fresh runtime proof verified' : 'Not verified by live IPC', kind: live ? 'safe' : 'neutral' },
    { label: 'Minecraft route', value: live ? `Verified · ${route.connectionTarget}` : conflict ? 'Not verified · conflicting history' : degraded ? 'Not verified · reduced safety' : route?.verified ? 'Not verified for this world safety state' : route?.label || 'Not verified', kind: live ? 'safe' : route?.state === 'blocked' || conflict ? 'danger' : degraded ? 'warning' : 'neutral' },
    ...(world?.id ? diagnostics.rows : []),
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
    if (checkpoint.title) value.title = checkpoint.title;
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
