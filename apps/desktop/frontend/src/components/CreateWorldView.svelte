<script lang="ts">
  import { nativeWorldActions } from '../lib/native-world-actions';
  import { WorldFormError, type WorldVisibility } from '../lib/world-actions';

  let createName: HTMLInputElement | undefined;
  let createMinecraft: HTMLInputElement | undefined;
  let createLoader: HTMLInputElement | undefined;
  let createCompatibility: HTMLInputElement | undefined;
  let createVisibility: HTMLSelectElement | undefined;
  let form: HTMLFormElement | undefined;
  let busy = $state(false);
  let error = $state('');

  // The current catalog controller still owns validated version suggestions,
  // so read the actual inputs at submit instead of overriding them with a
  // second reactive version source.
  async function create(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    if (typeof window.__swarmcraftAfterSvelteCreate !== 'function') {
      error = 'Desktop initialization has not finished. Retry when the application is ready.';
      return;
    }
    busy = true;
    error = '';
    let nativeSucceeded = false;
    try {
      const id = await nativeWorldActions().create({
        name: createName?.value ?? '',
        minecraft: createMinecraft?.value ?? '',
        fabricLoader: createLoader?.value ?? '',
        compatibility: createCompatibility?.value ?? '',
        visibility: (createVisibility?.value ?? 'private') as WorldVisibility,
      });
      nativeSucceeded = true;
      // Rust created the real world. The established world-list controller
      // still owns its rich details, selection and navigation until migrated.
      form?.reset();
      await window.__swarmcraftAfterSvelteCreate(id);
      if (createMinecraft) createMinecraft.value = '26.1.2';
      if (createLoader) createLoader.value = '0.19.3';
      if (createCompatibility) createCompatibility.value = 'vanilla-fabric';
    } catch (failure) {
      if (failure instanceof WorldFormError) {
        error = failure.message;
        document.getElementById(failure.field)?.focus();
      } else {
        // Never reflect potentially sensitive native argv or invite data into
        // the renderer. Keep the form intact for explicit user retry.
        error = nativeSucceeded
          ? 'World created successfully, but the world list could not refresh. Refresh Worlds; do not create the world again.'
          : 'Creation could not be confirmed. Check the selected versions and local installation, then refresh Worlds before trying again.';
      }
    } finally {
      busy = false;
    }
  }
</script>

<section id="createPanel" class="view task-view" data-view-panel="create" aria-labelledby="createTitle" hidden>
          <div class="task-shell">
            <div class="task-header">
              <button class="back-button" type="button" data-back-worlds aria-label="Back to worlds">←</button>
              <div>
                <h2 id="createTitle">Create a world</h2>
                <p>Start a private replicated world. Advanced compatibility settings can stay at their defaults for a normal Fabric setup.</p>
              </div>
            </div>

            <form id="createForm" bind:this={form} class="form-panel" data-journey-owner="svelte" aria-busy={busy} novalidate onsubmit={create}>
              <div class="field field-wide">
                <label for="createName">World name</label>
                <input id="createName" bind:this={createName} autocomplete="off" placeholder="Friends SMP" required />
              </div>
              <div class="field field-wide">
                <label for="createVisibility">Who can find it?</label>
                <select id="createVisibility" bind:this={createVisibility}>
                  <option value="private">Invite only</option>
                  <option value="unlisted">Unlisted</option>
                  <option value="public">Public</option>
                </select>
              </div>

              <details class="form-details field-wide">
                <summary>Compatibility settings</summary>
                <div class="field-grid details-fields">
                  <div class="field">
                    <label for="createMinecraft">Minecraft version</label>
                    <input id="createMinecraft" bind:this={createMinecraft} value="26.1.2" required />
                  </div>
                  <div class="field">
                    <label for="createLoader">Fabric loader</label>
                    <input id="createLoader" bind:this={createLoader} value="0.19.3" required />
                  </div>
                  <div class="field field-wide">
                    <label for="createCompatibility">Compatibility profile</label>
                    <input id="createCompatibility" bind:this={createCompatibility} value="vanilla-fabric" required />
                  </div>
                </div>
                <p class="helper">These values become signed world metadata. They determine which devices are eligible to host, not which devices may store replicas.</p>
              </details>

              <div id="createError" class="form-error field-wide" role="alert" hidden={!error}>{error}</div>
              <div class="form-actions field-wide">
                <button id="cancelCreate" class="button button-secondary" type="button" disabled={busy}>Cancel</button>
                <button id="createWorld" class="button button-primary" type="submit" disabled={busy}>Create world</button>
              </div>
            </form>
          </div>
        </section>
