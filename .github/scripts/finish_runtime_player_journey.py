from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def read(path: str) -> str:
    return (ROOT / path).read_text()


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content)


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise RuntimeError(f"missing integration anchor: {label}")
    return text.replace(old, new, 1)


def fix_create_profile() -> None:
    path = "crates/swarm-cli/src/main.rs"
    text = read(path)
    old = """            let legacy_hash =
                Hash32::from_domain_bytes(b"swarmcraft/legacy-compatibility/v1\\0", compatibility.as_bytes());
            let mut required_server_mods = vec![ArtifactRequirementV1 {
                artifact_id: "swarmcraft.legacy-compatibility".into(),
                version: "1".into(),
                artifact_hash: legacy_hash,
                side: ArtifactSideV1::Server,
                provider_hint: None,
            }];
            required_server_mods.extend(server_mods::requirements_from_jars(&server_mod)?);
"""
    if old in text:
        text = text.replace(
            old,
            """            // The legacy compatibility text is retained only as a CLI compatibility input.
            // It is not a physical server-mod requirement and must never make a clean world
            // appear to be missing a fictional JAR.
            let _legacy_compatibility = compatibility;
            let required_server_mods = server_mods::requirements_from_jars(&server_mod)?;
""",
            1,
        )
        text = text.replace(
            "    ArtifactRequirementV1, ArtifactSideV1, AuthorityPolicyV1, EpochMode, Hash32, InviteV1, JoinRequestV1,\n",
            "    AuthorityPolicyV1, EpochMode, InviteV1, JoinRequestV1,\n",
            1,
        )
    write(path, text)


def wire_tauri_mods() -> None:
    path = "apps/desktop/src-tauri/src/main.rs"
    text = read(path)
    if "async fn world_mods_status(" not in text:
        anchor = "#[tauri::command]\nasync fn world_compatibility"
        block = """#[tauri::command]
async fn world_mods_status(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "mods-status".into(), world, "--json".into()]).await
}

#[tauri::command(rename_all = "camelCase")]
async fn world_mods_add(app: AppHandle, world: String, jar_path: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let jar_path = require_value(jar_path, "Required mod JAR path")?;
    run_cli(&app, vec!["world".into(), "mods-add".into(), world, jar_path]).await
}

#[tauri::command(rename_all = "camelCase")]
async fn world_mods_remove(app: AppHandle, world: String, mod_id: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let mod_id = require_value(mod_id, "Mod ID")?;
    run_cli(&app, vec!["world".into(), "mods-remove".into(), world, mod_id]).await
}

#[tauri::command]
async fn open_world_mods_folder(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let raw = run_cli(&app, vec!["world".into(), "mods-path".into(), world]).await?;
    let path = require_value(raw, "Server mods folder")?;

    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");

    command.arg(&path).spawn().map_err(|error| format!("Could not open server mods folder: {error}"))?;
    Ok(path)
}

"""
        text = replace_once(text, anchor, block + anchor, "Tauri server-mod bridge")
    if "            world_mods_status,\n" not in text:
        text = replace_once(
            text,
            "            host_readiness,\n",
            "            host_readiness,\n            world_mods_status,\n            world_mods_add,\n            world_mods_remove,\n            open_world_mods_folder,\n",
            "Tauri handler server-mod bridge",
        )
    write(path, text)


def wire_adapter() -> None:
    path = "apps/desktop/src/backend-adapter.js"
    text = read(path)
    if "mods: Object.freeze({" not in text:
        anchor = "    runtime: Object.freeze({\n"
        block = """    mods: Object.freeze({
      status: async (world) => parseJsonContract(
        await call('world_mods_status', { world }),
        'Server mod status',
      ),
      supplyRequiredJar: (world, jarPath) => call('world_mods_add', { world, jarPath }),
      removeLocal: (world, modId) => call('world_mods_remove', { world, modId }),
      openFolder: (world) => call('open_world_mods_folder', { world }),
    }),

"""
        text = replace_once(text, anchor, block + anchor, "Desktop server-mod adapter")
    write(path, text)


def wire_html() -> None:
    path = "apps/desktop/src/index.html"
    text = read(path)
    if 'id="hostReadinessPanel"' not in text:
        anchor = """                <section id="safetyPanel" class="safety-panel neutral" aria-labelledby="safetyTitle">
                  <div>
                    <span id="safetyTitle" class="section-label">World safety</span>
                    <p id="selectedSummary">Safety state is unavailable.</p>
                  </div>
                  <button id="worldConflicts" class="text-button world-required" type="button">Review conflicts</button>
                </section>
"""
        block = anchor + """
                <section id="hostReadinessPanel" class="safety-panel neutral" aria-labelledby="hostReadinessQuestion">
                  <div>
                    <span id="hostReadinessQuestion" class="section-label">Can I turn off this PC?</span>
                    <h3 id="hostReadinessTitle">Checking shutdown safety…</h3>
                    <p id="hostReadinessDetail">SwarmCraft is checking whether another device can safely keep this world available.</p>
                  </div>
                </section>
"""
        text = replace_once(text, anchor, block, "Host Readiness player card")
    if 'id="modsPanel"' not in text:
        anchor = """                <details class="details-panel">
                  <summary>World details</summary>
"""
        block = """                <section id="modsPanel" class="player-section" aria-labelledby="modsTitle">
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

"""
        text = replace_once(text, anchor, block + anchor, "Mods player panel")
    write(path, text)


