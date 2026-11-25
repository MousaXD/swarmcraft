<script lang="ts">
  import { onMount } from 'svelte';
  import { nativeBridge } from '../lib/native-bridge';
  import { createIdentityController, type IdentitySource } from '../lib/device-identity';
  import { createNativeConnectivityController } from '../lib/connectivity-state';

  const controller = createIdentityController(() => nativeBridge().invoke('node_identity', {}));
  const identityState = controller.state;
  const connectivityController = createNativeConnectivityController(
    () => nativeBridge().invoke('connectivity_diagnostics', {}),
  );
  const nativeConnectivity = connectivityController.state;

  let networkStatusUnverified = $state(false);

  async function refreshNativeConnectivity() {
    // A stopped or uncertain daemon can leave an old signed-off snapshot file.
    // Never re-label that persisted snapshot as current peer connectivity.
    if (networkStatusUnverified) {
      connectivityController.invalidate('Start networking to check current peer connectivity.');
      return;
    }
    await connectivityController.refresh();
  }

  let listenInput: HTMLInputElement | undefined;
  let serviceBusy = $state(false);
  let serviceMessage = $state('');
  let serviceError = $state('');

  async function boundedService<T>(operation: () => Promise<T>): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([
        Promise.resolve().then(operation),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error('Daemon IPC timed out')), 30_000);
        }),
      ]);
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }

  async function changeNetworking(action: 'start' | 'stop') {
    if (serviceBusy) return;
    if (typeof window.__swarmcraftAfterSvelteNetworkingChange !== 'function') {
      serviceError = 'Desktop initialization has not finished. Retry after setup.';
      return;
    }
    const listen = String(listenInput?.value ?? '').trim();
    if (action === 'start' && !listen) {
      serviceError = 'Enter a listen multiaddress before starting networking.';
      listenInput?.focus();
      return;
    }
    serviceBusy = true;
    serviceMessage = '';
    serviceError = '';
    let nativeAccepted = false;
    try {
      if (action === 'stop') {
        networkStatusUnverified = true;
        connectivityController.invalidate('Stopping networking. Earlier peer paths are now unverified.');
        await window.__swarmcraftAfterSvelteNetworkingChange('unknown');
      }
      if (action === 'start') {
        const pid = await boundedService(() => nativeBridge().invoke('ensure_daemon_running', { listen }));
        if (!Number.isInteger(pid) || pid <= 0) throw new Error('Daemon PID not verified');
        nativeAccepted = true;
        networkStatusUnverified = false;
        connectivityController.invalidate();
        serviceMessage = 'Networking started. Native connectivity may still be unavailable.';
        await boundedService(() => window.__swarmcraftAfterSvelteNetworkingChange!('started'));
        await connectivityController.refresh();
      } else {
        await boundedService(() => nativeBridge().invoke('stop_daemon', {}));
        nativeAccepted = true;
        connectivityController.invalidate(
          'Networking was stopped on this device. Previously reported paths may no longer be available.',
        );
        serviceMessage = 'Desktop networking stopped. Replication and migration supervision may be interrupted.';
        await window.__swarmcraftAfterSvelteNetworkingChange('stopped');
      }
    } catch {
      networkStatusUnverified = true;
      try { await window.__swarmcraftAfterSvelteNetworkingChange('unknown'); } catch { /* keep unknown */ }
      // A side effect may have completed even when its IPC acknowledgment,
      // follow-up legacy view refresh, or status check failed. Never assert
      // the daemon stopped or started on a timed-out or ambiguous operation.
      connectivityController.invalidate(
        'Networking outcome is not verified. Check native connectivity before relying on earlier paths.',
      );
      serviceError = nativeAccepted
        ? 'The networking command completed, but status refresh failed. Check native connectivity before repeating it.'
        : 'The networking change could not be confirmed. Check native diagnostics before retrying.';
    } finally {
      serviceBusy = false;
    }
  }

  async function startNetworking() { await changeNetworking('start'); }
  async function stopNetworking() { await changeNetworking('stop'); }

  // A named method avoids optional TypeScript arrow parameters in the
  // component's SSR transform while preserving the typed legacy bridge.
  function publishIdentity(value: string, source: IdentitySource = 'identity'): boolean {
    return controller.publishLegacy(value, source);
  }

  onMount(() => {
    const original = window.__swarmcraftSvelteIdentity;
    const oldConnectivity = window.__swarmcraftSvelteConnectivity;
    const bridge = { publish: publishIdentity };
    const connectivityBridge = { refresh: refreshNativeConnectivity };
    window.__swarmcraftSvelteIdentity = bridge;
    window.__swarmcraftSvelteConnectivity = connectivityBridge;
    return () => {
      if (window.__swarmcraftSvelteIdentity === bridge) {
        window.__swarmcraftSvelteIdentity = original;
      }
      if (window.__swarmcraftSvelteConnectivity === connectivityBridge) {
        window.__swarmcraftSvelteConnectivity = oldConnectivity;
      }
    };
  });

  async function refreshIdentity() {
    await controller.refresh();
  }
