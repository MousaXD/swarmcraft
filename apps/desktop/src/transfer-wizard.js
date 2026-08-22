import './player-experience.js';

export function parsePeerId(identityText) {
  const match = String(identityText || '').match(/^Peer ID:\s*(\S+)$/m);
  return match?.[1] || '';
}

export function parseAuthority(statusText) {
  const match = String(statusText || '').match(/^Authority:\s*(\S+)$/m);
  return match?.[1] || '';
}

export function parseTransferPeers(raw, localPeerId = '') {
  return String(raw || '')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const match = line.match(/^(\S+)\s+authority_eligible=(true|false)\s+banned=(true|false)$/);
      if (!match) return null;
      return {
        peerId: match[1],
        authorityEligible: match[2] === 'true',
        banned: match[3] === 'true',
      };
    })
    .filter((peer) => peer && peer.peerId !== localPeerId && peer.authorityEligible && !peer.banned);
}

export function observeStatusLine(statusText) {
  const authority = parseAuthority(statusText);
  if (!authority || authority === 'unknown') return '';
  const epoch = String(statusText || '').match(/^Epoch:\s*(\S+)$/m)?.[1] || '';
  const epochKnown = Boolean(epoch) && epoch !== 'unknown';
  return `Authority is now ${authority}${epochKnown ? ` at epoch ${epoch}` : ''}.`;
}

const OBSERVE_GUIDANCE =
  'The successor epoch was accepted on this peer. Keep networking online while quorum propagation and the new host runtime readiness complete.';
const OBSERVE_POLL_INTERVAL_MS = 3000;
const OBSERVE_POLL_ATTEMPTS = 20;

const TRANSFER_TOKEN_FIELD_IDS = [
  'preparedTransferToken',
  'acceptedTransferToken',
  'committedTransferToken',
  'successorEpochToken',
  'incomingPreparedTransfer',
  'outgoingAcceptedTransfer',
  'incomingCommittedTransfer',
  'outgoingEpochTransfer',
];

const TRANSFER_RESULT_WRAP_IDS = [
  'preparedTransferWrap',
  'committedTransferWrap',
  'acceptedTransferWrap',
  'epochTransferWrap',
];

// Tokens are world-bound: progress captured for one world must never remain
// visible when the wizard is reopened against a different world.
function clearTransferProgress() {
  for (const id of TRANSFER_TOKEN_FIELD_IDS) {
    const field = document.getElementById(id);
    if (field) field.value = '';
  }
  hideTransferResults();
}

function hideTransferResults() {
  for (const id of TRANSFER_RESULT_WRAP_IDS) {
    const wrap = document.getElementById(id);
    if (wrap) wrap.hidden = true;
  }
  const complete = document.getElementById('sourceTransferComplete');
  if (complete) {
    complete.hidden = true;
    complete.textContent = '';
  }
}

function restoreTransferResults() {
  for (const id of TRANSFER_RESULT_WRAP_IDS) {
    const wrap = document.getElementById(id);
    const output = wrap?.querySelector('textarea');
    if (wrap && output?.value.trim()) wrap.hidden = false;
  }
  const complete = document.getElementById('sourceTransferComplete');
  if (complete?.textContent.trim()) complete.hidden = false;
}

