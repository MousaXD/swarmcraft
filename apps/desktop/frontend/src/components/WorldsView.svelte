<script lang="ts">
  import HostingContributionPanel from './HostingContributionPanel.svelte';
</script>

<section id="worldsView" class="view is-active" data-view-panel="worlds" aria-labelledby="viewTitle">
          <div id="worldLoadError" class="page-notice notice-danger" role="alert" hidden></div>
          <div class="worlds-layout">
            <section class="world-list-pane" aria-label="Your worlds">
              <div class="pane-head">
                <div>
                  <h2>Your worlds</h2>
                  <p id="worldCount" class="meta-text">Loading…</p>
                </div>
                <button id="refresh" class="icon-button" type="button" aria-label="Refresh worlds" title="Refresh worlds">↻</button>
              </div>
              <div id="worldCards" class="world-list" aria-live="polite" aria-busy="true">
                <div class="loading-list" aria-hidden="true">
                  <span></span><span></span><span></span>
                </div>
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
                      <h2 id="selectedName">World</h2>
                      <span id="selectedSafety" class="status-badge neutral">Unknown</span>
                    </div>
                    <button id="copyWorldId" class="machine-button" type="button" title="Copy World ID">
                      <span id="selectedWorldId" class="machine-line"></span>
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

                <div class="world-state-grid" aria-label="World summary">
                  <div class="state-cell">
                    <span class="state-label">Host</span>
                    <strong id="selectedAuthority">Not reported</strong>
                  </div>
                  <div class="state-cell">
                    <span class="state-label">Connectivity</span>
                    <strong id="selectedConnectivity">Not reported</strong>
                    <span id="selectedConnectivityDetail" class="state-detail">No structured connectivity state yet.</span>
                  </div>
                  <div class="state-cell">
                    <span class="state-label">Members</span>
                    <strong id="selectedMembers">Unknown</strong>
                  </div>
                  <div class="state-cell">
                    <span class="state-label">Latest checkpoint</span>
                    <strong id="selectedCheckpoint">None</strong>
                  </div>
                </div>

                <section id="safetyPanel" class="safety-panel neutral" aria-labelledby="safetyTitle">
                  <div>
                    <span id="safetyTitle" class="section-label">World safety</span>
                    <p id="selectedSummary">Safety state is unavailable.</p>
                  </div>
                  <button id="worldConflicts" class="text-button world-required" type="button">Review conflicts</button>
                </section>

                <section id="hostReadinessPanel" class="safety-panel neutral" aria-labelledby="hostReadinessQuestion">
                  <div>
                    <span id="hostReadinessQuestion" class="section-label">Can I turn off this PC?</span>
                    <h3 id="hostReadinessTitle">Checking shutdown safety…</h3>
                    <p id="hostReadinessDetail">SwarmCraft is checking whether another device can safely keep this world available.</p>
                  </div>
                </section>

                <HostingContributionPanel />

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
                    <div class="detail-row"><span>Minecraft</span><strong id="selectedMinecraft">Unknown</strong></div>
                    <div class="detail-row"><span>Compatibility</span><strong id="selectedCompatibility">Not synchronized</strong></div>
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
