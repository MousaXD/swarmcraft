const invoke = () => window.__TAURI__?.core?.invoke;
const byId = (id) => document.getElementById(id);
const clean = (value) => String(value ?? '').trim();

export function errorText(error) {
  if (!error) return 'The operation failed without a diagnostic.';
  if (typeof error === 'string') return error;
  if (error.message) return String(error.message);
  if (error.error?.message) return String(error.error.message);
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

function requireOkEnvelope(value, provider) {
  if (value?.status === 'ok' || value?.status === 'downloaded') return value.data ?? value;
  if (value?.status === 'manual_artifact_required') {
    throw new Error(
      value.data?.reason || `${provider} requires the exact artifact to be supplied manually because automatic download is not permitted.`,
    );
  }
  if (value?.error) throw new Error(value.error.message || `${provider} request failed.`);
  return value;
}

function providerHashes(hashes) {
  if (!hashes) return [];
  if (Array.isArray(hashes)) {
    return hashes
      .map((hash) => ({ algorithm: clean(hash.algorithm), digest: clean(hash.digest ?? hash.value) }))
      .filter((hash) => ['sha512', 'sha256', 'sha1', 'md5'].includes(hash.algorithm.toLowerCase()) && hash.digest);
  }
  return Object.entries(hashes)
    .filter(([, digest]) => clean(digest))
    .map(([algorithm, digest]) => ({ algorithm, digest: clean(digest) }))
    .filter((hash) => ['sha512', 'sha256', 'sha1', 'md5'].includes(hash.algorithm.toLowerCase()));
}

function dependencyTargets(dependencies, selectedByProject) {
  const result = [];
  for (const dependency of dependencies || []) {
    const projectId = clean(dependency.project_id ?? dependency.projectId);
    if (!projectId) continue;
    const kindRaw = clean(dependency.kind).toLowerCase();
    const kind =
      kindRaw === 'required'
        ? 'required'
        : kindRaw === 'optional'
          ? 'optional'
          : kindRaw === 'incompatible'
            ? 'incompatible'
            : ['embedded', 'embedded_library', 'include'].includes(kindRaw)
              ? 'embedded'
              : null;
    if (!kind) continue;
    const versionId = clean(dependency.version_id ?? dependency.versionId) || selectedByProject.get(projectId);
    if (!versionId) {
      if (kind === 'required') {
        throw new Error(`Required dependency ${projectId} did not resolve to an exact artifact.`);
      }
      continue;
    }
    result.push({ kind, projectId, versionId });
  }
  return result;
}

export function canonicalPackageFromDownloaded({ provider, version, file, downloaded, inspection, selectedByProject }) {
  const projectId = clean(version.project_id ?? version.projectId ?? file.project_id ?? file.projectId);
  const versionId = clean(
    version.version_id ?? version.versionId ?? file.version_id ?? file.versionId ?? file.file_id ?? file.fileId,
  );
  const artifactPath = clean(downloaded.path ?? downloaded.destination);
  if (!projectId || !versionId || !artifactPath) {
    throw new Error(`${provider} did not return an exact downloaded artifact identity.`);
  }
  const environment = clean(inspection.environment).toLowerCase();
  if (environment === 'client') {
    throw new Error(`${inspection.mod_id || inspection.modId} is client-only and cannot be required by a server world.`);
  }
  const side = environment === 'universal' ? 'both' : 'server';
  return {
    artifactId: clean(inspection.mod_id ?? inspection.modId),
    version: clean(inspection.version),
    side,
    artifactPath,
    provider,
    projectId,
    versionId,
    fileName: clean(downloaded.filename ?? file.filename ?? file.file_name ?? file.fileName),
    fileSize: Number(downloaded.size ?? downloaded.bytes ?? file.file_size ?? file.fileSize) || undefined,
    providerHashes: providerHashes(downloaded.hashes ?? file.hashes),
    retrieval: 'provider_download',
    dependencies: dependencyTargets(version.dependencies ?? file.dependencies, selectedByProject),
  };
}

function setNotice(target, message, kind = 'info') {
  if (!target) return;
  target.replaceChildren();
  target.textContent = message;
  target.hidden = !message;
  target.className = kind === 'error' ? 'form-error field-wide' : 'inline-notice field-wide';
  target.dataset.tone = kind === 'success' ? 'safe' : kind === 'warning' ? 'warning' : kind === 'error' ? 'danger' : 'neutral';
  target.dataset.kind = kind;
  delete target.dataset.createdWorldId;
}

function setCreateMessage(message, kind = 'info') {
  setNotice(byId('createError'), message, kind);
}

function makeButton(label, onClick, secondary = false) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = label;
  button.className = `button ${secondary ? 'button-subtle' : 'button-secondary'} button-small`;
  button.addEventListener('click', onClick);
  return button;
}