function ensureDialog() {
  let dialog = document.getElementById('transferDialog');
  if (dialog) return dialog;

  dialog = document.createElement('dialog');
  dialog.id = 'transferDialog';
  dialog.className = 'dialog';
  dialog.innerHTML = `
    <div class="dialog-card" role="document">
      <div class="dialog-heading">
        <div>
          <h2 id="transferDialogTitle">Transfer host</h2>
          <p id="transferDialogIntro">Signed handoff keeps the current authority fenced while another member takes over.</p>
        </div>
        <button id="closeTransferDialog" class="icon-button" type="button" aria-label="Close host transfer">×</button>
      </div>

      <div id="transferWizardError" class="form-error" role="alert" hidden></div>
      <p id="transferRoleNote" class="helper"></p>

      <section id="transferSourceFlow" hidden>
        <div class="field">
          <label for="transferTargetPeer">New host</label>
          <select id="transferTargetPeer"></select>
          <p id="transferTargetHint" class="field-help">Choose an authority-eligible member. SwarmCraft highlights the backend-recommended handoff candidate when one is available.</p>
        </div>
        <div class="compact-actions">
          <button id="prepareTransfer" class="button button-primary" type="button">Prepare safe handoff</button>
        </div>
        <div id="preparedTransferWrap" class="invite-result" hidden>
          <label for="preparedTransferToken">1. Prepared token for the new host</label>
          <textarea id="preparedTransferToken" rows="4" readonly spellcheck="false"></textarea>
          <button id="copyPreparedTransfer" class="button button-secondary" type="button">Copy prepared token</button>
          <p class="field-help">On the target device, open Transfer host, paste this token, and accept the handoff.</p>
        </div>

        <div class="field">
          <label for="acceptedTransferToken">2. Accepted token returned by the new host</label>
          <textarea id="acceptedTransferToken" rows="4" spellcheck="false" placeholder="Paste the accepted signed token from the target device"></textarea>
        </div>
        <div class="compact-actions">
          <button id="commitTransfer" class="button button-secondary" type="button">Commit handoff</button>
        </div>
        <div id="committedTransferWrap" class="invite-result" hidden>
          <label for="committedTransferToken">3. Committed token for the new host</label>
          <textarea id="committedTransferToken" rows="4" readonly spellcheck="false"></textarea>
          <button id="copyCommittedTransfer" class="button button-secondary" type="button">Copy committed token</button>
        </div>

        <div class="field">
          <label for="successorEpochToken">4. Successor epoch returned by the new host</label>
          <textarea id="successorEpochToken" rows="4" spellcheck="false" placeholder="Paste the signed successor epoch token"></textarea>
        </div>
        <div class="compact-actions">
          <button id="observeTransfer" class="button button-secondary" type="button">Observe new host epoch</button>
        </div>
        <p id="sourceTransferComplete" class="inline-notice" hidden></p>
      </section>

      <section id="transferTargetFlow" hidden>
        <p class="helper">This device is not the current host. It can accept a handoff only when the signed token names this peer and the exact canonical checkpoint is already present locally.</p>
        <div class="field">
          <label for="incomingPreparedTransfer">1. Prepared token from the current host</label>
          <textarea id="incomingPreparedTransfer" rows="4" spellcheck="false" placeholder="Paste the prepared signed token"></textarea>
        </div>
        <div class="compact-actions">
          <button id="acceptTransfer" class="button button-primary" type="button">Accept handoff</button>
        </div>
        <div id="acceptedTransferWrap" class="invite-result" hidden>
          <label for="outgoingAcceptedTransfer">2. Accepted token for the current host</label>
          <textarea id="outgoingAcceptedTransfer" rows="4" readonly spellcheck="false"></textarea>
          <button id="copyAcceptedTransfer" class="button button-secondary" type="button">Copy accepted token</button>
        </div>

        <div class="field">
          <label for="incomingCommittedTransfer">3. Committed token from the current host</label>
          <textarea id="incomingCommittedTransfer" rows="4" spellcheck="false" placeholder="Paste the committed signed token"></textarea>
        </div>
        <div class="compact-actions">
          <button id="activateTransfer" class="button button-secondary" type="button">Activate new host epoch</button>
        </div>
        <div id="epochTransferWrap" class="invite-result" hidden>
          <label for="outgoingEpochTransfer">4. Successor epoch for quorum peers</label>
          <textarea id="outgoingEpochTransfer" rows="4" readonly spellcheck="false"></textarea>
          <button id="copyEpochTransfer" class="button button-secondary" type="button">Copy successor epoch</button>
          <p class="field-help">Return this token to the previous host and any other quorum peers. Minecraft starts only after normal backend quorum/runtime checks pass.</p>
        </div>
      </section>

      <div class="dialog-actions">
        <button id="doneTransferDialog" class="button button-secondary" type="button">Done</button>
      </div>
    </div>`;
  document.body.append(dialog);
  return dialog;
}

function setError(message = '') {
  const element = document.getElementById('transferWizardError');
  if (!element) return;
  element.textContent = String(message || '');
  element.hidden = !message;
}

function setBusy(button, busy) {
  button.disabled = busy;
  if (busy) button.setAttribute('aria-busy', 'true');
  else button.removeAttribute('aria-busy');
}

