<script lang="ts">
  import { onMount } from 'svelte';
  import { resolveSignedDiscovery, formatDiscoveryResult, discoveryUnavailable, normalizeWorldId } from '../lib/discovery';

  let worldIdInput: HTMLInputElement | undefined;
  let worldIdNotice = $state('');
  let worldIdTone = $state('neutral');
  // Resolve the signed announcement through Rust. Discovery never joins.
  let resolvingWorldId = $state(false);
  let directoryDiagnosis = $state('');
  let discoveryRevision = 0;
  let discoveryExpiry: ReturnType<typeof setTimeout> | undefined;
  function clearDirectoryDiagnosis() {
    discoveryRevision += 1;
    clearTimeout(discoveryExpiry);
    directoryDiagnosis = '';
    worldIdNotice = '';
  }
  onMount(() => {
    const changed = () => {
      if (document.getElementById('discoverPanel')?.hidden) clearDirectoryDiagnosis();
    };
    document.addEventListener('swarmcraftcontextchanged', changed);
    return () => {
      document.removeEventListener('swarmcraftcontextchanged', changed);
      clearTimeout(discoveryExpiry);
    };
  });
  async function copyDirectoryDiagnosis() {
    if (!directoryDiagnosis) return;
    try { await navigator.clipboard.writeText(directoryDiagnosis); }
    catch { worldIdNotice = 'Could not copy directory diagnosis. Check clipboard access.'; worldIdTone = 'warning'; }
  }
  async function checkWorldId() {
    if (resolvingWorldId) return;
    const world = normalizeWorldId(worldIdInput?.value ?? '');
    if (!world) {
      worldIdNotice = 'Enter a full World ID (scworld: followed by 64 hexadecimal characters). Discovery does not grant membership.';
      worldIdTone = 'danger';
      worldIdInput?.focus();
      return;
    }
    clearDirectoryDiagnosis();
    const revision = discoveryRevision;
    resolvingWorldId = true;
    worldIdNotice = 'Checking the signed discovery announcement…';
    worldIdTone = 'neutral';
    let display;
    try {
      display = formatDiscoveryResult(await resolveSignedDiscovery(world), world);
    } catch {
      display = discoveryUnavailable();
    }
    resolvingWorldId = false;
    if (revision !== discoveryRevision) return;
    if (normalizeWorldId(worldIdInput?.value ?? '') !== world) { clearDirectoryDiagnosis(); return; }
    worldIdNotice = display.message;
    worldIdTone = display.tone;
    directoryDiagnosis = display.copy;
    const lifetime = Math.min(30_000, ('validUntil' in display && typeof display.validUntil === 'number' ? display.validUntil : Date.now() + 30_000) - Date.now());
    discoveryExpiry = setTimeout(() => {
      if (revision === discoveryRevision) {
        clearDirectoryDiagnosis();
        worldIdNotice = "Lookup observation expired. Check the listing again for current signed evidence.";
        worldIdTone = "warning";
      }
    }, Math.max(0, lifetime));
  }
</script>

<section id="discoverPanel" class="view task-view" data-view-panel="discover" aria-labelledby="discoverTitle" hidden>
          <div class="task-shell discovery-shell">
            <div class="task-header discovery-header">
              <div>
                <h2 id="discoverTitle">Discover worlds</h2>
                <p>Browse signed public listings or check an exact public or unlisted World ID. A directory listing is not membership, server availability, or a verified game route.</p>
              </div>
              <button id="discoverToJoin" class="button button-secondary" type="button">Join with invite</button>
            </div>
            <details class="task-secondary">
              <summary>Check an exact World ID</summary>
              <p>Public or unlisted worlds can be checked by exact ID. Private worlds are not listed. Ask the owner for a signed invite to request membership.</p>
              <div class="inline-field">
                <label for="joinWorldId">World ID</label>
                <input id="joinWorldId" bind:this={worldIdInput} oninput={clearDirectoryDiagnosis} class="machine-input" placeholder="scworld:…" autocomplete="off" spellcheck="false" maxlength="72" />
                <button id="joinWorldIdButton" class="button button-secondary" type="button" onclick={checkWorldId} disabled={resolvingWorldId} aria-busy={resolvingWorldId}>Check listing</button>
              </div>
              <div id="joinWorldIdNotice" class="inline-notice" role="status" data-tone={worldIdTone} hidden={!worldIdNotice}>{worldIdNotice}</div>
              <button id="copyDiscoveryDiagnosis" class="button button-subtle" type="button" onclick={copyDirectoryDiagnosis} disabled={!directoryDiagnosis}>Copy safe directory diagnosis</button>
            </details>
            <section id="publicWorldDiscovery" class="discovery-section" aria-labelledby="publicWorldTitle">
              <h3 id="publicWorldTitle">Public worlds</h3>
              <form id="publicWorldForm" class="discovery-search">
                <label for="publicWorldQuery">Search public worlds</label>
                <div class="discovery-search-controls">
                  <input id="publicWorldQuery" type="search" placeholder="Name, description or tag" maxlength="128" autocomplete="off" />
                  <button id="publicWorldSearch" type="submit" class="button button-primary">Search</button>
                </div>
              </form>
              <p id="publicWorldStatus" class="inline-notice" role="status" aria-live="polite" aria-atomic="true">Search by name or tag, or leave the field blank to browse.</p>
              <div id="publicWorldPages" class="discovery-pages" hidden>
                <button id="publicWorldPrev" class="button button-secondary button-small" type="button" disabled>Previous</button>
                <span id="publicWorldPage" role="status" aria-atomic="true"></span>
                <button id="publicWorldNext" class="button button-secondary button-small" type="button" disabled>Next</button>
              </div>
              <ul id="publicWorldResults" class="discovery-results" aria-label="Verified public listings" aria-busy="false"></ul>
            </section>
          </div>
        </section>