function makeResultRow(textValue, action) {
  const row = document.createElement('div');
  row.className = 'detail-row launcher-result-row';
  const text = document.createElement('strong');
  text.textContent = textValue;
  row.append(text, action);
  return row;
}

function installModsUi() {
  const form = byId('createForm');
  const submit = form?.querySelector('button[type="submit"]');
  const actions = submit?.closest('.form-actions');
  if (!form || !submit || !actions || byId('launcherMods')) return;
  const section = document.createElement('section');
  section.id = 'launcherMods';
  section.className = 'player-section field-wide launcher-section';
  section.innerHTML = `
    <div class="section-heading">
      <div>
        <h3>Mods</h3>
        <p>Search official provider catalogs. SwarmCraft resolves exact compatible files and required dependencies before creating the world.</p>
      </div>
    </div>
    <div class="field-grid launcher-fields">
      <div class="field">
        <label for="modProvider">Provider</label>
        <select id="modProvider">
          <option value="modrinth">Modrinth</option>
          <option value="curseforge">CurseForge</option>
        </select>
      </div>
      <div class="field">
        <label for="modSearch">Search mods</label>
        <input id="modSearch" type="search" placeholder="e.g. Lithium" autocomplete="off" />
      </div>
    </div>
    <div class="compact-actions"><button id="modSearchButton" type="button" class="button button-secondary button-small">Search</button></div>
    <p id="modSearchStatus" class="field-help" role="status" aria-live="polite"></p>
    <div id="modSearchResults" class="details-grid launcher-results"></div>
    <div class="section-heading launcher-subheading"><div><h3>Selected mods</h3></div></div>
    <div id="selectedMods" class="details-grid launcher-results"><p class="field-help">No third-party mods selected.</p></div>`;
  actions.before(section);
}

function installDiscoveryUi() {
  const form = byId('joinForm');
  if (!form || byId('publicWorldDiscovery')) return;
  const section = document.createElement('section');
  section.id = 'publicWorldDiscovery';
  section.className = 'player-section launcher-section';
  section.innerHTML = `
    <div class="section-heading">
      <div>
        <h3>Public worlds</h3>
        <p>Browse authenticated public announcements. Discovery never grants membership; invite-only worlds still require a signed invite.</p>
      </div>
    </div>
    <div class="field field-wide launcher-fields">
      <label for="publicWorldQuery">Search public worlds</label>
      <input id="publicWorldQuery" type="search" placeholder="World name or tag" />
    </div>
    <div class="compact-actions"><button id="publicWorldSearch" type="button" class="button button-secondary button-small">Search</button></div>
    <div id="publicWorldStatus" class="inline-notice" role="status" aria-live="polite" hidden></div>
    <div id="publicWorldResults" class="details-grid launcher-results"></div>`;
  form.parentNode.insertBefore(section, form.nextSibling);
}

function hideInternalInputs() {
  const compatibility = byId('createCompatibility');
  if (compatibility) {
    compatibility.required = false;
    const label = compatibility.closest('label');
    if (label) label.hidden = true;
  }
  const bootstrap = byId('bootstrapAddrs');
  const details = bootstrap?.closest('details');
  if (details) details.hidden = true;
}

function importCatalogValues(response, field) {
  const values = response?.[field] || response?.versions || response?.items || response;
  if (!Array.isArray(values)) throw new Error('Official version catalog returned an invalid response.');
  return values.map((item) => clean(item?.id ?? item?.version ?? item)).filter(Boolean);
}

