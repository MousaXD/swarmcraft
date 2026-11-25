<script lang="ts">
  import { onMount } from 'svelte';
  import { nativeBridge } from '../lib/native-bridge';
  import { createNativeWorldListController, worldSafetyTone, nativeWorldSafetySummary } from '../lib/world-list-state';
  import { createNativeHostReadinessController } from '../lib/host-readiness-state';
  import HostingContributionPanel from './HostingContributionPanel.svelte';

  const worldsController = createNativeWorldListController({
    listWorlds: () => nativeBridge().invoke('list_worlds', {}),
    worldStatus: world => nativeBridge().invoke('world_status', { world }),
    worldCompatibility: world => nativeBridge().invoke('world_compatibility', { world }),
  });
  const worldState = worldsController.state;
  const readinessController = createNativeHostReadinessController(
    world => nativeBridge().invoke('host_readiness', { world }),
  );
  const readinessState = readinessController.state;
  let selectedId = $state('');
  const selectedEntry = $derived($worldState.worlds.find(world => world.id === selectedId));
  let connectivityLabel = $state('Checking connectivity');
  let selectionError = $state('');

  function selectWorld(id: string) {
    if ($worldState.busy) return;
    if (typeof window.__swarmcraftSelectWorldFromSvelte !== 'function') {
      selectionError = 'World selection is not available. Restart the desktop app.';
      return;
    }
    selectionError = '';
    if (!window.__swarmcraftSelectWorldFromSvelte(id)) {
      selectionError = 'This world is no longer in the verified list. Refresh Worlds and try again.';
    }
  }

  function refreshWorlds() {
    if (typeof window.__swarmcraftRefreshWorldsFromSvelte !== 'function') {
      selectionError = 'World refresh is not initialized. Restart the desktop app.';
      return;
    }
    selectionError = '';
    void window.__swarmcraftRefreshWorldsFromSvelte();
  }

  onMount(() => {
    const old = window.__swarmcraftSvelteWorlds;
    const oldReadiness = window.__swarmcraftSvelteReadiness;
    const readinessBridge = {
      select: (world: string) => readinessController.select(world),
      clear: () => readinessController.clear(),
    };
    window.__swarmcraftSvelteReadiness = readinessBridge;
    const bridge = {
      refresh: () => worldsController.refresh(),
      setSelected: (id: string) => { selectedId = id; },
      setConnectivity: (label: string) => {
        connectivityLabel = label.trim() || 'Connectivity not verified';
      },
    };
    window.__swarmcraftSvelteWorlds = bridge;
    return () => {
      if (window.__swarmcraftSvelteWorlds === bridge) {
        window.__swarmcraftSvelteWorlds = old;
      }
      if (window.__swarmcraftSvelteReadiness === readinessBridge) {
        window.__swarmcraftSvelteReadiness = oldReadiness;
      }
    };
  });
</script>

