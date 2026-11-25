import { withDiscoveryDeadline } from './discovery-diagnostics.js';

const PEER = /^scpeer:[0-9a-f]{64}$/;
const KEY = /^[0-9a-f]{64}$/;
const WORLD = /^scworld:[0-9a-f]{64}$/;
const CONTACT_LIMIT = 256;
const OBSERVATION_TTL = 30_000;
const byId = id => document.getElementById(id);
const make = (tag, value, className) => {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
};
const text = (value, max) => typeof value === 'string' ? value.replace(/[\u0000-\u001f\u007f]/g, ' ').slice(0,max) : '';

export function normalizeContactSettings(raw) {
  if (!raw || !PEER.test(raw.own_peer_id || '') || !KEY.test(raw.own_public_key_hex || '')
      || typeof raw.presence_enabled !== 'boolean' || !Array.isArray(raw.contacts)
      || raw.contacts.length > CONTACT_LIMIT) throw new Error('Invalid contact settings');
  const seen = new Set();
  const contacts = raw.contacts.map(contact => {
    if (!contact || !PEER.test(contact.peer_id || '') || !KEY.test(contact.public_key_hex || '')
        || contact.peer_id === raw.own_peer_id || seen.has(contact.peer_id)
        || typeof contact.label !== 'string' || !contact.label.trim()
        || typeof contact.share_presence !== 'boolean' || contact.presence !== 'unknown'
        || !Array.isArray(contact.shared_worlds)) throw new Error('Invalid contact');
    seen.add(contact.peer_id);
    return {
      peer_id: contact.peer_id, label: text(contact.label, 96),
      share_presence: contact.share_presence,
      shared_worlds: contact.shared_worlds.filter(w => w && WORLD.test(w.world_id || '')
        && typeof w.name === 'string').slice(0, 8).map(w => ({world_id: w.world_id, name: text(w.name, 128)})),
    };
  });
  return { own_peer_id: raw.own_peer_id, own_public_key_hex: raw.own_public_key_hex,
    presence_enabled: raw.presence_enabled, contacts };
}

export function normalizePresence(raw, peer, now = Date.now()) {
  const unknown = (message) => ({ message, expires: null });
  if (raw?.state === 'timeout') return unknown('Not confirmed · check timed out.');
  if (!raw || !PEER.test(peer) || raw.peer_id !== peer) return unknown('Not confirmed · invalid response.');
  if (raw.state === 'online') {
    const observed = raw.observed_unix_ms;
    const expires = raw.expires_unix_ms;
    if (!Number.isSafeInteger(observed) || !Number.isSafeInteger(expires) || observed <= 0
        || observed > now || expires <= now || expires <= observed || expires - observed > OBSERVATION_TTL) {
      return unknown('Not confirmed · observation expired or invalid.');
    }
    return { message: 'Online observed at ' + new Date(observed).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit',second:'2-digit'})
      + ' · not a live game route.', expires };
  }
  if (raw.state === 'network_unavailable') return unknown('Not confirmed · network unavailable.');
  if (raw.state === 'stale') return unknown('Not confirmed · stale response.');
  if (raw.state === 'invalid') return unknown('Not confirmed · invalid response.');
  // Old "offline", refusal, hide and absence of a response do not establish
  // offline. Never display raw errors, remote addresses, or peer metadata.
  return unknown('Not confirmed · hidden, not shared or unreachable.');
}

export function contactInput(peer, key, label) {
  peer = String(peer || '').trim().toLowerCase();
  key = String(key || '').trim().toLowerCase();
  label = String(label || '').trim();
  if (!PEER.test(peer) || !KEY.test(key) || !label || new TextEncoder().encode(label).length > 96
      || /[\u0000-\u001f\u007f]/.test(label)) throw new Error('Enter a complete peer ID, public key and label (up to 96 UTF-8 bytes).');
  return { peer, publicKey:key, label };
}

