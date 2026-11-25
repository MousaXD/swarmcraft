import { discoveryTime, normalizeDiscoveryCard, withDiscoveryDeadline } from './discovery-diagnostics.js';

export const DISCOVERY_RESULT_CAP = 32; // Runtime V1 default; no remote cursor.
export const DISCOVERY_PAGE_SIZE = 6;
const STATES = new Set(['ok', 'empty', 'partial', 'network_unavailable', 'provider_unavailable', 'timeout']);

export function normalizePublicSearch(report, now = Date.now()) {
  if (report?.state === 'timeout') return { cards: [], state: 'timeout', truncated: false };
  if (!report || !STATES.has(report.state) || !Array.isArray(report.results)
      || report.results.length > DISCOVERY_RESULT_CAP
      || typeof report.truncated !== 'boolean'
      || !Number.isSafeInteger(report.providers_seen) || report.providers_seen < 0 || report.providers_seen > 64
      || !Number.isSafeInteger(report.providers_responded) || report.providers_responded < 0
      || report.providers_responded > report.providers_seen
      || (['ok', 'empty'].includes(report.state) && (report.providers_responded === 0
        || report.providers_responded !== report.providers_seen || report.truncated))
      || (report.results.length && report.providers_responded === 0)
      || (report.state === 'empty' && report.results.length)
      || (report.state === 'ok' && !report.results.length)
      || (!['ok', 'partial'].includes(report.state) && report.results.length)) {
    return { cards: [], state: 'invalid', truncated: false };
  }
  const counts = new Map();
  for (const raw of report.results) counts.set(raw?.world_id, (counts.get(raw?.world_id) || 0) + 1);
  const cards = report.results.map(raw => normalizeDiscoveryCard(raw, now))
    .filter(card => card?.visibility === 'public' && counts.get(card.world_id) === 1);
  return {
    cards, truncated: report.truncated,
    state: cards.length !== report.results.length ? 'partial' : report.state,
    responded: report.providers_responded, seen: report.providers_seen,
  };
}

export function searchSummary(result) {
  const scope = 'Only a bounded directory snapshot, not all worlds.';
  const suffix = result.truncated ? ' Search limit reached; narrow the search or refresh.' : '';
  if (result.state === 'ok' || (result.state === 'partial' && result.cards.length)) {
    const count = result.cards.length;
    return `${count} verified public listing${count === 1 ? '' : 's'} · ${result.responded} of ${result.seen} providers responded. ${result.state === 'partial' ? 'Some results could not be verified. ' : ''}${scope}${suffix}`;
  }
  if (result.state === 'empty') return `No matching public listings from responding providers. Try a different search or browse all. ${scope}`;
  if (result.state === 'partial') return `Discovery incomplete. No verified matching listings were returned; this does not prove none exist.${suffix}`;
  if (result.state === 'provider_unavailable') return 'No discovery provider available. Check directory setup, or join with a signed invite.';
  if (result.state === 'network_unavailable') return 'Discovery network unavailable. Check connectivity and retry.';
  if (result.state === 'timeout') return 'Directory search timed out. Retry when networking is responsive.';
  return 'Directory response could not be checked. Retry discovery.';
}

export function discoveryPage(cards, page, now = Date.now()) {
  const active = cards.filter(card => card.expires_unix_ms > now);
  const pages = Math.max(1, Math.ceil(active.length / DISCOVERY_PAGE_SIZE));
  const index = Math.max(0, Math.min(Number.isInteger(page) ? page : 0, pages - 1));
  return {
    cards: active.slice(index * DISCOVERY_PAGE_SIZE, (index + 1) * DISCOVERY_PAGE_SIZE),
    total: active.length, index, pages, expired: cards.length - active.length,
  };
}

const byId = id => document.getElementById(id);
const text = (tag, value, className) => {
  const element = document.createElement(tag);
  element.textContent = value;
  if (className) element.className = className;
  return element;
};

