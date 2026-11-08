<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { nativeWorldActions } from '../lib/native-world-actions';
  import { WorldFormError } from '../lib/world-actions';
  import { requestLeaveWorld } from '../lib/native-world-actions';

  let minutes: HTMLSelectElement | undefined;
  let addresses: HTMLTextAreaElement | undefined;
  let output: HTMLTextAreaElement | undefined;
  let busy = $state(false);
  let error = $state('');
  let signed = $state('');
  let showSigned = $state(false);
  let leaving = $state(false);
  let leaveError = $state('');
  const resetLeave = () => { if (!leaving) leaveError = ''; };

  function resetInvite() {
    if (busy) return;
    error = '';
    signed = '';
    showSigned = false;
  }

  onMount(() => {
    const old = window.__swarmcraftResetSvelteInvite;
    const oldLeave = window.__swarmcraftResetSvelteLeave;
    window.__swarmcraftResetSvelteInvite = resetInvite;
    window.__swarmcraftResetSvelteLeave = resetLeave;
    return () => {
      if (window.__swarmcraftResetSvelteInvite === resetInvite) {
        window.__swarmcraftResetSvelteInvite = old;
      }
      if (window.__swarmcraftResetSvelteLeave === resetLeave) {
        window.__swarmcraftResetSvelteLeave = oldLeave;
      }
    };
  });

  async function createInvite() {
    if (busy) return;
    busy = true;
    error = '';
    signed = '';
    showSigned = false;
    try {
      const world = (document.getElementById('world') as HTMLInputElement | null)?.value ?? '';
      // Canonical ID comes from the selected-world controller, not the UI.
      const value = await nativeWorldActions().invite(
        world,
        Number(minutes?.value || 60),
        String(addresses?.value ?? '').split(/\r?\n/),
      );
      if (!String(value).trim()) {
        throw new Error('The backend returned no signed invite.');
      }
      signed = String(value);
      showSigned = true;
      await tick();
      output?.focus();
      // Never add signed tokens or embedded addresses to Activity or CI logs.
      window.__swarmcraftSvelteActivity?.append(
        new Date().toLocaleTimeString() + '  Invite\nSigned invite created.',
      );
    } catch (failure) {
      if (failure instanceof WorldFormError) {
        error = failure.message;
        if (failure.field === 'inviteMinutes') minutes?.focus();
      } else {
        error = 'Could not create a signed invite. Check the world and connection, then retry.';
      }
    } finally {
      busy = false;
    }
  }

  async function confirmLeave() {
    if (leaving) return;
    const id = (document.getElementById('world') as HTMLInputElement | null)?.value ?? '';
    if (typeof window.__swarmcraftAfterSvelteLeave !== 'function') {
      leaveError = 'Desktop initialization has not finished.';
      return;
    }
    leaving = true;
    leaveError = '';
    try {
      await requestLeaveWorld(id);
      // Only a successful Rust leave response authorizes clearing UI selection.
      await window.__swarmcraftAfterSvelteLeave();
    } catch {
      // The membership operation may already have completed. Never suggest
      // blindly repeating a signed request after a list-refresh failure.
      leaveError = 'Could not confirm the leave outcome. Refresh Worlds before trying again.';
    } finally {
      leaving = false;
    }
  }

  function preventCloseWhilePending(event: Event) {
    if (busy || leaving) event.preventDefault();
  }
</script>

<dialog id="inviteDialog" class="dialog" oncancel={preventCloseWhilePending}>
    <form id="inviteForm" class="dialog-card" method="dialog">
      <div class="dialog-heading">
        <div>
          <h2>Invite to <span id="inviteWorldName">world</span></h2>
          <p>Create a signed invite. The recipient still joins through canonical membership.</p>
        </div>
        <button id="closeInvite" class="icon-button" value="cancel" type="submit" disabled={busy} aria-label="Close invite dialog">×</button>
      </div>
      <div class="field">
        <label for="inviteMinutes">Invite expires in</label>
        <select id="inviteMinutes" bind:this={minutes}>
          <option value="30">30 minutes</option>
          <option value="60" selected>1 hour</option>
          <option value="360">6 hours</option>
          <option value="1440">24 hours</option>
        </select>
      </div>
      <details class="form-details">
        <summary>Advanced bootstrap addresses</summary>
        <div class="field details-fields">
          <label for="bootstrapAddrs">Bootstrap addresses</label>
          <textarea id="bootstrapAddrs" bind:this={addresses} class="machine-input" rows="3" placeholder="/ip4/…/p2p/…"></textarea>
        </div>
      </details>
      <div id="inviteError" class="form-error" role="alert" hidden={!error}>{error}</div>
      <div id="inviteResultWrap" class="invite-result" hidden={!showSigned}>
        <label for="inviteResult">Signed invite</label>
        <textarea id="inviteResult" bind:this={output} value={signed} rows="5" readonly spellcheck="false"></textarea>
        <button id="copyInvite" class="button button-secondary" type="button">Copy invite</button>
      </div>
      <div class="dialog-actions">
        <button id="cancelInvite" class="button button-secondary" value="cancel" type="submit" disabled={busy}>Done</button>
        <button id="createInvite" class="button button-primary world-required" type="button" disabled={busy} aria-busy={busy} onclick={createInvite}>Create invite</button>
      </div>
    </form>
  </dialog>

<dialog id="sleepDialog" class="dialog">
    <form class="dialog-card" method="dialog">
      <div>
        <h2>Stop this world?</h2>
        <p>SwarmCraft will stop the local Minecraft runtime. This does not itself create a durable sleeping migration state. Background replica storage can continue separately.</p>
      </div>
      <div class="dialog-actions">
        <button class="button button-secondary" value="cancel" type="submit">Cancel</button>
        <button id="confirmSleep" class="button button-warning" value="default" type="button">Stop world</button>
      </div>
    </form>
  </dialog>

<dialog id="leaveDialog" class="dialog" oncancel={preventCloseWhilePending}>
    <form method="dialog" class="dialog-card">
      <div>
        <h2>Leave this world?</h2>
        <p id="leaveDialogText">This stages a signed leave request for the selected world.</p>
      </div>
      <p class="form-error" role="alert" hidden={!leaveError}>{leaveError}</p>
      <div class="dialog-actions">
        <button id="cancelLeave" class="button button-secondary" value="cancel" type="submit" disabled={leaving}>Cancel</button>
        <button id="confirmLeave" class="button button-danger" value="default" type="button" disabled={leaving} aria-busy={leaving} onclick={confirmLeave}>Leave world</button>
      </div>
    </form>
  </dialog>