function buildImportSelect(source, values, preferred = '') {
  const select = document.createElement('select');
  select.id = source.id;
  select.name = source.name;
  select.required = source.required;
  select.className = source.className;
  for (const value of values) {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = value;
    select.append(option);
  }
  select.value = values.includes(preferred) ? preferred : values[0] || '';
  return select;
}

function ensureImportCatalogUi() {
  const form = byId('importForm');
  const minecraft = byId('importMinecraft');
  const loader = byId('importLoader');
  if (!form || !minecraft || !loader) return null;
  let status = byId('importCatalogStatus');
  let error = byId('importCatalogError');
  let retry = byId('importCatalogRetry');
  if (!status) {
    const fieldGrid = minecraft.closest('.field-grid');
    status = document.createElement('p');
    status.id = 'importCatalogStatus';
    status.className = 'field-help field-wide';
    status.setAttribute('role', 'status');
    status.setAttribute('aria-live', 'polite');
    error = document.createElement('div');
    error.id = 'importCatalogError';
    error.className = 'form-error field-wide';
    error.setAttribute('role', 'alert');
    error.hidden = true;
    const actions = document.createElement('div');
    actions.className = 'compact-actions field-wide';
    retry = document.createElement('button');
    retry.id = 'importCatalogRetry';
    retry.type = 'button';
    retry.className = 'button button-secondary button-small';
    retry.textContent = 'Retry version catalogs';
    retry.hidden = true;
    actions.append(retry);
    fieldGrid?.after(status, error, actions);
  }
  return { form, minecraft, loader, status, error, retry };
}

function setImportCatalogError(ui, message = '') {
  ui.error.textContent = message;
  ui.error.hidden = !message;
  ui.retry.hidden = !message;
}

async function loadImportFabric(call, minecraftSelect, loaderSelect, ui, { refresh = false, preferred = '' } = {}) {
  const minecraftVersion = clean(minecraftSelect.value);
  ui.status.textContent = `Fetching compatible Fabric Loader versions for Minecraft ${minecraftVersion}…`;
  setImportCatalogError(ui, '');
  const result = await call('fabric_loader_versions', { minecraftVersion, refresh });
  const values = importCatalogValues(result, 'loaders');
  if (!values.length) throw new Error(`Fabric Meta returned no compatible loaders for Minecraft ${minecraftVersion}.`);
  const replacement = buildImportSelect(loaderSelect, values, preferred);
  loaderSelect.replaceWith(replacement);
  ui.loader = replacement;
  ui.status.textContent = 'Import version catalogs are ready.';
  return replacement;
}

export async function hydrateImportCatalogs(call, { refresh = false } = {}) {
  const ui = ensureImportCatalogUi();
  if (!ui) return null;
  const preferredMinecraft = clean(ui.minecraft.value);
  const preferredLoader = clean(ui.loader.value);
  ui.status.textContent = 'Fetching official Minecraft and Fabric Loader versions for import…';
  setImportCatalogError(ui, '');
  ui.retry.onclick = () => hydrateImportCatalogs(call, { refresh: true });

  try {
    const catalog = await call('minecraft_versions', { includeSnapshots: false, refresh });
    const versions = importCatalogValues(catalog, 'versions');
    if (!versions.length) throw new Error('Mojang returned no supported Minecraft releases.');
    const minecraftSelect = buildImportSelect(ui.minecraft, versions, preferredMinecraft);
    const minecraftVersion = clean(minecraftSelect.value);
    const fabric = await call('fabric_loader_versions', { minecraftVersion, refresh });
    const loaders = importCatalogValues(fabric, 'loaders');
    if (!loaders.length) throw new Error(`Fabric Meta returned no compatible loaders for Minecraft ${minecraftVersion}.`);
    const loaderSelect = buildImportSelect(ui.loader, loaders, preferredLoader);

    ui.minecraft.replaceWith(minecraftSelect);
    ui.loader.replaceWith(loaderSelect);
    ui.minecraft = minecraftSelect;
    ui.loader = loaderSelect;
    ui.status.textContent = 'Import version catalogs are ready.';

    let lastMinecraft = minecraftSelect.value;
    minecraftSelect.addEventListener('change', async () => {
      const requestedMinecraft = minecraftSelect.value;
      const previousLoader = loaderSelect.value;
      loaderSelect.disabled = true;
      try {
        const replacement = await loadImportFabric(call, minecraftSelect, ui.loader, ui, {
          refresh: false,
          preferred: previousLoader,
        });
        replacement.disabled = false;
        lastMinecraft = requestedMinecraft;
      } catch (error) {
        minecraftSelect.value = lastMinecraft;
        ui.loader.disabled = false;
        ui.status.textContent = `Could not load Fabric Loader versions for Minecraft ${requestedMinecraft}. The previous compatible selection was kept.`;
        setImportCatalogError(ui, errorText(error));
      }
    });
    return ui;
  } catch (error) {
    ui.status.textContent = 'Official Import version catalogs are unavailable. Exact version fields remain usable; retry when the providers are reachable.';
    setImportCatalogError(ui, errorText(error));
    return ui;
  }
}

