/**
 * Minecraft Java's server connection is separate from SwarmCraft's authenticated
 * control plane. A local migration-status.json is NOT a signed connection ticket,
 * and game_endpoint is a configured hint, not a validated route.
 *
 * This is intentionally presentation-only: no socket, auto-join or endpoint
 * selection may be derived from the migration status on this path.
 */
import { isUsableGameRoute } from './game-route-status.js';

const outcome = (state, title, detail, tone = 'warning', connectionTarget = null) => Object.freeze({
  state,
  title,
  detail,
  tone,
  automaticReconnect: false,
  connectionTarget,
});

const UNKNOWN = outcome(
  'unknown',
  'Minecraft connection not verified',
  'SwarmCraft cannot yet verify a client connection to this world. Minecraft does not automatically follow SwarmCraft host changes.',
);

function phaseOf(value) {
  return String(value || '').trim().toLowerCase().replace(/-/g, '_');
}

function isLocalRuntimeReportBoundToWorld(status, worldId) {
  return status.world_id === worldId
    && typeof status.authority_peer_id === 'string'
    && status.authority_peer_id.trim().length > 0
    && Number.isSafeInteger(status.epoch) && status.epoch >= 0
    && Number.isSafeInteger(status.fencing_token) && status.fencing_token >= 0
    && typeof status.snapshot_hash === 'string'
    && status.snapshot_hash.trim().length > 0;
}

/**
 * Classifies the visible client outcome without manufacturing a trusted game
 * address. Even a fully populated local Ready report does not authenticate the
 * address of the remote Minecraft server or prove the client reconnected.
 *
 * @param {unknown} status raw machine-local migration report
 * @param {string} expectedWorldId world currently selected in the UI
 * @param {string} safety separately observed local world-safety classification
 * @param {object} route independently verified, fresh native game-route result
 */
