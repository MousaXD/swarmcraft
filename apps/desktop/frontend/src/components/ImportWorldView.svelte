<script lang="ts">
  import { nativeWorldActions } from '../lib/native-world-actions';
  import { WorldFormError, type WorldVisibility } from '../lib/world-actions';

  let form: HTMLFormElement | undefined;
  let source: HTMLInputElement | undefined;
  let name: HTMLInputElement | undefined;
  let visibility: HTMLSelectElement | undefined;
  let minecraft: HTMLInputElement | undefined;
  let fabricLoader: HTMLInputElement | undefined;
  let serverMods: HTMLTextAreaElement | undefined;
  let noServerMods: HTMLInputElement | undefined;
  let busy = $state(false);
  let error = $state('');

  async function importWorld(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    if (typeof window.__swarmcraftAfterSvelteImport !== 'function') {
      error = 'Desktop initialization has not finished. Retry when the application is ready.';
      return;
    }
    busy = true;
    error = '';
    try {
      // Source/JAR paths are validated by Rust before any canonical import;
      // no files, binaries or EULA state are imported by this renderer.
      const response = await nativeWorldActions().import({
        source: source?.value ?? '',
        name: name?.value ?? '',
        minecraft: minecraft?.value ?? '',
        fabricLoader: fabricLoader?.value ?? '',
        visibility: (visibility?.value ?? 'private') as WorldVisibility,
        serverMods: serverMods?.value ?? '',
        noServerMods: noServerMods?.checked === true,
      });
      await window.__swarmcraftAfterSvelteImport(response.worldId);
      form?.reset();
      if (minecraft) minecraft.value = '26.1.2';
      if (fabricLoader) fabricLoader.value = '0.19.3';
    } catch (failure) {
      if (failure instanceof WorldFormError) {
        error = failure.message;
        document.getElementById(failure.field)?.focus();
      } else {
        // Do not echo arbitrary absolute paths or sensitive backend messages.
        // The backend may have completed; avoid encouraging duplicate imports.
        error = 'Could not complete the import. Verify the source folder and required mods, and refresh Worlds before retrying.';
      }
    } finally {
      busy = false;
    }
  }
</script>

<section id="importPanel" class="view task-view" data-view-panel="import" aria-labelledby="importTitle" hidden>
          <div class="task-shell">
            <div class="task-header">
              <button class="back-button" type="button" data-back-worlds aria-label="Back to worlds">←</button>
              <div>
                <h2 id="importTitle">Import existing world</h2>
                <p>Bring an existing Minecraft save into SwarmCraft without changing the source folder, accepting the Minecraft EULA, or importing Java/runtime binaries.</p>
              </div>
            </div>

            <form id="importForm" bind:this={form} class="form-panel" data-journey-owner="svelte" aria-busy={busy} novalidate onsubmit={importWorld}>
              <div class="field field-wide">
                <label for="importSource">Minecraft world folder</label>
                <input id="importSource" bind:this={source} class="machine-input" autocomplete="off" placeholder="/path/to/.minecraft/saves/My World" required />
                <p class="field-help">Paste or type the existing world folder path from your file manager. The source folder stays unchanged.</p>
              </div>
              <div class="field field-wide">
                <label for="importName">World display name</label>
                <input id="importName" bind:this={name} autocomplete="off" placeholder="Friends SMP" required />
              </div>
              <div class="field field-wide">
                <label for="importVisibility">Who can find it?</label>
                <select id="importVisibility" bind:this={visibility}>
                  <option value="private">Invite only</option>
                  <option value="unlisted">Unlisted</option>
                  <option value="public">Public</option>
                </select>
              </div>
              <div class="field-grid field-wide">
                <div class="field">
                  <label for="importMinecraft">Exact Minecraft version</label>
                  <input id="importMinecraft" bind:this={minecraft} value="26.1.2" required />
                </div>
                <div class="field">
                  <label for="importLoader">Exact Fabric Loader version</label>
                  <input id="importLoader" bind:this={fabricLoader} value="0.19.3" required />
                </div>
              </div>
              <div class="field field-wide">
                <label for="importServerMods">Required third-party server mod JARs</label>
                <textarea id="importServerMods" bind:this={serverMods} class="machine-input" rows="4" placeholder="/path/to/Lithium.jar&#10;/path/to/FerriteCore.jar" spellcheck="false"></textarea>
                <p class="field-help">Enter one full local JAR path per line. The Rust backend verifies compatibility; SwarmCraft does not infer mod requirements from level.dat.</p>
              </div>
              <label class="check-row field-wide" for="importNoServerMods">
                <input id="importNoServerMods" bind:this={noServerMods} type="checkbox" />
                <span>I confirm this world requires no third-party server mods.</span>
              </label>
              <p class="helper field-wide">Choose either required server-mod JARs or the confirmation above. Import never accepts the Minecraft EULA and never imports Java/runtime binaries.</p>
              <div id="importError" class="form-error field-wide" role="alert" hidden={!error}>{error}</div>
              <div class="form-actions field-wide">
                <button id="cancelImport" class="button button-secondary" type="button" disabled={busy}>Cancel</button>
                <button id="importWorld" class="button button-primary" type="submit" disabled={busy}>Import world</button>
              </div>
            </form>
          </div>
        </section>