function renderCreateRepairState({ created, pendingPackages, error, retry }) {
  const target = byId('createError');
  if (!target) return;
  target.replaceChildren();
  target.hidden = false;
  target.className = 'inline-notice field-wide';
  target.dataset.tone = 'warning';
  target.dataset.kind = 'warning';
  target.dataset.createdWorldId = clean(created?.worldId);

  const message = document.createElement('p');
  message.textContent = `World ${clean(created?.worldId)} was created canonically, but local mod setup needs attention: ${errorText(error)}`;
  const detail = document.createElement('p');
  detail.className = 'field-help launcher-repair-detail';
  detail.textContent = `${pendingPackages.length} local mod artifact${pendingPackages.length === 1 ? '' : 's'} still need installation. Retrying continues setup for this existing world and will not create a second world.`;
  const actions = document.createElement('div');
  actions.className = 'compact-actions';
  const button = document.createElement('button');
  button.id = 'createRepairRetry';
  button.type = 'button';
  button.className = 'button button-secondary button-small';
  button.textContent = 'Retry local mod setup';
  button.addEventListener('click', retry);
  actions.append(button);
  target.append(message, detail, actions);
}

function setLocalStatus(target, message, tone = 'neutral') {
  if (!target) return;
  target.textContent = message;
  target.hidden = !message;
  target.dataset.tone = tone;
}

