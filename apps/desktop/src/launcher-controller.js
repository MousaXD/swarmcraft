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

function setCreateMessage(message, kind = 'info') {
  const target = byId('createError');
  if (!target) return;
  target.textContent = message;
  target.dataset.kind = kind;
}

function makeButton(label, onClick, secondary = false) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = label;
  if (secondary) button.className = 'secondary';
  button.addEventListener('click', onClick);
  return button;
}

function installModsUi() {
  const form = byId('createForm');
  const submit = form?.querySelector('button[type="submit"]');
  if (!form || !submit || byId('launcherMods')) return;
  const section = document.createElement('section');
  section.id = 'launcherMods';
  section.className = 'form-section';
  section.innerHTML = `
    <h3>Mods</h3>
    <p class="muted">Search official provider catalogs. SwarmCraft resolves exact compatible files and required dependencies before the world is created.</p>
    <div class="field-grid">
      <label>Provider
        <select id="modProvider">
          <option value="modrinth">Modrinth</option>
          <option value="curseforge">CurseForge</option>
        </select>
      </label>
      <label>Search mods
        <input id="modSearch" type="search" placeholder="e.g. Lithium" autocomplete="off" />
      </label>
    </div>
    <div class="actions"><button id="modSearchButton" type="button" class="secondary">Search</button></div>
    <div id="modSearchStatus" class="muted" aria-live="polite"></div>
    <div id="modSearchResults" class="stack"></div>
    <h4>Selected mods</h4>
    <div id="selectedMods" class="stack"><p class="muted">No third-party mods selected.</p></div>`;
  form.insertBefore(section, submit);
}

function installDiscoveryUi() {
  const form = byId('joinForm');
  if (!form || byId('publicWorldDiscovery')) return;
  const section = document.createElement('section');
  section.id = 'publicWorldDiscovery';
  section.className = 'form-section';
  section.innerHTML = `
    <h3>Public worlds</h3>
    <p class="muted">Browse authenticated public announcements. Discovery never grants membership; invite-only worlds still require a signed invite.</p>
    <div class="field-grid"><label>Search public worlds<input id="publicWorldQuery" type="search" placeholder="World name or tag" /></label></div>
    <div class="actions"><button id="publicWorldSearch" type="button" class="secondary">Search</button></div>
    <div id="publicWorldStatus" class="muted" aria-live="polite"></div>
    <div id="publicWorldResults" class="stack"></div>`;
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

async function hydrateImportCatalogs(call) {
  const minecraftInput = byId('importMinecraft');
  const loaderInput = byId('importLoader');
  if (!minecraftInput || !loaderInput || minecraftInput.tagName === 'SELECT') return;
  const minecraftSelect = document.createElement('select');
  minecraftSelect.id = minecraftInput.id;
  minecraftSelect.name = minecraftInput.name;
  minecraftSelect.required = true;
  minecraftInput.replaceWith(minecraftSelect);
  const loaderSelect = document.createElement('select');
  loaderSelect.id = loaderInput.id;
  loaderSelect.name = loaderInput.name;
  loaderSelect.required = true;
  loaderInput.replaceWith(loaderSelect);
  const catalog = await call('minecraft_versions');
  const versions = catalog.versions || catalog.items || catalog;
  for (const item of versions || []) {
    const id = clean(item.id ?? item.version ?? item);
    if (id) minecraftSelect.add(new Option(id, id));
  }
  async function loadFabric() {
    loaderSelect.replaceChildren(new Option('Loading compatible loaders…', ''));
    loaderSelect.disabled = true;
    const result = await call('fabric_loader_versions', { minecraftVersion: minecraftSelect.value });
    const values = result.loaders || result.versions || result.items || result;
    loaderSelect.replaceChildren();
    for (const item of values || []) {
      const id = clean(item.version ?? item.id ?? item);
      if (id) loaderSelect.add(new Option(id, id));
    }
    loaderSelect.disabled = false;
  }
  minecraftSelect.addEventListener('change', () => loadFabric().catch(() => {}));
  if (minecraftSelect.options.length) {
    minecraftSelect.selectedIndex = 0;
    await loadFabric();
  }
}

function install() {
  const call = invoke();
  if (!call) return;
  hideInternalInputs();
  installModsUi();
  installDiscoveryUi();
  hydrateImportCatalogs(call).catch((error) => console.warn('Import catalog unavailable', error));

  const roots = [];
  const renderSelected = () => {
    const target = byId('selectedMods');
    if (!target) return;
    target.replaceChildren();
    if (!roots.length) {
      const empty = document.createElement('p');
      empty.className = 'muted';
      empty.textContent = 'No third-party mods selected.';
      target.append(empty);
      return;
    }
    for (const root of roots) {
      const row = document.createElement('div');
      row.className = 'card-row';
      const text = document.createElement('span');
      text.textContent = `${root.title} · ${root.provider === 'modrinth' ? 'Modrinth' : 'CurseForge'}`;
      row.append(
        text,
        makeButton(
          'Remove',
          () => {
            roots.splice(roots.indexOf(root), 1);
            renderSelected();
          },
          true,
        ),
      );
      target.append(row);
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
        const row = document.createElement('div');
        row.className = 'card-row';
        const text = document.createElement('span');
        text.textContent = title;
        row.append(text, makeButton('Add', () => addRoot({ provider, projectId, title })));
        results.append(row);
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
        for (const pkg of unique.values()) {
          await call('world_mods_add', { world: created.worldId, jarPath: pkg.artifactPath });
        }
        setCreateMessage(`World created with canonical fingerprint ${created.canonical?.compatibilityFingerprint || ''}.`, 'success');
        window.setTimeout(() => window.location.reload(), 250);
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
    status.textContent = 'Searching authenticated public announcements…';
    results.replaceChildren();
    try {
      const report = await call('discovery_search', { query: clean(byId('publicWorldQuery').value) || null });
      status.textContent =
        report.detail ||
        (report.results?.length
          ? `${report.results.length} public world${report.results.length === 1 ? '' : 's'} found.`
          : 'No public worlds found.');
      for (const world of report.results || []) {
        const row = document.createElement('div');
        row.className = 'card-row';
        const text = document.createElement('span');
        text.textContent = `${world.name} · Minecraft ${world.minecraft_version} · Fabric ${world.loader_version}`;
        const action = makeButton(
          world.join_action === 'invite_required' ? 'Invite required' : 'View',
          () => {
            byId('joinWorldId').value = world.world_id;
            status.textContent =
              world.join_action === 'invite_required'
                ? `${world.name} is public to discover, but its authority still requires a signed invite for membership.`
                : `${world.name} was discovered. Membership is still decided by the world authority.`;
          },
          true,
        );
        row.append(text, action);
        results.append(row);
      }
    } catch (error) {
      status.textContent = errorText(error);
    }
  });

  byId('joinWorldIdButton')?.addEventListener(
    'click',
    async (event) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      const world = clean(byId('joinWorldId').value);
      if (!world) return;
      const status = byId('publicWorldStatus') || byId('joinError');
      try {
        const report = await call('discovery_resolve', { world });
        status.textContent =
          report.state === 'found'
            ? `${report.world.name} is ${report.world.visibility}. Discovery verified its signed announcement; a signed invite is still required when membership policy is invite-only.`
            : report.detail || `World resolution: ${report.state}`;
      } catch (error) {
        status.textContent = errorText(error);
      }
    },
    true,
  );
}

if (typeof window !== 'undefined' && typeof document !== 'undefined') {
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', install, { once: true });
  else install();
}
