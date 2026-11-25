<script lang="ts">
  import { onMount } from 'svelte';
  import { resourceStatus } from '../../../src/hosting-resources.js';
  import { readHostingContribution, saveHostingContribution } from '../lib/hosting-contribution';
  import type { HostingContributionReadiness } from '../lib/native-bridge';

  const { world: selectedWorld }: { world: string } = $props();
  let readiness = $state<HostingContributionReadiness | null>(null);
  let checking = $state(false);
  let saving = $state(false);
  let explicitConsent = $state(false);
  let heapMib = $state(4096);
  let cpuMillicores = $state(1000);
  let error = $state('');
  let notice = $state('');
  let generation = 0;
  let polling = false;

  const validHeap = $derived(Number.isSafeInteger(heapMib) && heapMib >= 512 && heapMib <= 65_536);
  const validCpu = $derived(Number.isSafeInteger(cpuMillicores) && cpuMillicores >= 250 && cpuMillicores <= 64_000);

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
      heapMib = response.selected_heap_mib || 4096;
      cpuMillicores = response.selected_cpu_millicores || 1000;
      explicitConsent = false;
    } catch {
      if (request !== generation || selectedWorld !== world) return;
      readiness = null;
      error = 'Cannot read this device’s hosting contribution. Review Diagnostics and retry.';
    } finally {
      if (request === generation) checking = false;
    }
  }

  onMount(() => {
    void refresh();
    const timer = window.setInterval(async () => {
      if (!readiness || saving || checking || polling ||
        !['stopping_at_checkpoint', 'checkpoint_blocked'].includes(readiness.checkpoint_state || '') &&
        !['linux_cgroup_active', 'linux_cgroup_reserved'].includes(readiness.process_cpu?.mode || '')) return;
      polling = true;
      const request = ++generation;
      const world = selectedWorld;
      try {
        const response = await readHostingContribution(world);
        if (request === generation && selectedWorld === world) {
          readiness = response;
          if (response.checkpoint_state === 'no_pending_checkpoint') {
            notice = 'No pending local checkpoint is observed. Review world safety before shutting down this PC.';
          }
        }
      } catch {
        if (request === generation && selectedWorld === world) {
          readiness = null;
          error = 'Checkpoint status cannot be verified. Keep the host running and inspect Diagnostics.';
        }
      } finally { polling = false; }
    }, 1500);
    return () => { generation += 1; window.clearInterval(timer); };
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!selectedWorld || !readiness || checking || saving || !explicitConsent || !validHeap || !validCpu) return;
    const world = selectedWorld;
    const request = ++generation;
    saving = true;
    error = '';
    notice = '';
    try {
      // The native write can revoke host consent and fence this instance.
      // Never leave an old green route/shutdown claim displayed while it runs.
      window.__swarmcraftInvalidateTransientSafety?.(world);
      const response = await saveHostingContribution(world, heapMib, cpuMillicores);
      if (request !== generation || selectedWorld !== world) return;
      readiness = response;
      explicitConsent = false;
      notice = 'Local preference saved. Only verified world policy, quorum and fencing can grant host authority.';
    } catch {
      if (request !== generation || selectedWorld !== world) return;
      readiness = null;
      explicitConsent = false;
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
      window.__swarmcraftInvalidateTransientSafety?.(world);
      const response = await saveHostingContribution(world, 0, 0);
      if (request !== generation || selectedWorld !== world) return;
      readiness = response;
      explicitConsent = false;
      notice = 'Player-only preference saved. An active host must still finish safe checkpoint, fencing and handoff or shutdown.';
    } catch {
      if (request !== generation || selectedWorld !== world) return;
      readiness = null;
      explicitConsent = false;
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
      <h3 id="hostingContributionTitle">Contribute device resources</h3>
      <p>Optional on this device. Player-only by default. Your choice does not grant world authority.</p>
    </div>
  </div>

  {#if !selectedWorld}
    <p class="section-note">Choose a world to inspect its local hosting preference.</p>
  {:else}
    <p id="hostingContributionStatus" class="section-note" role="status">
      {#if checking}
        Checking saved consent and device capacity…
      {:else if readiness?.checkpoint_state === 'stopping_at_checkpoint'}
        Stopping at checkpoint
      {:else if readiness?.checkpoint_state === 'checkpoint_blocked'}
        Checkpoint not confirmed — recovery required
      {:else if readiness}
        {readiness.consented ? 'Hosting resource preference saved' : 'Player only — 0 MiB / 0 CPU'}
      {:else}
        Local hosting preference unavailable — no eligibility is assumed.
      {/if}
    </p>
    {#if readiness}
      {#if readiness.checkpoint_state === 'checkpoint_blocked'}
        <p class="form-error" role="alert">Keep the controller running if Java is still active. Review Diagnostics, wait for the save barrier if it can finish, or preserve the runtime files for recovery. Do not force-stop or reset the world.</p>
        <p class="section-note">After Java has definitely exited and hosting is withdrawn, Linux/macOS operators can use the local CLI <code>world archive-unconfirmed-runtime</code> to preserve the save separately. Windows archival remains unsupported; keep the retained files and pending marker.</p>
      {/if}
      <p id="hostingContributionReason" class="section-note">
        {readiness.reason} This is a local capacity check, not permission to take authority.
      </p>
      <p class="section-note" id="hostingContributionCpuStatus">{resourceStatus(readiness).cpu}</p>
      <p class="section-note">{resourceStatus(readiness).memory}</p>
    {/if}

    <form id="hostingContributionForm" onsubmit={save}>
      <div class="field">
        <label for="hostingContributionHeap">Java server heap budget (MiB)</label>
        <input id="hostingContributionHeap" class="machine-input" type="number" min="512" max="65536" step="1"
          bind:value={heapMib} oninput={() => { explicitConsent = false; }} disabled={saving || checking || !readiness} />
        <p class="field-help">4096 MiB = 4 GiB. Rust checks native JVM, OS, client and daemon headroom.</p>
      </div>
      <div class="field">
        <label for="hostingContributionCpu">CPU limit (millicores)</label>
        <input id="hostingContributionCpu" class="machine-input" type="number" min="250" max="64000" step="1"
          bind:value={cpuMillicores} oninput={() => { explicitConsent = false; }} disabled={saving || checking || !readiness} />
        <p class="field-help">1000 = one logical CPU equivalent. An explicit cap requires delegated Linux CPU and memory controllers. No distributed hosting is enabled.</p>
      </div>
      <label class="check-row field-wide" for="hostingContributionConfirm">
        <input id="hostingContributionConfirm" type="checkbox" bind:checked={explicitConsent}
          disabled={saving || checking || !readiness} />
        <span>I explicitly allow this device to offer the selected Java heap and CPU limit when eligible.</span>
      </label>
      <div class="compact-actions field-wide">
        <button id="hostingContributionOptIn" type="submit" class="button button-secondary"
          disabled={saving || checking || !readiness || !explicitConsent || !validHeap || !validCpu}>
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
