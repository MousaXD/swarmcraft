<script lang="ts">
  import { nativeWorldActions } from '../lib/native-world-actions';
  import { WorldFormError } from '../lib/world-actions';

  let inviteInput: HTMLTextAreaElement | undefined;
  let busy = $state(false);
  let error = $state('');
  let hostCandidate = $state(false);

  async function join(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    if (typeof window.__swarmcraftAfterSvelteJoin !== 'function') {
      error = 'Desktop initialization is not complete. Retry when the application is ready.';
      return;
    }
    busy = true;
    error = '';
    let nativeSucceeded = false;
    try {
      // The Rust command verifies the entire signed invite and membership.
      // Neither the plaintext invite nor signing material is logged here.
      const response = await nativeWorldActions().join(inviteInput?.value ?? '', hostCandidate);
      nativeSucceeded = true;
      // Rust finished the native join command; clearing its token before a
      // UI refresh failure avoids unintended duplicate submissions. Only
      // canonical membership status can confirm whether joining is complete.
      if (inviteInput) inviteInput.value = '';
      hostCandidate = false;
      const match = String(response).match(/World ID:\s*(scworld:[^\s]+)/);
      // A valid Rust join can succeed without an ID in older CLI output:
      // still refresh the actual world list, never fabricate a membership.
      await window.__swarmcraftAfterSvelteJoin(match?.[1] ?? '');
    } catch (failure) {
      if (failure instanceof WorldFormError) {
        error = failure.message;
        inviteInput?.focus();
      } else {
        // Preserve the sensitive invite in the input for deliberate retry,
        // but never repeat it or native argument values in an error banner.
        error = nativeSucceeded
          ? 'The native join command completed, but the world list could not refresh. Refresh Worlds and confirm membership before submitting the invite again.'
          : 'Join could not be confirmed. Check the invite and networking, then refresh Worlds before retrying.';
      }
    } finally {
      busy = false;
    }
  }

</script>

<section id="joinPanel" class="view task-view" data-view-panel="join" aria-labelledby="joinTitle" hidden>
          <div class="task-shell">
            <div class="task-header">
              <button class="back-button" type="button" data-back-worlds aria-label="Back to worlds">←</button>
              <div>
                <h2 id="joinTitle">Join with an invite</h2>
                <p>Paste a complete signed invite. Joining stages a request; membership approval and checkpoint synchronization must finish before play. Discovery alone never grants membership.</p>
              </div>
            </div>

            <form id="joinForm" class="form-panel" novalidate data-journey-owner="svelte" aria-busy={busy} onsubmit={join}>
              <div class="field field-wide">
                <label for="joinInvite">Invite</label>
                <textarea id="joinInvite" bind:this={inviteInput} rows="7" placeholder="scinvite:…" required spellcheck="false"></textarea>
              </div>
              <label class="field field-wide" for="joinHostCandidate">
                <span><input id="joinHostCandidate" type="checkbox" bind:checked={hostCandidate} disabled={busy} /> Request eligibility to host this world in the future</span>
                <span class="muted">Accepted members can replicate and vote even if this is unchecked. Hosting additionally requires explicit local memory consent, EULA acceptance and verified authority.</span>
              </label>
              <div id="joinError" class="form-error field-wide" role="alert" hidden={!error}>{error}</div>
              <div class="form-actions field-wide">
                <button id="cancelJoin" class="button button-secondary" type="button" disabled={busy}>Cancel</button>
                <button id="joinWorld" class="button button-primary" type="submit" disabled={busy}>Join world</button>
              </div>
            </form>

            <p class="field-help">Have only a World ID? <button id="joinToDiscover" class="button button-subtle button-small" type="button">Check discovery</button></p>
          </div>
        </section>