</script>

<section id="diagnosticsView" class="view diagnostics-view" data-view-panel="diagnostics" aria-labelledby="diagnosticsTitle" hidden>
          <div class="content-shell">
            <div class="task-header diagnostics-header">
              <button class="back-button" type="button" data-back-worlds aria-label="Back to worlds">←</button>
              <div>
                <h2 id="diagnosticsTitle">Diagnostics</h2>
                <p>Technical controls for connectivity, runtime setup, verification, and recovery.</p>
              </div>
            </div>

            <div class="compact-actions support-diagnostic-link">
              <button id="openSupportHistory" class="button button-secondary button-small" type="button">Support history and redacted report</button>
            </div>

            <section id="diagnosticWorldContext" class="diagnostic-world-context" aria-live="polite">
              <div class="context-copy">
                <span class="context-label">Selected world</span>
                <strong id="diagnosticWorldName">No world selected</strong>
                <span id="diagnosticWorldId" class="machine-line">World-specific controls are unavailable.</span>
              </div>
              <span id="diagnosticSafety" class="status-badge neutral" hidden>Unknown</span>
            </section>

            <div class="diagnostic-grid">
              <section class="diagnostic-section" id="connectivitySection">
                <div class="section-title-row">
                  <div><h3>Connectivity</h3><p>Player-facing path summary plus daemon controls.</p></div>
                  <span id="diagnosticConnectivity" class={`status-badge ${$nativeConnectivity.tone}`} data-native-connection-phase={$nativeConnectivity.phase} data-native-verified-count={$nativeConnectivity.verifiedReadCount} aria-busy={$nativeConnectivity.busy}>{$nativeConnectivity.label}</span>
                </div>
                <p id="diagnosticConnectivityDetail" class="helper" aria-live="polite">{$nativeConnectivity.detail}</p>
                <div class="compact-actions">
                  <button id="refreshNativeConnectivity" class="button button-subtle" type="button" onclick={refreshNativeConnectivity} aria-busy={$nativeConnectivity.busy} disabled={$nativeConnectivity.busy || networkStatusUnverified}>Check native connection</button>
                </div>
                <div class="field">
                  <label for="daemonListen">Listen multiaddress</label>
                  <input id="daemonListen" bind:this={listenInput} class="machine-input" value="/ip4/0.0.0.0/udp/0/quic-v1" />
                </div>
                <div class="compact-actions">
                  <button id="startDaemon" class="button button-secondary" type="button" onclick={startNetworking} disabled={serviceBusy} aria-busy={serviceBusy}>Start networking</button>
                  <button id="stopDaemon" class="button button-subtle" type="button" onclick={stopNetworking} disabled={serviceBusy} aria-busy={serviceBusy}>Stop networking</button>
                </div>
                <p id="networkServiceNotice" class="helper" role="status" aria-live="polite" hidden={!serviceMessage}>{serviceMessage}</p>
                <p id="networkServiceError" class="form-error" role="alert" hidden={!serviceError}>{serviceError}</p>
                <p class="helper">Relay transport can provide connectivity, but it does not make a peer the canonical host.</p>
                <h4>Selected world connection</h4>
                <pre id="routeDiagnosis" class="diagnostic-output" aria-live="polite">Current connection not yet checked.</pre>
                <div class="compact-actions">
                  <button id="retryRouteDiagnosis" class="button button-secondary world-required" type="button">Retry world connection</button>
                  <button id="copyRouteDiagnosis" class="button button-subtle world-required" type="button">Copy safe diagnosis</button>
                </div>
              </section>

              <section class="diagnostic-section" id="localNodeSection">
                <div class="section-title-row">
                  <div><h3>This device</h3><p>Local peer identity and initialization.</p></div>
                </div>
                <pre id="identity" class="diagnostic-output" aria-live="polite" aria-busy={$identityState.busy} data-identity-phase={$identityState.phase} data-native-verified-refresh-count={$identityState.nativeVerifiedRefreshCount}>{$identityState.text}</pre>
                <p class="form-error" role="alert" hidden={!$identityState.error}>{$identityState.error}</p>
                <div class="compact-actions">
                  <button id="init" class="button button-secondary" type="button">Set up device</button>
                  <button id="identityButton" class="button button-subtle" type="button" onclick={refreshIdentity} aria-busy={$identityState.busy} disabled={$identityState.busy}>Refresh identity</button>
                </div>
              </section>

              <section class="diagnostic-section" id="runtimeSection">
                <div class="section-title-row">
                  <div><h3>Minecraft runtime</h3><p>Paths required by the current direct-host Play implementation.</p></div>
                </div>
                <div id="runtimeNotice" class="inline-notice" role="status" aria-live="polite" hidden></div>
                <div class="field">
                  <label for="java">Java executable</label>
                  <input id="java" class="machine-input" value="java" />
                </div>
                <div class="field">
                  <label for="serverJar">Fabric server jar</label>
                  <input id="serverJar" class="machine-input" placeholder="/path/to/fabric-server.jar" />
                </div>
                <div class="field">
                  <label for="modJar">SwarmCraft Fabric mod jar</label>
                  <input id="modJar" class="machine-input" placeholder="/path/to/swarmcraft-fabric.jar" />
                </div>
                <label class="check-row" for="eula"><input id="eula" type="checkbox" /> <span>I accept the Minecraft server EULA.</span></label>
                <div class="compact-actions">
                  <button id="host" class="button button-secondary world-required authority-required" type="button">Start runtime</button>
                  <button id="stopHost" class="button button-subtle" type="button">Stop runtime</button>
                </div>
                <p id="runtimeEligibilityHint" class="helper">Select an authority-eligible world to start the runtime.</p>
              </section>

              <section class="diagnostic-section" id="replicationSection">
                <div class="section-title-row">
                  <div><h3>Replica storage</h3><p>Verification and background availability for the selected world.</p></div>
                </div>
                <p class="helper">A storage-only replica may remain useful even when this device is not eligible to host.</p>
                <div class="compact-actions">
                  <button id="diagnosticSeedOn" class="button button-secondary world-required" type="button">Keep replica</button>
                  <button id="diagnosticSeedOff" class="button button-subtle world-required" type="button">Stop seeding</button>
                  <button id="diagnosticVerify" class="button button-subtle world-required" type="button">Verify</button>
                </div>
              </section>

              <section class="diagnostic-section diagnostic-wide" id="recoverySection">
                <div class="section-title-row">
                  <div><h3>Export and recovery</h3><p>Inspect exact signed checkpoints and preserved branch evidence.</p></div>
                </div>
                <div class="compact-actions">
                  <button id="openRecoveryCopy" class="button button-primary world-required" type="button">Review checkpoints and conflicts</button>
                </div>
                <p class="helper">Export a selected checkpoint or create a new private world. The original and all unresolved conflicts stay intact.</p>
                <details>
                  <summary>Basic file export</summary>
                <div class="field-grid">
                  <div class="field field-wide">
                    <label for="destination">Destination</label>
                    <input id="destination" class="machine-input" placeholder="/path/to/world-export" />
                  </div>
                  <div class="field">
                    <label for="snapshotNumber">Snapshot number</label>
                    <input id="snapshotNumber" type="number" min="1" value="1" />
                  </div>
                </div>
                <div class="compact-actions">
                  <button id="exportWorld" class="button button-secondary world-required" type="button">Export latest</button>
                  <button id="recoverWorld" class="button button-warning world-required" type="button">Recover snapshot</button>
                </div>
                </details>
                <div id="recoveryCopyNotice" class="inline-notice" role="status" aria-live="polite" hidden></div>
              </section>
            </div>
          </div>
        </section>