def wire_app() -> None:
    path = "apps/desktop/src/app.js"
    text = read(path)
    if "let hostReadinessRequestGeneration = 0;" not in text:
        text = replace_once(
            text,
            "let migrationRequestGeneration = 0;\n",
            "let migrationRequestGeneration = 0;\nlet hostReadinessRequestGeneration = 0;\nlet modsRequestGeneration = 0;\n",
            "player journey request generations",
        )

    if "function renderHostReadiness(" not in text:
        anchor = "function refreshVisibleMigration() {\n"
        idx = text.index(anchor)
        end = text.index("\n}\n\nfunction selectWorld", idx) + 3
        existing = text[idx:end]
        block = existing + """

function renderHostReadiness(readiness) {
  const panel = $('hostReadinessPanel');
  if (!panel) return;
  const state = readiness?.state || 'unknown';
  const copy = {
    safe: ['Safe to shut down this PC', 'Another ready device can take over this world.'],
    sleeping: ['Safe to shut down this PC', 'This world is durably stopped and its latest checkpoint is saved.'],
    blocked_by_runtime: ['Keep this PC on', 'Another device has a current copy, but its Minecraft runtime is not ready.'],
    blocked_by_mods: ['Keep this PC on', 'Another device is missing or has incompatible required server mods.'],
    syncing: ['Wait before shutting down', 'Another device is still syncing the latest world state.'],
    world_will_stop: ['World will go offline', 'No other ready host is currently reachable.'],
    blocked_by_quorum: ['World will go offline', 'Another device has a copy, but the remaining members cannot safely complete host takeover.'],
    conflict: ['Host handoff unavailable', 'This world has conflicting history that needs attention.'],
    degraded_safety: ['Keep this PC on', 'SwarmCraft cannot currently prove a safe host takeover.'],
    not_current_host: ['Shutdown safety not proven', 'This device is not the current host; SwarmCraft is not claiming takeover safety from this report.'],
    unknown: ['Checking shutdown safety…', readiness?.detail || 'SwarmCraft has not produced a fresh host-readiness report yet.'],
  };
  const [title, detail] = copy[state] || copy.unknown;
  const tone = state === 'safe' || state === 'sleeping'
    ? 'safe'
    : state === 'conflict'
      ? 'danger'
      : state === 'unknown'
        ? 'neutral'
        : 'warning';
  panel.className = `safety-panel ${tone}`;
  $('hostReadinessTitle').textContent = title;
  $('hostReadinessDetail').textContent = detail;
}

async function refreshHostReadiness(world) {
  if (!world) {
    renderHostReadiness(null);
    return;
  }
  const requestedWorldId = world.id;
  const requestGeneration = ++hostReadinessRequestGeneration;
  try {
    const readiness = await backend.hostReadiness(requestedWorldId);
    if (selectedWorldId === requestedWorldId && requestGeneration === hostReadinessRequestGeneration) {
      renderHostReadiness(readiness);
    }
  } catch (error) {
    if (selectedWorldId === requestedWorldId && requestGeneration === hostReadinessRequestGeneration) {
      renderHostReadiness({ state: 'unknown', detail: `Shutdown safety is unavailable: ${String(error)}` });
    }
  }
}

function refreshVisibleHostReadiness() {
  if (document.hidden) return;
  const world = selectedWorld();
  if (world) refreshHostReadiness(world);
}

function modComponent(runtimeStatus, id) {
  return runtimeStatus?.components?.find((component) => component.id === id) || null;
}

function renderWorldMods(runtimeStatus, modsStatus) {
  const fabricApi = modComponent(runtimeStatus, 'fabric_api');
  const swarmcraft = modComponent(runtimeStatus, 'swarmcraft_integration');
  $('fabricApiState').textContent = fabricApi ? `${fabricApi.state}${fabricApi.version ? ` · ${fabricApi.version}` : ''}` : 'Not reported';
  $('swarmcraftModState').textContent = swarmcraft ? `${swarmcraft.state}${swarmcraft.version ? ` · ${swarmcraft.version}` : ''}` : 'Not reported';

  const list = $('serverModsList');
  list.replaceChildren();
  const required = Array.isArray(modsStatus?.required)
    ? modsStatus.required.filter((item) => item.component_kind !== 'managed_runtime')
    : [];
  const installed = Array.isArray(modsStatus?.installed)
    ? modsStatus.installed.filter((item) => item.component_kind !== 'managed_runtime')
    : [];
  const issues = Array.isArray(modsStatus?.issues) ? modsStatus.issues : [];

  if (!required.length) {
    const row = document.createElement('div');
    row.className = 'detail-row';
    row.textContent = 'No third-party server mods are required by this world.';
    list.append(row);
  } else {
    for (const requirement of required) {
      const row = document.createElement('div');
      row.className = 'detail-row';
      const label = document.createElement('span');
      label.textContent = `${requirement.mod_id} ${requirement.version}`;
      const problem = issues.find((issue) => issue.mod_id === requirement.mod_id);
      const status = document.createElement('strong');
      status.textContent = problem ? problem.message : 'Verified';
      row.append(label, status);
      list.append(row);
    }
  }

  for (const item of installed) {
    const row = document.createElement('div');
    row.className = 'detail-row';
    const label = document.createElement('span');
    label.textContent = `${item.mod_id} ${item.version} · local`;
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'text-button';
    remove.textContent = 'Remove local copy';
    remove.addEventListener('click', async () => {
      try {
        await run('Removing local server mod…', () => backend.mods.removeLocal(worldId(), item.mod_id), {
          successMessage: `Removed the local ${item.mod_id} artifact. The signed world modpack was not changed.`,
        });
        refreshWorldMods(selectedWorld());
      } catch (_) {}
    });
    row.append(label, remove);
    list.append(row);
  }

  const ready = modsStatus?.ready === true;
  $('modsBadge').textContent = ready ? 'Verified' : 'Needs attention';
  $('modsBadge').className = `status-badge ${ready ? 'safe' : 'warning'}`;
  $('modsSummary').textContent = ready
    ? 'All required third-party server mods on this computer match the signed world profile.'
    : 'One or more required server mods are missing, incompatible, duplicated, or corrupt.';
  $('modsIssues').hidden = !issues.length;
  $('modsIssues').textContent = issues.map((issue) => issue.message).join(' · ');
}

async function refreshWorldMods(world) {
  if (!world) return;
  const requestedWorldId = world.id;
  const requestGeneration = ++modsRequestGeneration;
  const [runtimeResult, modsResult] = await Promise.allSettled([
    backend.runtime.status(requestedWorldId),
    backend.mods.status(requestedWorldId),
  ]);
  if (selectedWorldId !== requestedWorldId || requestGeneration !== modsRequestGeneration) return;
  const runtimeStatus = runtimeResult.status === 'fulfilled' ? runtimeResult.value : null;
  const modsStatus = modsResult.status === 'fulfilled'
    ? modsResult.value
    : { ready: false, required: [], installed: [], issues: [{ message: String(modsResult.reason) }] };
  renderWorldMods(runtimeStatus, modsStatus);
}

async function supplyRequiredMod() {
  const jarPath = $('modJarPath').value.trim();
  if (!jarPath) {
    showInline('worldNotice', 'Choose or enter the local path to the required Fabric mod JAR first.', 'warning');
    $('modJarPath').focus();
    return;
  }
  try {
    await run('Supplying required server mod…', () => backend.mods.supplyRequiredJar(worldId(), jarPath), {
      successMessage: 'Required server mod verified and copied into this world’s local runtime profile.',
    });
    $('modJarPath').value = '';
    await refreshWorldMods(selectedWorld());
    await refreshHostReadiness(selectedWorld());
  } catch (_) {}
}

async function openModsFolder() {
  try {
    await run('Opening mods folder…', () => backend.mods.openFolder(worldId()), { logResult: false });
  } catch (_) {}
}
"""
        text = text[:idx] + block + text[end:]

    if "hostReadinessRequestGeneration += 1;" not in text:
        text = replace_once(
            text,
            "  migrationRequestGeneration += 1;\n",
            "  migrationRequestGeneration += 1;\n  hostReadinessRequestGeneration += 1;\n  modsRequestGeneration += 1;\n",
            "clear selection readiness invalidation",
        )
    if "  renderHostReadiness(null);\n" not in text:
        text = replace_once(
            text,
            "  showInline('runtimeNotice', '');\n  updateWorldSpecificControls();\n",
            "  showInline('runtimeNotice', '');\n  renderHostReadiness(null);\n  updateWorldSpecificControls();\n",
            "clear selection host readiness",
        )
    if "  refreshHostReadiness(world);\n" not in text:
        text = replace_once(
            text,
            "  refreshMigrationState(world);\n",
            "  refreshMigrationState(world);\n  refreshHostReadiness(world);\n  refreshWorldMods(world);\n",
            "select world player safety refresh",
        )
    if "refreshVisibleHostReadiness();" not in text.split("async function startup()", 1)[1].split("}", 1)[0]:
        text = replace_once(
            text,
            "  refreshVisibleMigration();\n}\n\nasync function showIdentity",
            "  refreshVisibleMigration();\n  refreshVisibleHostReadiness();\n}\n\nasync function showIdentity",
            "startup host readiness refresh",
        )
    if "bindAction('supplyRequiredMod'" not in text:
        text = replace_once(
            text,
            "bindAction('diagnosticSeedOff', () => setSeeding(false));\n",
            "bindAction('diagnosticSeedOff', () => setSeeding(false));\nbindAction('supplyRequiredMod', supplyRequiredMod);\nbindAction('refreshMods', () => refreshWorldMods(selectedWorld()));\nbindAction('openModsFolder', openModsFolder);\n",
            "mods action bindings",
        )
    if "setInterval(refreshVisibleHostReadiness" not in text:
        text = replace_once(
            text,
            "setInterval(refreshVisibleMigration, MIGRATION_REFRESH_MS);\n",
            "setInterval(refreshVisibleMigration, MIGRATION_REFRESH_MS);\nsetInterval(refreshVisibleHostReadiness, MIGRATION_REFRESH_MS);\n",
            "host readiness polling",
        )
    write(path, text)