export function installFriends(invoke, openInvite) {
  const panel = byId('friendsPanel');
  if (!panel || panel.dataset.bound || typeof invoke !== 'function') return;
  panel.dataset.bound = 'true';
  const list = byId('friendsContacts');
  const refresh = byId('friendsRefresh');
  const mode = byId('friendsModeToggle');
  const service = byId('friendsServiceToggle');
  const add = byId('friendAdd');
  const settingsNotice = byId('friendsStatus');
  let settings = null;
  let running = null;
  let operation = false;
  let context = 0;
  let serviceTimer;
  let serviceRevision = 0;
  let expiryTimer;
  let pendingRemoval = null;
  const observations = new Map();
  let probe = null;

  function notice(message, tone = 'neutral') {
    settingsNotice.textContent = message;
    settingsNotice.dataset.tone = tone;
  }
  function controls() {
    const valid = !!settings;
    refresh.disabled = operation;
    mode.disabled = operation || !valid;
    add.disabled = operation || !valid;
    service.disabled = operation || running === null || (!running && (!settings?.presence_enabled
      || !settings.contacts.some(c=>c.share_presence)));
    mode.textContent = settings?.presence_enabled ? 'Hide presence' : 'Enable sharing';
    service.textContent = running ? 'Stop service' : 'Start service';
    byId('friendsModeText').textContent = valid ? settings.presence_enabled
      ? 'Sharing enabled for allowed contacts only.' : 'Hidden · no new presence proofs.' : 'Settings not verified.';
    byId('friendsServiceStatus').textContent = running === true
      ? 'Desktop-owned process started · reachability not verified.'
      : running === false ? 'Desktop-owned presence service stopped.' : 'Presence service status not verified.';
    list.setAttribute('aria-busy', String(operation));
    for (const button of list.querySelectorAll('button')) {
      const isProbe = button.dataset.action === 'probe';
      button.disabled = operation || (isProbe && probe !== null);
    }
  }
  function retireObservations() {
    context++;
    observations.clear();
    clearTimeout(expiryTimer);
    pendingRemoval = null;
  }
  function render() {
    const focus = document.activeElement;
    const focused = focus?.closest?.('li[data-peer]');
    const focusedPeer = focused?.dataset.peer;
    const focusedAction = focus?.dataset.action;
    list.replaceChildren();
    if (!settings?.contacts.length) {
      const row = make('li', settings ? 'No contacts yet. Add a verified peer ID and public key below.' : 'Contact list not verified.', 'field-help');
      list.append(row);
    } else for (const contact of settings.contacts) {
      const row = make('li', '', 'friend-row');
      row.dataset.peer = contact.peer_id;
      const body = make('div', '', 'friend-copy');
      body.append(make('strong', contact.label), make('span', contact.peer_id, 'machine-line'));
      const presence = observations.get(contact.peer_id);
      body.append(make('span', presence?.message || (probe === contact.peer_id ? 'Checking signed presence…' : 'Presence not checked.'), 'friend-presence'));
      const share = contact.share_presence && settings.presence_enabled ? 'Allowed to see your presence'
        : contact.share_presence ? 'Allowed when sharing is enabled · hidden now' : 'Presence not shared with this contact';
      body.append(make('span', share, 'field-help'));
      if (contact.shared_worlds.length) {
        body.append(make('p', 'Locally known shared worlds: ' + contact.shared_worlds.map(w=>w.name).join(', '), 'field-help'));
      }
      const actions = make('div', '', 'friend-actions');
      const button = (label, action, handler) => {
        const b = make('button', label, 'button button-secondary button-small');
        b.type = 'button'; b.dataset.action = action;
        b.setAttribute('aria-label', label + ' for ' + contact.label);
        b.addEventListener('click', handler);
        actions.append(b);
      };
      button('Check presence','probe',()=>checkPresence(contact.peer_id));
      button(contact.share_presence?'Withdraw sharing':'Allow sharing','share',()=>mutate('friend_share_presence',{peer:contact.peer_id,enabled:!contact.share_presence},'Contact sharing updated.'));
      if (pendingRemoval === contact.peer_id) {
        body.append(make('p','Remove this device? Future presence access is revoked; existing world membership and bearer invitations are unchanged.','field-help'));
        button('Confirm removal','confirm',()=>mutate('friend_remove',{peer:contact.peer_id},'Contact removed. Membership unchanged.'));
        button('Cancel','cancel',()=>{pendingRemoval=null;render();});
      } else button('Remove','remove',()=>{pendingRemoval=contact.peer_id;render(); [...list.children].find(el=>el.dataset.peer===contact.peer_id)?.querySelector('[data-action="cancel"]')?.focus();});
      row.append(body,actions);list.append(row);
    }
    if (focusedPeer) {
      const row = [...list.children].find(el=>el.dataset.peer===focusedPeer);
      const replacement = [...(row?.querySelectorAll('button')||[])].find(el=>el.dataset.action===focusedAction);
      if (replacement) replacement.focus();
      else refresh.focus();
    }
    byId('friendOwnPeer').value = settings?.own_peer_id || '';
    byId('friendOwnKey').value = settings?.own_public_key_hex || '';
    controls();
  }
  async function readSettings() {
    settings = normalizeContactSettings(await invoke('friend_settings', {}));
  }
  async function readService() {
    const revision = ++serviceRevision;
    try {
      const raw = await invoke('friend_service_status', {});
      if (revision === serviceRevision) running = typeof raw?.process_started === 'boolean' ? raw.process_started : null;
    } catch { if (revision === serviceRevision) running = null; }
  }
  async function load() {
    if (operation || panel.hidden) return;
    operation = true;
    retireObservations();
    settings = null;
    running = null;
    notice('Reading local contacts and privacy settings…');
    render();
    try {
      await Promise.all([readSettings(), readService()]);
      notice(settings.contacts.length + ' local contact(s). No presence is inferred from the contact list.');
    } catch {
      settings = null;
      notice('Contact settings could not be checked. Refresh before changing privacy.','warning');
    } finally { operation=false; render(); }
  }
  async function mutate(command, args, message) {
    if (operation || !settings) return;
    operation = true;
    retireObservations();
    controls();
    notice('Updating local contact settings…');
    try {
      const raw = await invoke(command,args);
      // Re-read the actual store, not the clicked toggle or a hopeful result.
      await Promise.all([readSettings(),readService()]);
      if (command === 'friend_remove' && (typeof raw?.removed !== 'boolean' || raw.world_membership_changed !== false
          || settings.contacts.some(c=>c.peer_id===args.peer))) throw new Error('Removal not confirmed');
      if (command === 'friend_add' && !settings.contacts.some(c=>c.peer_id===args.peer)) throw new Error('Add not confirmed');
      if (command === 'friend_presence_mode' && settings.presence_enabled !== args.enabled) throw new Error('Mode not confirmed');
      if (command === 'friend_share_presence' && !settings.contacts.some(c=>c.peer_id===args.peer && c.share_presence===args.enabled)) throw new Error('Sharing not confirmed');
      if (command === 'friend_add') byId('friendAddForm').reset();
      notice(message);
    } catch {
      settings = null;
      notice('Change could not be confirmed. Refresh the local settings before retrying.','warning');
    } finally { operation=false;render(); }
  }
  async function changeService() {
    if (operation || service.disabled) return;
    operation=true;serviceRevision++;controls();
    try {
      await invoke(running?'friend_service_stop':'friend_service_start',{});
      await readService();
      notice(running?'Presence process started. Remote reachability must be checked separately.':'Desktop-owned presence service stopped.');
    } catch {
      await readService();
      notice('Presence process change could not be confirmed. Check service status and directory setup.','warning');
    } finally { operation=false;controls(); }
  }
  function expire() {
    clearTimeout(expiryTimer);
    const now = Date.now();
    for (const [id, value] of observations) if (value.expires && value.expires <= now) {
      observations.set(id,{message:'Not confirmed · observation expired.',expires:null});
    }
    render();
    const next = [...observations.values()].map(v=>v.expires).filter(Boolean);
    if (next.length) expiryTimer=setTimeout(expire,Math.max(1,Math.min(...next)-Date.now()+1));
  }
  async function checkPresence(peer) {
    if (operation || probe || !settings?.contacts.some(c=>c.peer_id===peer)) return;
    const generation = context;
    probe=peer;
    observations.delete(peer);
    render();
    try {
      const raw=await withDiscoveryDeadline(invoke('friend_probe',{peer}),20_000);
      if (generation===context && !panel.hidden && settings?.contacts.some(c=>c.peer_id===peer)) {
        observations.set(peer,normalizePresence(raw,peer));
      }
    } catch {
      if (generation===context && !panel.hidden) observations.set(peer,{message:'Not confirmed · presence check failed.',expires:null});
    } finally { probe=null;expire(); }
  }
  refresh.addEventListener('click',load);
  mode.addEventListener('click',()=>mutate('friend_presence_mode',{enabled:!settings.presence_enabled},
    settings.presence_enabled?'Presence hidden. Future proofs denied.':'Sharing enabled for explicitly allowed contacts.'));
  service.addEventListener('click',changeService);
  byId('friendsInvite').addEventListener('click',openInvite);
  byId('friendAddForm').addEventListener('submit',event=>{
    event.preventDefault();
    if (operation || !settings) return;
    try {
      const args=contactInput(byId('friendPeer').value,byId('friendKey').value,byId('friendLabel').value);
      if (args.peer===settings.own_peer_id) throw new Error('Do not add your own device.');
      void mutate('friend_add',args,'Contact saved. Adding alone does not grant presence access.');
    } catch (error) { notice(error.message,'warning'); }
  });
  document.addEventListener('swarmcraftcontextchanged',()=>{
    if (panel.hidden) {
      retireObservations();
      clearInterval(serviceTimer);
      serviceTimer=null;
      return;
    }
    if (!serviceTimer) {
      void load();
      serviceTimer=setInterval(async()=>{
        if (!panel.hidden && !operation) { await readService();controls(); }
      },5000);
    }
  });
}
