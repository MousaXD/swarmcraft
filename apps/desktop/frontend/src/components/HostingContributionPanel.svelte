<script lang="ts">
  import { onMount } from 'svelte';
  import { readHostingContribution, saveHostingContribution } from '../lib/hosting-contribution';
  import type { HostingContributionReadiness } from '../lib/native-bridge';

  let selectedWorld = $state('');
  let readiness = $state<HostingContributionReadiness | null>(null);
  let checking = $state(false);
  let saving = $state(false);
  let explicitConsent = $state(false);
  let heapMib = $state(1024);
  let error = $state('');
  let notice = $state('');
  let generation = 0;

  const validHeap = $derived(Number.isSafeInteger(heapMib) && heapMib >= 512 && heapMib <= 65_536);

  async function refresh(world = selectedWorld) {
    const request = ++generation;
    if (!world) {
      readiness = null;
      checking = false;
      return;
    }
    checking = true;
    error = '';
    try {
      const response = await readHostingContribution(world);
      if (request !== generation || selectedWorld !== world) return;
      readiness = response;
      heapMib = response.selected_heap_mib || 1024;
    } catch {
      if (request !== generation || selectedWorld !== world) return;
      readiness = null;
      error = 'Cannot read this device’s hosting contribution. Review Diagnostics and retry.';
    } finally {
      if (request === generation) checking = false;
    }
  }

  function onSelectionChange(event: Event) {
    const detail = (event as CustomEvent<{ world?: string }>).detail;
    selectedWorld = detail?.world ?? '';
    generation += 1;
    readiness = null;
    checking = false;
    saving = false;
    explicitConsent = false;
    heapMib = 1024;
    error = '';
    notice = '';
    if (selectedWorld) void refresh(selectedWorld);
  }

  onMount(() => {
    window.addEventListener('swarmcraft:world-selected', onSelectionChange);
    const initial = (document.getElementById('world') as HTMLInputElement | null)?.value ?? '';
    if (initial) onSelectionChange(new CustomEvent('swarmcraft:world-selected', { detail: { world: initial } }));
    return () => {
      window.removeEventListener('swarmcraft:world-selected', onSelectionChange);
      generation += 1;
    };
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!selectedWorld || !readiness || checking || saving || !explicitConsent || !validHeap) return;
    const world = selectedWorld;
    const request = ++generation;
    saving = true;
    error = '';
    notice = '';
    try {
      const response = await saveHostingContribution(world, heapMib);
      if (request !== generation || selectedWorld !== world) return;
      readiness = response;
      explicitConsent = false;
      notice = 'Local preference saved. Only verified world policy, quorum and fencing can grant host authority.';
    } catch {
      if (request !== generation || selectedWorld !== world) return;
      error = 'Could not save hosting preference. No successful change is assumed; refresh status before retrying.';
    } finally {
      if (request === generation) saving = false;
    }
  }

  async function withdraw() {
    if (!selectedWorld || !readiness || checking || saving || readiness.selected_heap_mib === 0) return;
    const world = selectedWorld;
    const request = ++generation;
    saving = true;
    error = '';
    notice = '';
    try {
      const response = await saveHostingContribution(world, 0);
      if (request !== generation || selectedWorld !== world) return;
      readiness = response;
      explicitConsent = false;
      notice = 'Player-only preference saved. An active host must still finish safe checkpoint, fencing and handoff or shutdown.';
    } catch {
      if (request !== generation || selectedWorld !== world) return;
      error = 'Could not confirm withdrawal. Refresh status before retrying; do not assume a running host has stopped.';
    } finally {
      if (request === generation) saving = false;
    }
  }
</script>

<section id="hostingContributionPanel" class="player-section" aria-labelledby="hostingContributionTitle"
  data-consent-state={checking ? 'checking' : readiness ? (readiness.consented ? 'opted-in' : 'player-only') : error ? 'error' : 'unknown'}>
  <div class="section-heading">
    <div>
      <h3 id="hostingContributionTitle">Contribute hosting memory</h3>
      <p>Optional on this device. Player-only by default. Your choice does not grant world authority.</p>
    </div>
  </div>

  {#if !selectedWorld}
    <p class="section-note">Choose a world to inspect its local hosting preference.</p>
  {:else}
    <p id="hostingContributionStatus" class="section-note" role="status">
      {#if checking}
        Checking saved consent and local memory…
      {:else if readiness}
        {readiness.consented ? 'Hosting memory contribution selected' : 'Player only — 0 MiB'}
      {:else}
        Local hosting preference unavailable — no eligibility is assumed.
      {/if}
    </p>
    {#if readiness}
      <p id="hostingContributionReason" class="section-note">
        {readiness.reason} This is a local capacity check, not permission to take authority.
      </p>
      <p class="section-note">
        Java heap: {readiness.selected_heap_mib} MiB · Available memory:
        {readiness.available_mib === null ? 'unknown' : String(readiness.available_mib) + ' MiB'}.
        Whole-JVM RSS is not hard-capped.
      </p>
    {/if}

    <form id="hostingContributionForm" onsubmit={save}>
      <div class="field field-wide">
        <label for="hostingContributionHeap">Java server heap budget (MiB)</label>
        <input id="hostingContributionHeap" class="machine-input" type="number" min="512" max="65536" step="1"
          bind:value={heapMib} disabled={saving || checking || !readiness} />
        <p class="field-help">Minimum 512 MiB. OS, client, JVM-native and daemon headroom are checked by Rust.</p>
      </div>
      <label class="check-row field-wide" for="hostingContributionConfirm">
        <input id="hostingContributionConfirm" type="checkbox" bind:checked={explicitConsent}
          disabled={saving || checking || !readiness} />
        <span>I explicitly allow this device to offer the selected Java heap when eligible.</span>
      </label>
      <div class="compact-actions">
        <button id="hostingContributionOptIn" type="submit" class="button button-secondary"
          disabled={saving || checking || !readiness || !explicitConsent || !validHeap}>
          {saving ? 'Saving…' : 'Save explicit opt-in'}
        </button>
        <button id="hostingContributionWithdraw" type="button" class="button button-subtle"
          onclick={withdraw} disabled={saving || checking || !readiness || readiness.selected_heap_mib === 0}>
          Player only — withdraw
        </button>
        <button id="hostingContributionRefresh" type="button" class="button button-subtle"
          onclick={() => void refresh()} disabled={saving || checking}>Refresh status</button>
      </div>
    </form>
    {#if error}<p id="hostingContributionError" class="form-error" role="alert">{error}</p>{/if}
    {#if notice}<p id="hostingContributionNotice" class="section-note" role="status">{notice}</p>{/if}
  {/if}
</section>