def add_contract_tests() -> None:
    path = "apps/desktop/tests/frontend-contract.test.mjs"
    text = read(path)
    if "authoritative host readiness is visible in the normal selected-world journey" not in text:
        text += """

test('authoritative host readiness is visible in the normal selected-world journey', async () => {
  const app = await text('app.js');
  const html = await text('index.html');
  assert.match(html, /id="hostReadinessPanel"/);
  assert.match(html, /Can I turn off this PC\?/);
  assert.match(app, /backend\.hostReadiness\(requestedWorldId\)/);
  assert.match(app, /Safe to shut down this PC/);
  assert.match(app, /Keep this PC on/);
  assert.match(app, /Wait before shutting down/);
  assert.match(app, /World will go offline/);
  assert.match(app, /Host handoff unavailable/);
});

test('normal Play is intercepted by the managed runtime wizard rather than requiring manual JAR hunting', async () => {
  const adapter = await text('backend-adapter.js');
  const wizard = await text('runtime-wizard.js');
  assert.match(adapter, /registerRuntimeWizard\(adapter\)/);
  assert.match(wizard, /event\.target\.closest\?\.\('#playWorld'\)/);
  assert.match(wizard, /backend\.runtime\.status\(world\.id\)/);
  assert.match(wizard, /backend\.runtime\.install\(world\.id/);
  assert.match(wizard, /backend\.runtime\.verify\(world\.id\)/);
  assert.match(wizard, /backend\.runtime\.launch\(world\.id\)/);
  assert.match(wizard, /runtimeEulaAccept/);
});

test('Mods panel supplies only canonical local artifacts and exposes exact backend verification', async () => {
  const html = await text('index.html');
  const app = await text('app.js');
  const adapter = await text('backend-adapter.js');
  const tauriMain = await desktopText('src-tauri/src/main.rs');
  assert.match(html, /id="modsPanel"/);
  assert.match(html, /Supply required JAR/);
  assert.match(html, /does not change the world's signed modpack/);
  assert.match(app, /backend\.mods\.status/);
  assert.match(app, /Remove local copy/);
  assert.match(adapter, /world_mods_status/);
  assert.match(adapter, /world_mods_add/);
  assert.match(adapter, /world_mods_remove/);
  assert.match(adapter, /open_world_mods_folder/);
  assert.match(tauriMain, /"mods-status"\.into\(\)/);
  assert.match(tauriMain, /"mods-add"\.into\(\)/);
  assert.match(tauriMain, /"mods-remove"\.into\(\)/);
});

test('Desktop Stop world waits for durable sleeping state instead of reporting raw process kill as success', async () => {
  const commands = await desktopText('src-tauri/src/runtime_commands.rs');
  const migration = await desktopText('../../crates/swarm-cli/src/migration.rs');
  assert.match(commands, /"migration-status"\.into\(\)/);
  assert.match(commands, /phase == "sleeping"/);
  assert.doesNotMatch(commands, /processes\.stop_host\(\)/);
  assert.match(migration, /session\.prepare_shutdown/);
  assert.match(migration, /RuntimeDisposition::Sleep/);
  assert.match(migration, /save_sleep_record/);
});
"""
    write(path, text)


def main() -> None:
    fix_create_profile()
    wire_tauri_mods()
    wire_adapter()
    wire_html()
    wire_app()
    add_contract_tests()


if __name__ == '__main__':
    main()