async function copyText(value) {
  const text = String(value || '').trim();
  if (!text) return;
  if (!navigator.clipboard?.writeText) throw new Error('Clipboard access is unavailable. Select and copy the token manually.');
  await navigator.clipboard.writeText(text);
}

function roleCopy(isSource, localPeer, authority) {
  if (isSource) return `This device (${localPeer}) is the current authority. The running Minecraft server will pass the Fabric transfer save barrier before a prepared token is returned.`;
  return `Current authority: ${authority || 'unknown'}. This device: ${localPeer || 'unknown'}. Incoming tokens are signature-, generation-, membership-, and checkpoint-validated by the Rust backend.`;
}

export function registerTransferWizard(backend) {
  if (typeof document === 'undefined') return;

  const setup = () => {
    const trigger = document.getElementById('transferHost');
    if (!trigger || trigger.dataset.transferWizardBound === 'true') return;
    trigger.dataset.transferWizardBound = 'true';
    const dialog = ensureDialog();
    let currentWorld = '';

    const close = () => dialog.close();
    document.getElementById('closeTransferDialog').addEventListener('click', close);
    document.getElementById('doneTransferDialog').addEventListener('click', close);
    dialog.addEventListener('cancel', () => setError(''));
    dialog.addEventListener('close', () => stopObservePolling());

    let observeTimer = null;
    const stopObservePolling = () => {
      if (observeTimer !== null) {
        clearInterval(observeTimer);
        observeTimer = null;
      }
    };

    const startObservePolling = (world, complete) => {
      stopObservePolling();
      let attempts = 0;
      observeTimer = setInterval(async () => {
        attempts += 1;
        if (attempts > OBSERVE_POLL_ATTEMPTS) {
          stopObservePolling();
          return;
        }
        try {
          const statusText = await backend.worldStatus(world);
          const line = observeStatusLine(statusText);
          if (line && dialog.open && dialog.dataset.world === world) {
            complete.textContent = `The successor epoch was accepted on this peer. ${line} Keep networking online while quorum propagation completes.`;
            complete.dataset.tone = 'safe';
          }
        } catch {
          if (dialog.open && dialog.dataset.world === world) complete.textContent = OBSERVE_GUIDANCE;
        }
        if (attempts >= OBSERVE_POLL_ATTEMPTS) stopObservePolling();
      }, OBSERVE_POLL_INTERVAL_MS);
    };

    const runStep = async (buttonId, work) => {
      const button = document.getElementById(buttonId);
      setError('');
      setBusy(button, true);
      try {
        return await work();
      } catch (error) {
        setError(error?.message || String(error));
        return null;
      } finally {
        setBusy(button, false);
      }
    };

    document.getElementById('prepareTransfer').addEventListener('click', () => runStep('prepareTransfer', async () => {
      const target = document.getElementById('transferTargetPeer').value;
      if (!target) throw new Error('No authority-eligible transfer target is available.');
      const token = await backend.migration.transferPrepare(currentWorld, target);
      document.getElementById('preparedTransferToken').value = token;
      document.getElementById('preparedTransferWrap').hidden = false;
      return token;
    }));

    document.getElementById('commitTransfer').addEventListener('click', () => runStep('commitTransfer', async () => {
      const token = document.getElementById('acceptedTransferToken').value.trim();
      if (!token) throw new Error('Paste the accepted token returned by the target device.');
      const committed = await backend.migration.transferCommit(currentWorld, token);
      document.getElementById('committedTransferToken').value = committed;
      document.getElementById('committedTransferWrap').hidden = false;
      return committed;
    }));

    document.getElementById('observeTransfer').addEventListener('click', () => runStep('observeTransfer', async () => {
      const token = document.getElementById('successorEpochToken').value.trim();
      if (!token) throw new Error('Paste the signed successor epoch returned by the new host.');
      const result = await backend.migration.transferObserve(currentWorld, token);
      const complete = document.getElementById('sourceTransferComplete');
      complete.textContent = OBSERVE_GUIDANCE;
      complete.dataset.tone = 'safe';
      complete.hidden = false;
      startObservePolling(currentWorld, complete);
      return result;
    }));

    document.getElementById('acceptTransfer').addEventListener('click', () => runStep('acceptTransfer', async () => {
      const token = document.getElementById('incomingPreparedTransfer').value.trim();
      if (!token) throw new Error('Paste the prepared transfer token from the current host.');
      const accepted = await backend.migration.transferAccept(currentWorld, token);
      document.getElementById('outgoingAcceptedTransfer').value = accepted;
      document.getElementById('acceptedTransferWrap').hidden = false;
      return accepted;
    }));

    document.getElementById('activateTransfer').addEventListener('click', () => runStep('activateTransfer', async () => {
      const token = document.getElementById('incomingCommittedTransfer').value.trim();
      if (!token) throw new Error('Paste the committed transfer token from the current host.');
      const epoch = await backend.migration.transferActivate(currentWorld, token);
      document.getElementById('outgoingEpochTransfer').value = epoch;
      document.getElementById('epochTransferWrap').hidden = false;
      return epoch;
    }));

    for (const [buttonId, valueId] of [
      ['copyPreparedTransfer', 'preparedTransferToken'],
      ['copyCommittedTransfer', 'committedTransferToken'],
      ['copyAcceptedTransfer', 'outgoingAcceptedTransfer'],
      ['copyEpochTransfer', 'outgoingEpochTransfer'],
    ]) {
      document.getElementById(buttonId).addEventListener('click', () => {
        copyText(document.getElementById(valueId).value).catch((error) => setError(error?.message || String(error)));
      });
    }

    trigger.addEventListener('click', async (event) => {
      // app.js retains a compatibility listener for older builds. The signed
      // wizard owns transfer-capable builds and stops that legacy listener from
      // attempting a target-less transfer call.
      event.preventDefault();
      event.stopImmediatePropagation();
      setError('');
      currentWorld = document.getElementById('world')?.value.trim() || '';
      if (!currentWorld) {
        setError('Choose a world first.');
        dialog.showModal();
        return;
      }
      if ((dialog.dataset.world || '') !== currentWorld) {
        stopObservePolling();
        clearTransferProgress();
      }
      dialog.dataset.world = currentWorld;

      try {
        await backend.migration.refreshCapabilities();
        if (!backend.migration.capabilities.transfer) throw new Error('Signed manual host transfer is not available in this build.');
        const [identityText, statusText, peerText, readiness] = await Promise.all([
          backend.nodeIdentity(),
          backend.worldStatus(currentWorld),
          backend.worldPeers(currentWorld),
          backend.hostReadiness(currentWorld).catch(() => null),
        ]);
        const localPeer = parsePeerId(identityText);
        const authority = parseAuthority(statusText);
        const isSource = Boolean(localPeer && authority && localPeer === authority);
        const peers = parseTransferPeers(peerText, localPeer);

        document.getElementById('transferDialogTitle').textContent = isSource ? 'Transfer host safely' : 'Receive host handoff';
        document.getElementById('transferRoleNote').textContent = roleCopy(isSource, localPeer, authority);
        document.getElementById('transferSourceFlow').hidden = !isSource;
        document.getElementById('transferTargetFlow').hidden = isSource;
        restoreTransferResults();

        if (isSource) {
          const select = document.getElementById('transferTargetPeer');
          select.replaceChildren();
          const recommended = readiness?.handoffCandidatePeerId || readiness?.successorPeerId || '';
          for (const peer of peers) {
            const option = document.createElement('option');
            option.value = peer.peerId;
            option.textContent = peer.peerId === recommended ? `${peer.peerId} · recommended` : peer.peerId;
            option.selected = peer.peerId === recommended;
            select.append(option);
          }
          const hint = document.getElementById('transferTargetHint');
          if (!peers.length) {
            hint.textContent = 'No other unbanned authority-eligible member is currently present in canonical membership.';
          } else if (recommended) {
            hint.textContent = `Backend Host Readiness recommends ${recommended}. The Rust transfer protocol still validates membership, exact canonical checkpoint, signatures, and generation fencing at every step.`;
          } else {
            hint.textContent = 'Host Readiness has not identified a preferred successor. A transfer can remain safe but may leave the world offline until the target runtime/mod setup and quorum are ready.';
          }
          document.getElementById('prepareTransfer').disabled = !peers.length;
        }

        dialog.showModal();
      } catch (error) {
        setError(error?.message || String(error));
        dialog.showModal();
      }
    }, { capture: true });
  };

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', setup, { once: true });
  else setup();
}
