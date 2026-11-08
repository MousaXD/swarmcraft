<script lang="ts">
  import { onMount } from 'svelte';
  import { nativeBridge } from '../lib/native-bridge';
  import { createIdentityController, type IdentitySource } from '../lib/device-identity';

  const controller = createIdentityController(() => nativeBridge().invoke('node_identity', {}));
  const identityState = controller.state;

  onMount(() => {
    const original = window.__swarmcraftSvelteIdentity;
    const bridge = { publish: (value: string, source?: IdentitySource) => controller.publishLegacy(value, source) };
    window.__swarmcraftSvelteIdentity = bridge;
    return () => {
      if (window.__swarmcraftSvelteIdentity === bridge) {
        window.__swarmcraftSvelteIdentity = original;
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
                  <span id="diagnosticConnectivity" class="status-badge neutral">Not reported</span>
                </div>
                <p id="diagnosticConnectivityDetail" class="helper">The current backend does not expose structured connectivity for this world.</p>
                <div class="field">
                  <label for="daemonListen">Listen multiaddress</label>
                  <input id="daemonListen" class="machine-input" value="/ip4/0.0.0.0/udp/0/quic-v1" />
                </div>
                <div class="compact-actions">
                  <button id="startDaemon" class="button button-secondary" type="button">Start networking</button>
                  <button id="stopDaemon" class="button button-subtle" type="button">Stop networking</button>
                </div>
                <p class="helper">Relay transport can provide connectivity, but it does not make a peer the canonical host.</p>
              </section>

              <section class="diagnostic-section" id="localNodeSection">
                <div class="section-title-row">
                  <div><h3>This device</h3><p>Local peer identity and initialization.</p></div>
                </div>
                <pre id="identity" class="diagnostic-output" aria-live="polite">{$identityState.text}</pre>
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
                  <div><h3>Export and recovery</h3><p>Copy verified state or recover a specific snapshot into a destination.</p></div>
                </div>
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
              </section>
            </div>
          </div>
        </section>