<section id="worldsView" class="view is-active" data-view-panel="worlds" aria-labelledby="viewTitle">
          <div id="worldLoadError" class="page-notice notice-danger" role="alert" hidden={!$worldState.error}>{$worldState.error}</div>
          <p class="form-error" role="alert" hidden={!selectionError}>{selectionError}</p>
          <div class="worlds-layout">
            <section class="world-list-pane" aria-label="Your worlds">
              <div class="pane-head">
                <div>
                  <h2>Your worlds</h2>
                  <p id="worldCount" class="meta-text">{#if $worldState.busy}Loading…{:else if $worldState.phase === 'unavailable'}Could not read local worlds{:else if !$worldState.worlds.length}No local worlds{:else}{$worldState.worlds.length} {$worldState.worlds.length === 1 ? 'world' : 'worlds'}{/if}</p>
                </div>
                <button id="refresh" class="icon-button" type="button" aria-label="Refresh worlds" title="Refresh worlds" onclick={refreshWorlds} disabled={$worldState.busy} aria-busy={$worldState.busy}>↻</button>
              </div>
              <div id="worldCards" class="world-list" aria-live="polite" aria-busy={$worldState.busy} data-world-list-phase={$worldState.phase} data-native-verified-read-count={$worldState.verifiedReadCount}>
                {#if $worldState.busy && !$worldState.worlds.length}
                  <div class="loading-list" aria-hidden="true">
                    <span></span><span></span><span></span>
                  </div>
                {:else if !$worldState.busy && $worldState.phase === 'ready' && !$worldState.worlds.length}
                  <div class="empty-state">
                    <div class="empty-mark" aria-hidden="true">▦</div>
                    <h2>No worlds yet</h2>
                    <p>Create a world, import an existing Minecraft save, or join with a signed invite.</p>
                    <div class="compact-actions">
                      <button class="button button-primary" type="button" onclick={() => document.getElementById('quickCreate')?.click()}>Create world</button>
                      <button class="button button-secondary" type="button" onclick={() => document.getElementById('quickImport')?.click()}>Import existing world</button>
                      <button class="button button-secondary" type="button" onclick={() => document.getElementById('quickJoin')?.click()}>Join with invite</button>
                    </div>
                  </div>
                {:else}
                  {#each $worldState.worlds as world (world.id)}
                    <button type="button" class="world-row" class:is-selected={selectedId === world.id}
                      data-world-id={world.id} aria-pressed={selectedId === world.id}
                      disabled={$worldState.busy} onclick={() => selectWorld(world.id)}>
                      <span class="world-row-title">
                        <strong>{world.name}</strong>
                        <span class="world-row-id">{world.id}</span>
                      </span>
                      <span class={`status-badge ${worldSafetyTone(world.status.Safety)}`}>{world.status.Safety || 'Unknown'}</span>
                      <span class="world-row-meta">
                        <span>Minecraft {world.status.Minecraft || 'unknown'}</span>
                        <span>{world.status.Authority ? `Host ${world.status.Authority}` : 'Host not reported'}</span>
                        <span class="world-row-connectivity">{connectivityLabel}</span>
                      </span>
                    </button>
                  {/each}
                {/if}
              </div>
              <div class="list-footer">
                <button id="quickCreate" class="button button-secondary button-small" type="button">Create world</button>
                <button id="quickImport" class="button button-subtle button-small" type="button">Import existing world</button>
                <button id="quickJoin" class="button button-subtle button-small" type="button">Join with invite</button>
              </div>
            </section>

            <section id="selectedPanel" class="world-detail" aria-label="Selected world">
              <input id="world" type="hidden" />
              <div id="noSelection" class="selection-empty">
                <div class="empty-mark" aria-hidden="true">▦</div>
                <h2>Choose a world</h2>
                <p>Select a world to play, invite friends, keep a replica available, or inspect its safety.</p>
              </div>

              <div id="selectionContent" class="selection-content" hidden>
                <header class="world-hero">
                  <div class="world-heading">
                    <div class="title-line">
                      <h2 id="selectedName">{selectedEntry?.name || 'World'}</h2>
                      <span id="selectedSafety" class={`status-badge ${worldSafetyTone(selectedEntry?.status.Safety)}`}>{selectedEntry?.status.Safety || 'Unknown'}</span>
                    </div>
                    <button id="copyWorldId" class="machine-button" type="button" title="Copy World ID">
                      <span id="selectedWorldId" class="machine-line">{selectedEntry?.id || ''}</span>
                      <span aria-hidden="true">⧉</span>
                    </button>
                  </div>
                  <div class="hero-actions">
                    <button id="playWorld" class="button button-primary play-button" type="button" disabled>Play</button>
                    <button id="inviteWorld" class="button button-secondary" type="button">Invite</button>
                    <button id="transferHost" class="button button-secondary authority-transfer" type="button" disabled>Transfer host</button>
                  </div>
                </header>

                <p id="playAvailability" class="action-reason">Checking whether this device can host the world…</p>
                <p id="transferAvailability" class="action-reason" hidden></p>
                <div class="compact-actions">
                  <button id="manageInvites" class="button button-subtle button-small world-required" type="button">Manage invites</button>
                  <button id="refreshMembership" class="button button-subtle button-small world-required" type="button" hidden>Check membership</button>
                </div>
                <div id="membershipNotice" class="inline-notice" role="status" aria-live="polite" hidden></div>

                <!-- The native game-route controller owns this display and
                     both reconnect buttons. This Svelte candidate must never
                     infer a playable route from compatibility or migration. -->
                <div class="player-route-entry" aria-label="Minecraft multiplayer route">
                  <div class="player-route-copy">
                    <span class="section-label">Join current host</span>
                    <strong data-primary-route-state>Minecraft route not verified</strong>
                    <p data-primary-route-detail>A fresh authenticated route is required before reconnecting.</p>
                  </div>
                  <button class="button button-secondary" type="button" data-reconnect-world disabled>Reconnect instructions</button>
                </div>

                <div class="world-state-grid" aria-label="World summary">
                  <div class="state-cell">
                    <span class="state-label">Host</span>
                    <strong id="selectedAuthority">{selectedEntry?.status.Authority || 'Not reported'}</strong>
                  </div>
                  <div class="state-cell">
                    <span class="state-label">Connectivity</span>
                    <strong id="selectedConnectivity">Not reported</strong>
                    <span id="selectedConnectivityDetail" class="state-detail">No structured connectivity state yet.</span>
                  </div>
                  <div class="state-cell">
                    <span class="state-label">Members</span>
                    <strong id="selectedMembers">{selectedEntry?.status['Authorized peers'] || 'Unknown'}</strong>
                  </div>
                  <div class="state-cell">
                    <span class="state-label">Latest checkpoint</span>
                    <strong id="selectedCheckpoint">{selectedEntry?.status['Latest snapshot'] || (selectedEntry?.snapshots ? `${selectedEntry.snapshots} stored` : 'None')}</strong>
                  </div>
                </div>

                <section id="safetyPanel" class={`safety-panel ${worldSafetyTone(selectedEntry?.status.Safety)}`} aria-labelledby="safetyTitle">
                  <div>
                    <span id="safetyTitle" class="section-label">World safety</span>
                    <p id="selectedSummary">{nativeWorldSafetySummary(selectedEntry?.status.Safety)}</p>
                  </div>
                  <button id="worldConflicts" class="text-button world-required" type="button">Review conflicts</button>
                </section>

                <section id="hostReadinessPanel" class={`safety-panel ${$readinessState.tone}`} data-native-readiness-phase={$readinessState.phase} data-native-readiness-verified-count={$readinessState.verifiedReadCount} aria-busy={$readinessState.busy} aria-labelledby="hostReadinessQuestion">
                  <div>
                    <span id="hostReadinessQuestion" class="section-label">Can I turn off this PC?</span>
                    <h3 id="hostReadinessTitle">{$readinessState.title}</h3>
                    <p id="hostReadinessDetail" aria-live="polite">{$readinessState.detail}</p>
                  </div>
                </section>

                {#if selectedId}
                  {#key selectedId}
                    <HostingContributionPanel world={selectedId} />
                  {/key}
                {/if}

                <section class="player-section" aria-labelledby="localRolesTitle">
                  <div class="section-heading"><div>
                    <h3 id="localRolesTitle">Roles on this device</h3>
                    <p>Locally accepted V1 membership, not live authorization. Accepted members can play, replicate and vote without volunteering to host. Independent viewer and administrator grants are not supported in V1.</p>
                  </div></div>
                  <div class="world-state-grid">
                    <div class="state-cell">
                      <span class="section-label">Membership</span>
                      <strong data-local-role-field="Local member role">{selectedEntry?.status['Local member role'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Player</span>
                      <strong data-local-role-field="Local player role">{selectedEntry?.status['Local player role'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Replica</span>
                      <strong data-local-role-field="Local replica role">{selectedEntry?.status['Local replica role'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Voter</span>
                      <strong data-local-role-field="Local voter role">{selectedEntry?.status['Local voter role'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Host candidate</span>
                      <strong data-local-role-field="Local host candidate">{selectedEntry?.status['Local host candidate'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Administration</span>
                      <strong data-local-role-field="Local administration">{selectedEntry?.status['Local administration'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Membership transition</span>
                      <strong data-local-role-field="Local membership transition">{selectedEntry?.status['Local membership transition'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Hosting preference</span>
                      <strong data-local-role-field="Local hosting consent">{selectedEntry?.status['Local hosting consent'] || 'Not verified'}</strong>
                    </div>
                    <div class="state-cell">
                      <span class="section-label">Background seeding</span>
                      <strong data-local-role-field="Background seeding">{selectedEntry?.status['Background seeding'] || 'Not verified'}</strong>
                    </div>
                  </div>
                  <p class="field-help">Host eligibility, local memory consent and actual live authority are separate checks. Replica membership does not mean this device has a complete, verified checkpoint. Refresh Worlds for updated local records.</p>
                </section>

                <section class="player-section" aria-labelledby="availabilityTitle">
                  <div class="section-heading">
                    <div>
                      <h3 id="availabilityTitle">Keep the world available</h3>
                      <p>Store and serve verified world data in the background when this app is running.</p>
                    </div>
                  </div>
                  <div class="compact-actions">
                    <button id="seedOn" class="button button-secondary world-required" type="button">Keep a replica</button>
                    <button id="seedOff" class="button button-subtle world-required" type="button">Stop seeding</button>
                    <button id="verifyWorld" class="button button-subtle world-required" type="button">Verify replica</button>
                  </div>
                </section>

                <section id="migrationCard" class="player-section migration-card" aria-labelledby="migrationTitle">
                  <div class="section-heading migration-heading">
                    <div>
                      <h3 id="migrationTitle">Host handoff</h3>
                      <p id="migrationSummary">No host migration is active.</p>
                    </div>
                    <span id="migrationBadge" class="status-badge neutral">Not active</span>
                  </div>
                  <div id="migrationProgress" class="migration-progress" hidden>
                    <div class="progress-track" aria-hidden="true"><span id="migrationProgressBar"></span></div>
                    <ol id="migrationSteps" class="migration-steps" aria-label="Migration progress"></ol>
                  </div>
                  <div class="safety-panel neutral" aria-labelledby="clientContinuityTitle">
                    <div>
                      <span class="section-label">Minecraft client connection</span>
                      <h4 id="clientContinuityTitle">Minecraft connection not verified</h4>
                      <p id="clientContinuityDetail" role="status" aria-live="polite">SwarmCraft cannot yet verify a Minecraft client connection after host migration.</p>
                    </div>
                  </div>
                  <div class="compact-actions">
                    <button id="wakeWorld" class="button button-secondary migration-wake" type="button" disabled>Wake world</button>
                    <button id="sleepWorld" class="button button-subtle world-required" type="button">Stop world…</button>
                  </div>
                  <p id="migrationAvailability" class="section-note">Transfer and wake are unavailable in this build. Existing Play and graceful stop remain available.</p>
                </section>

                <section id="modsPanel" class="player-section" aria-labelledby="modsTitle">
                  <div class="section-heading">
                    <div>
                      <h3 id="modsTitle">Mods</h3>
                      <p>SwarmCraft verifies the exact server mods already required by this world's signed profile.</p>
                    </div>
                    <span id="modsBadge" class="status-badge neutral">Checking…</span>
                  </div>

                  <div class="world-state-grid" aria-label="Managed mod components">
                    <div class="state-cell">
                      <span class="state-label">Managed component</span>
                      <strong>Fabric API</strong>
                      <span id="fabricApiState" class="state-detail">Checking…</span>
                    </div>
                    <div class="state-cell">
                      <span class="state-label">Managed component</span>
                      <strong>SwarmCraft integration</strong>
                      <span id="swarmcraftModState" class="state-detail">Checking…</span>
                    </div>
                  </div>

                  <div class="section-heading">
                    <div>
                      <h4>Server mods</h4>
                      <p id="modsSummary">Checking the canonical server-mod profile…</p>
                    </div>
                  </div>
                  <div id="serverModsList" class="details-grid" aria-live="polite"></div>
                  <p id="modsIssues" class="section-note" hidden></p>

                  <div class="field field-wide">
                    <label for="modJarPath">Required mod JAR on this computer</label>
                    <input id="modJarPath" autocomplete="off" placeholder="/path/to/Lithium.jar" />
                    <p class="field-help">This supplies a locally missing canonical artifact. It does not change the world's signed modpack.</p>
                  </div>
                  <div class="compact-actions">
                    <button id="supplyRequiredMod" class="button button-secondary world-required" type="button">Supply required JAR</button>
                    <button id="refreshMods" class="button button-subtle world-required" type="button">Refresh / Verify</button>
                    <button id="openModsFolder" class="button button-subtle world-required" type="button">Open mods folder</button>
                  </div>
                </section>

                <details class="details-panel">
                  <summary>World details</summary>
                  <div class="details-grid">
                    <div class="detail-row"><span>Minecraft</span><strong id="selectedMinecraft">{selectedEntry?.status.Minecraft || 'Unknown'}</strong></div>
                    <div class="detail-row"><span>Compatibility</span><strong id="selectedCompatibility">{selectedEntry?.compatibility['Authority eligibility'] || 'Not synchronized'}</strong></div>
                  </div>
                  <div class="compact-actions">
                    <button id="worldPeers" class="button button-subtle world-required" type="button">Members</button>
                    <button id="worldCompatibility" class="button button-subtle world-required" type="button">Compatibility</button>
                    <button id="worldStatus" class="button button-subtle world-required" type="button">Full status</button>
                    <button id="openWorldDiagnostics" class="button button-subtle" type="button">Diagnostics</button>
                  </div>
                </details>

                <div id="worldNotice" class="inline-notice" role="status" aria-live="polite" hidden></div>

                <details class="danger-zone">
                  <summary>Membership and recovery</summary>
                  <div class="danger-zone-body">
                    <p>Leaving removes this device from the world after the signed request is accepted. Recovery tools are in Diagnostics.</p>
                    <button id="leaveWorld" class="button button-danger world-required" type="button">Leave world…</button>
                  </div>
                </details>
              </div>
            </section>
          </div>
        </section>