export function install() {
  const call = invoke();
  if (!call) return null;
  hideInternalInputs();
  installModsUi();
  installDiscoveryUi();
  hydrateImportCatalogs(call).catch((error) => {
    const ui = ensureImportCatalogUi();
    if (ui) {
      ui.status.textContent = 'Official Import version catalogs are unavailable. Exact version fields remain usable.';
      setImportCatalogError(ui, errorText(error));
    }
  });

  const roots = [];
  const renderSelected = () => {
    const target = byId('selectedMods');
    if (!target) return;
    target.replaceChildren();
    if (!roots.length) {
      const empty = document.createElement('p');
      empty.className = 'field-help';
      empty.textContent = 'No third-party mods selected.';
      target.append(empty);
      return;
    }
    for (const root of roots) {
      target.append(
        makeResultRow(
          `${root.title} · ${root.provider === 'modrinth' ? 'Modrinth' : 'CurseForge'}`,
          makeButton(
            'Remove',
            () => {
              roots.splice(roots.indexOf(root), 1);
              renderSelected();
            },
            true,
          ),
        ),
      );
    }
  };

  const addRoot = (root) => {
    if (!roots.some((item) => item.provider === root.provider && item.projectId === root.projectId)) roots.push(root);
    renderSelected();
  };

  byId('modSearchButton')?.addEventListener('click', async () => {
    const provider = byId('modProvider').value;
    const query = clean(byId('modSearch').value);
    const minecraft = clean(byId('createMinecraft').value);
    const loader = clean(byId('createLoader').value);
    const status = byId('modSearchStatus');
    const results = byId('modSearchResults');
    if (!minecraft || !loader) {
      status.textContent = 'Choose Minecraft and Fabric Loader first.';
      return;
    }
    status.textContent = `Searching ${provider === 'modrinth' ? 'Modrinth' : 'CurseForge'}…`;
    results.replaceChildren();
    try {
      let items;
      if (provider === 'modrinth') {
        const response = await call('modrinth_search', {
          query: {
            query,
            minecraft_version: minecraft,
            loader: 'fabric',
            environment: 'server',
            release_type: null,
            offset: 0,
            limit: 20,
          },
        });
        items = response.items || [];
      } else {
        const response = requireOkEnvelope(
          await call('curseforge_search', {
            query,
            minecraft,
            loader: 'fabric',
            environment: 'server',
            index: 0,
            pageSize: 20,
          }),
          'CurseForge',
        );
        items = response.projects || [];
      }
      status.textContent = items.length
        ? `${items.length} compatible result${items.length === 1 ? '' : 's'}.`
        : 'No compatible mods found.';
      for (const item of items) {
        const projectId = clean(item.project_id ?? item.projectId);
        const title = clean(item.title ?? item.name ?? item.slug ?? projectId);
        results.append(makeResultRow(title, makeButton('Add', () => addRoot({ provider, projectId, title }))));
      }
    } catch (error) {
      status.textContent = errorText(error);
    }
  });

  async function inspectPackage(path) {
    return call('inspect_mod_artifact', { path });
  }

  async function prepareModrinth(root, staging) {
    const graph = await call('modrinth_resolve_project', {
      projectId: root.projectId,
      minecraftVersion: clean(byId('createMinecraft').value),
      loader: 'fabric',
      environment: 'server',
    });
    const versions = graph.versions || [];
    const selectedByProject = new Map(versions.map((version) => [clean(version.project_id), clean(version.version_id)]));
    const packages = [];
    for (const version of versions) {
      const file =
        (version.files || []).find((candidate) => candidate.primary && candidate.retrieval?.state === 'provider_download') ||
        (version.files || []).find((candidate) => candidate.retrieval?.state === 'provider_download');
      if (!file) {
        throw new Error(`${version.display_name || version.project_id} cannot be downloaded automatically from Modrinth.`);
      }
      const destinationDir = `${staging}/modrinth/${version.project_id}/${version.version_id}`;
      const downloaded = await call('modrinth_download', {
        request: { locator: file.locator, destination_dir: destinationDir, max_bytes: null },
      });
      const inspection = await inspectPackage(downloaded.path);
      packages.push(canonicalPackageFromDownloaded({ provider: 'modrinth', version, file, downloaded, inspection, selectedByProject }));
    }
    return packages;
  }

  async function prepareCurseForge(root, staging) {
    const envelope = await call('curseforge_resolve_project', {
      projectId: Number(root.projectId),
      minecraft: clean(byId('createMinecraft').value),
      loader: 'fabric',
      environment: 'server',
    });
    const graph = requireOkEnvelope(envelope, 'CurseForge');
    const versions = graph.packages || [];
    const selectedByProject = new Map(
      versions.map((version) => [clean(version.project_id), clean(version.version_id ?? version.file_id)]),
    );
    const packages = [];
    for (const version of versions) {
      const fileName = clean(version.file_name);
      const destination = `${staging}/curseforge/${version.project_id}/${version.version_id}/${fileName}`;
      const downloadedEnvelope = await call('curseforge_download', {
        fileId: Number(version.file_id),
        destination,
      });
      const downloaded = requireOkEnvelope(downloadedEnvelope, 'CurseForge');
      const inspection = await inspectPackage(downloaded.destination);
      packages.push(
        canonicalPackageFromDownloaded({ provider: 'curseforge', version, file: version, downloaded, inspection, selectedByProject }),
      );
    }
    return packages;
  }

  async function finishLocalModSetup(created, pendingPackages, submit) {
    while (pendingPackages.length) {
      const pkg = pendingPackages[0];
      await call('world_mods_add', { world: created.worldId, jarPath: pkg.artifactPath });
      pendingPackages.shift();
    }
    setCreateMessage(`World created with canonical fingerprint ${created.canonical?.compatibilityFingerprint || ''}.`, 'success');
    if (submit) {
      submit.disabled = true;
      submit.textContent = 'World created';
    }
    window.setTimeout(() => window.location.reload(), 250);
  }

  byId('createForm')?.addEventListener(
    'submit',
    async (event) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      const name = clean(byId('createName').value);
      const minecraftVersion = clean(byId('createMinecraft').value);
      const loaderVersion = clean(byId('createLoader').value);
      const visibility = clean(byId('createVisibility').value);
      if (!name || !minecraftVersion || !loaderVersion || !visibility) {
        setCreateMessage('Choose a name, Minecraft version, Fabric Loader, and visibility.', 'error');
        return;
      }
      const submit = event.currentTarget.querySelector('button[type="submit"]');
      if (submit) submit.disabled = true;
      setCreateMessage('Resolving exact mods and dependencies…');
      try {
        const staging = await call('provider_staging_dir');
        const packages = [];
        for (const root of [...roots].sort((a, b) => `${a.provider}:${a.projectId}`.localeCompare(`${b.provider}:${b.projectId}`))) {
          packages.push(...(await (root.provider === 'modrinth' ? prepareModrinth(root, staging) : prepareCurseForge(root, staging))));
        }
        const unique = new Map(packages.map((item) => [`${item.provider}:${item.projectId}:${item.versionId}`, item]));
        setCreateMessage('Creating canonical world…');
        const created = await call('create_canonical_world', {
          request: {
            name,
            visibility,
            modpack: {
              minecraftVersion,
              loaderId: 'fabric',
              loaderVersion,
              packages: [...unique.values()],
              datapacks: [],
            },
          },
        });
        const pendingPackages = [...unique.values()];
        try {
          await finishLocalModSetup(created, pendingPackages, submit);
        } catch (setupError) {
          const retry = async (retryEvent) => {
            const button = retryEvent.currentTarget;
            button.disabled = true;
            button.textContent = 'Retrying local setup…';
            try {
              await finishLocalModSetup(created, pendingPackages, submit);
            } catch (retryError) {
              renderCreateRepairState({ created, pendingPackages, error: retryError, retry });
            }
          };
          renderCreateRepairState({ created, pendingPackages, error: setupError, retry });
        }
      } catch (error) {
        setCreateMessage(errorText(error), 'error');
        if (submit) submit.disabled = false;
      }
    },
    true,
  );

  byId('publicWorldSearch')?.addEventListener('click', async () => {
    const status = byId('publicWorldStatus');
    const results = byId('publicWorldResults');
    setLocalStatus(status, 'Searching authenticated public announcements…');
    results.replaceChildren();
    try {
      const report = await call('discovery_search', { query: clean(byId('publicWorldQuery').value) || null });
      setLocalStatus(
        status,
        report.detail ||
          (report.results?.length
            ? `${report.results.length} public world${report.results.length === 1 ? '' : 's'} found.`
            : 'No public worlds found.'),
      );
      for (const world of report.results || []) {
        const action = makeButton(
          world.join_action === 'invite_required' ? 'Invite required' : 'View',
          () => {
            byId('joinWorldId').value = world.world_id;
            setLocalStatus(
              status,
              world.join_action === 'invite_required'
                ? `${world.name} is public to discover, but its authority still requires a signed invite for membership.`
                : `${world.name} was discovered. Membership is still decided by the world authority.`,
            );
          },
          true,
        );
        results.append(
          makeResultRow(`${world.name} · Minecraft ${world.minecraft_version} · Fabric ${world.loader_version}`, action),
        );
      }
    } catch (error) {
      setLocalStatus(status, errorText(error), 'danger');
    }
  });

  byId('joinWorldIdButton')?.addEventListener(
    'click',
    async (event) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      const world = clean(byId('joinWorldId').value);
      if (!world) return;
      const status = byId('joinWorldIdNotice');
      setLocalStatus(status, 'Checking the signed discovery announcement…');
      try {
        const report = await call('discovery_resolve', { world });
        setLocalStatus(
          status,
          report.state === 'found'
            ? `${report.world.name} is ${report.world.visibility}. Discovery verified its signed announcement; a signed invite is still required when membership policy is invite-only.`
            : report.detail || `World resolution: ${report.state}`,
          report.state === 'found' ? 'safe' : 'warning',
        );
      } catch (error) {
        setLocalStatus(status, errorText(error), 'danger');
      }
    },
    true,
  );

  return { roots };
}

if (typeof window !== 'undefined' && typeof document !== 'undefined') {
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', install, { once: true });
  else install();
}