export function clientContinuityFromMigration(status, expectedWorldId, safety, route = null) {
  if (!expectedWorldId) return UNKNOWN;

  // Canonical history in the UI is still not a live routing certificate, but
  // known divergence or degraded history must never show a positive hint.
  if (/conflict|divergen/i.test(String(safety || ''))) {
    return outcome(
      'conflict',
      'World history needs review',
      'Competing world histories are preserved. Do not reconnect to either server until the conflict is resolved through SwarmCraft.',
      'danger',
    );
  }
  // A valid fresh route supersedes a possibly stale local migration report;
  // that report can describe an old host even after the new one is verified.
  // This is only reachable via the independently validated native route API,
  // never through migration_status.game_endpoint or a manually entered IP.
  if (isUsableGameRoute(route, expectedWorldId, safety)) {
    return outcome(
      'route_verified',
      'Authenticated Minecraft route verified',
      route.detail,
      'safe',
      route.connectionTarget,
    );
  }
  if (!status || typeof status !== 'object' || Array.isArray(status)) return UNKNOWN;
  if (status.world_id !== expectedWorldId) {
    return outcome(
      'world_mismatch',
      'Connection status does not match this world',
      'A different or missing world identity was returned. SwarmCraft will not use this host report for reconnection.',
      'danger',
    );
  }

  const phase = phaseOf(status.phase);
  if (phase === 'superseded') {
    return outcome(
      'superseded',
      'Previous host is no longer authoritative',
      'Do not reconnect to the old host. Wait for a new canonical authority and a separately verified Minecraft connection route.',
      'danger',
    );
  }
  if (phase === 'failed') {
    return outcome(
      'failed',
      'Host recovery failed',
      'Minecraft reconnection is unavailable. Keep the client disconnected from old hosts while the recovery error is investigated.',
      'danger',
    );
  }
  if (phase === 'blocked') {
    const quorum = /quorum|majority/i.test(String(status.failure_reason || ''));
    return outcome(
      quorum ? 'quorum_unavailable' : 'blocked',
      quorum ? 'No safe successor quorum' : 'Host recovery blocked',
      quorum
        ? 'The surviving peers cannot authorize a replacement host. Do not join an old or unconfirmed server.'
        : 'SwarmCraft has not authorized a usable replacement Minecraft server. Resolve the reported blocker before reconnecting.',
      'danger',
    );
  }
  if (phase === 'sleeping') {
    return outcome(
      'sleeping',
      'World is sleeping',
      'The world is not accepting Minecraft connections. Request a safe wake through SwarmCraft rather than reusing an old server address.',
    );
  }
  if (/degraded|solo/i.test(String(safety || ''))) {
    return outcome(
      'reduced_safety',
      'World safety is reduced',
      'SwarmCraft has not established normal canonical safety. Do not treat a previously used server address as a safe failover destination.',
    );
  }

  if (!/^canonical$/i.test(String(safety || '').trim())) {
    return outcome(
      'safety_unverified',
      'World authority safety not verified',
      'SwarmCraft has not confirmed the selected world has canonical history. Do not use an old server address as a failover destination.',
    );
  }

  if (phase === 'ready') {
    if (status.runtime_ready !== true || !isLocalRuntimeReportBoundToWorld(status, expectedWorldId)) {
      return outcome(
        'unverified',
        'Server readiness is not confirmed',
        'This local report does not establish a world-bound, generation-bound running server. No Minecraft client address is safe to suggest.',
        'danger',
      );
    }
    return outcome(
      'route_unverified',
      'Minecraft reconnection is not verified',
      'The last local runtime report reached Ready for this world and generation, but the report can remain after a crash and does not prove the server is still running. SwarmCraft has no authenticated, client-reachable game route or automatic Minecraft reconnect. Do not reuse an old address or join an unverified successor.',
    );
  }
  if ([
    'waiting_for_authority', 'waiting_for_quorum', 'selecting_snapshot',
    'preparing_runtime', 'restoring_world', 'launching_runtime', 'verifying_fabric',
    'checkpointing', 'awaiting_transfer_acceptance',
  ].includes(phase)) {
    return outcome(
      'waiting',
      'Waiting for a safe Minecraft host',
      'Recovery or transfer is in progress. Remain disconnected until the accepted host has started and a trusted Minecraft connection route is available.',
    );
  }
  return UNKNOWN;
}

/**
 * Svelte compatibility: the candidate app mounts legacy Rust-backed controls
 * but may not have the legacy continuity-panel markup yet. Preserve the
 * visible fail-closed message without assuming either frontend owns the DOM.
 */
export function ensureClientContinuityPanel(doc) {
  const existingTitle = doc.getElementById('clientContinuityTitle');
  const existingDetail = doc.getElementById('clientContinuityDetail');
  if (existingTitle && existingDetail) return { title: existingTitle, detail: existingDetail };
  // Never manufacture a second element when only one target ID exists.
  if (existingTitle || existingDetail) return null;

  const actions = doc.getElementById('migrationCard')?.querySelector('.compact-actions');
  if (!actions?.parentNode) return null;

  const panel = doc.createElement('section');
  panel.className = 'safety-panel neutral';
  panel.setAttribute('aria-labelledby', 'clientContinuityTitle');
  const content = doc.createElement('div');
  const sectionLabel = doc.createElement('span');
  sectionLabel.className = 'section-label';
  sectionLabel.textContent = 'Minecraft client connection';
  const title = doc.createElement('h4');
  title.id = 'clientContinuityTitle';
  title.textContent = 'Minecraft connection not verified';
  const detail = doc.createElement('p');
  detail.id = 'clientContinuityDetail';
  detail.setAttribute('role', 'status');
  detail.setAttribute('aria-live', 'polite');
  detail.textContent = 'SwarmCraft cannot yet verify a Minecraft client connection.';
  content.append(sectionLabel, title, detail);
  panel.append(content);
  actions.parentNode.insertBefore(panel, actions);
  return { title, detail };
}