export function installDiscoveryLobby(invoke) {
  const form = byId('publicWorldForm');
  if (!form || form.dataset.bound) return;
  form.dataset.bound = 'true';
  const query = byId('publicWorldQuery');
  const search = byId('publicWorldSearch');
  const status = byId('publicWorldStatus');
  const list = byId('publicWorldResults');
  const pagination = byId('publicWorldPages');
  const previous = byId('publicWorldPrev');
  const next = byId('publicWorldNext');
  const pageLabel = byId('publicWorldPage');
  let result = null;
  let page = 0;
  let revision = 0;
  let busy = false;
  let expiryTimer;

  const report = (message, tone = 'neutral') => {
    if (status.textContent !== message) status.textContent = message;
    status.dataset.tone = tone;
  };
  function setBusy(value) {
    busy = value;
    search.disabled = value;
    search.textContent = value ? 'Searching…' : 'Search';
    list.setAttribute('aria-busy', String(value));
  }
  function reset(message) {
    revision++;
    clearTimeout(expiryTimer);
    result = null;
    page = 0;
    list.replaceChildren();
    pagination.hidden = true;
    previous.disabled = next.disabled = true;
    report(message);
  }
  function inspect(card) {
    // A result action is a read-only lookup of the same ID. It cannot call
    // join_world or open a route. Refuse expired rows even before timer fires.
    if (card.expires_unix_ms <= Date.now()) { render(); return; }
    const input = byId('joinWorldId');
    input.closest('details').open = true;
    input.value = card.world_id;
    // Both exact-ID owners invalidate their prior result on input.
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const button = byId('joinWorldIdButton');
    if (button.disabled) { input.focus(); return; }
    button.focus();
    button.click();
  }
  function render() {
    clearTimeout(expiryTimer);
    if (!result) return;
    const now = Date.now();
    const current = discoveryPage(result.cards, page, now);
    page = current.index;
    const focusedRow = document.activeElement?.closest?.('.discovery-result');
    const focusedId = focusedRow?.dataset.worldId;
    const fragments = document.createDocumentFragment();
    for (const card of current.cards) {
      const item = document.createElement('li');
      item.className = 'discovery-result';
      item.dataset.worldId = card.world_id;
      const body = document.createElement('div');
      body.className = 'discovery-result-copy';
      const name = text('strong', card.name);
      name.title = card.name;
      body.append(name,
        text('span', `Minecraft ${card.minecraft_version} · ${card.loader_id} ${card.loader_version}`, 'meta-text'),
        text('span', `Verified at ${discoveryTime(card.verified_unix_ms)} · Expires ${discoveryTime(card.expires_unix_ms)}`, 'discovery-freshness'),
        text('span', card.world_id, 'machine-line'));
      if (card.description) body.append(text('p', card.description, 'field-help'));
      const metadata = [card.approximate_region, ...card.tags].filter(Boolean).join(' · ');
      if (metadata) body.append(text('p', metadata, 'field-help'));
      const action = document.createElement('div');
      action.className = 'discovery-result-actions';
      const check = text('button', 'Check listing', 'button button-secondary button-small');
      check.type = 'button';
      check.setAttribute('aria-label', 'Check listing for ' + card.name);
      check.addEventListener('click', () => inspect(card));
      action.append(check, text('span', 'Signed invite required', 'field-help'));
      item.append(body, action);
      fragments.append(item);
    }
    list.replaceChildren(fragments);
    pagination.hidden = current.pages <= 1;
    previous.disabled = page === 0;
    next.disabled = page >= current.pages - 1;
    pageLabel.textContent = `Page ${page + 1} of ${current.pages} · ${current.total} listings`;
    const shown = { ...result, cards: result.cards.filter(card => card.expires_unix_ms > now) };
    report(current.expired
      ? `${searchSummary(shown.state === 'ok' && !shown.cards.length ? { ...shown, state: 'partial' } : shown)} ${current.expired} listing(s) expired. Search again for fresh results.`
      : searchSummary(shown), current.expired || result.state !== 'ok' && result.state !== 'empty' ? 'warning' : 'neutral');
    // Don't lose keyboard focus when a timer retires the current row/page.
    if (focusedId) {
      const replacement = [...list.children].find(row => row.dataset.worldId === focusedId)?.querySelector('button');
      (replacement || search).focus();
    }
    const expiries = shown.cards.map(card => card.expires_unix_ms);
    if (expiries.length) expiryTimer = setTimeout(render, Math.max(1, Math.min(...expiries) - Date.now() + 1));
  }
  query.addEventListener('input', () => reset('Search changed. Run the new search for verified listings.'));
  document.addEventListener('swarmcraftcontextchanged', () => {
    if (byId('discoverPanel').hidden) reset('Search by name or tag, or leave the field blank to browse.');
  });
  previous.addEventListener('click', () => { if (!busy && result) { page--; render(); if (previous.disabled) next.focus(); } });
  next.addEventListener('click', () => { if (!busy && result) { page++; render(); if (next.disabled) previous.focus(); } });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (busy) return;
    const term = query.value.trim();
    // Bound bytes as well as characters; the full native wire query has
    // its own encoded-size limit. Never pass arbitrary oversized strings.
    if (new TextEncoder().encode(term).length > 128) {
      reset('Search is too long. Use a shorter name or tag (up to 128 UTF-8 bytes).');
      query.focus();
      return;
    }
    reset('Searching and verifying signed public announcements…');
    const request = revision;
    const hadFocus = document.activeElement === search;
    setBusy(true);
    try {
      const raw = await withDiscoveryDeadline(invoke('discovery_search', { query: term || null }));
      if (revision !== request || query.value.trim() !== term || byId('discoverPanel').hidden) return;
      result = normalizePublicSearch(raw);
      render();
    } catch {
      if (revision === request) report('Directory search failed. Check networking and retry.', 'warning');
    } finally {
      setBusy(false);
      if (hadFocus && document.activeElement === document.body && !byId('discoverPanel').hidden) search.focus();
    }
  });
}
