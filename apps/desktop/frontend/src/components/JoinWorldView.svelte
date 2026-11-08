<script lang="ts">
  import { nativeWorldActions } from '../lib/native-world-actions';
  import { WorldFormError } from '../lib/world-actions';

  let inviteInput: HTMLTextAreaElement | undefined;
  let busy = $state(false);
  let error = $state('');

  async function join(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    if (typeof window.__swarmcraftAfterSvelteJoin !== 'function') {
      error = 'Desktop initialization is not complete. Retry when the application is ready.';
      return;
    }
    busy = true;
    error = '';
    try {
      // The Rust command verifies the entire signed invite and membership.
      // Neither the plaintext invite nor signing material is logged here.
      const response = await nativeWorldActions().join(inviteInput?.value ?? '');
      const match = String(response).match(/World ID:\s*(scworld:[^\s]+)/);
      // A valid Rust join can succeed without an ID in older CLI output:
      // still refresh the actual world list, never fabricate a membership.
      await window.__swarmcraftAfterSvelteJoin(match?.[1] ?? '');
      if (inviteInput) inviteInput.value = '';
    } catch (failure) {
      if (failure instanceof WorldFormError) {
        error = failure.message;
        inviteInput?.focus();
      } else {
        // Preserve the sensitive invite in the input for deliberate retry,
        // but never repeat it or native argument values in an error banner.
        error = 'Could not complete the join. Check the invite and networking, then refresh Worlds before retrying in case the request succeeded.';
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
                <p>Paste the complete signed invite from a world member. Discovery alone never grants membership.</p>
              </div>
            </div>

            <form id="joinForm" class="form-panel" novalidate data-journey-owner="svelte" aria-busy={busy} onsubmit={join}>
              <div class="field field-wide">
                <label for="joinInvite">Invite</label>
                <textarea id="joinInvite" bind:this={inviteInput} rows="7" placeholder="scinvite:…" required spellcheck="false"></textarea>
              </div>
              <div id="joinError" class="form-error field-wide" role="alert" hidden={!error}>{error}</div>
              <div class="form-actions field-wide">
                <button id="cancelJoin" class="button button-secondary" type="button" disabled={busy}>Cancel</button>
                <button id="joinWorld" class="button button-primary" type="submit" disabled={busy}>Join world</button>
              </div>
            </form>

            <details class="task-secondary">
              <summary>Have only a World ID?</summary>
              <p>A World ID can check discovery for public or unlisted worlds, but you still need accepted membership before joining a private world.</p>
              <div class="inline-field">
                <label for="joinWorldId">World ID</label>
                <input id="joinWorldId" class="machine-input" placeholder="scworld:…" />
                <button id="joinWorldIdButton" class="button button-secondary" type="button">Check</button>
              </div>
              <div id="joinWorldIdNotice" class="inline-notice" role="status" hidden></div>
            </details>
          </div>
        </section>
