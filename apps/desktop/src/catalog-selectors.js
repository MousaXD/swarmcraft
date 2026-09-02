function nonEmptyString(value) {
  return typeof value === 'string' && value.trim() === value && value.length > 0;
}

function providerVersions(response, provider) {
  if (!response || typeof response !== 'object' || !Array.isArray(response.versions)) {
    throw new Error(`${provider} catalog response did not contain a versions array.`);
  }
  return response.versions;
}

export function normalizeMinecraftCatalog(response) {
  return providerVersions(response, 'Mojang').map((version) => {
    if (!version || !nonEmptyString(version.id) || !nonEmptyString(version.type) || !nonEmptyString(version.release_time)) {
      throw new Error('Mojang catalog contained an invalid Minecraft version entry.');
    }
    if (typeof version.supported !== 'boolean') {
      throw new Error('Mojang catalog omitted the supported flag.');
    }
    return {
      id: version.id,
      type: version.type,
      releaseTime: version.release_time,
      supported: version.supported,
    };
  });
}

export function normalizeFabricCatalog(response, minecraftVersion) {
  return providerVersions(response, 'Fabric').map((loader) => {
    if (!loader || !nonEmptyString(loader.version) || typeof loader.stable !== 'boolean') {
      throw new Error('Fabric catalog contained an invalid loader entry.');
    }
    if (loader.minecraft_version !== minecraftVersion) {
      throw new Error('Fabric catalog returned a loader for the wrong Minecraft version.');
    }
    return {
      version: loader.version,
      stable: loader.stable,
      minecraftVersion: loader.minecraft_version,
    };
  });
}

export function chooseMinecraftVersion(versions, preferred = '') {
  if (preferred && versions.some((version) => version.id === preferred && version.supported)) return preferred;
  return versions.find((version) => version.supported && version.type === 'release')?.id
    || versions.find((version) => version.supported)?.id
    || '';
}

export function chooseFabricLoader(versions, preferred = '') {
  if (preferred && versions.some((loader) => loader.version === preferred)) return preferred;
  return versions.find((loader) => loader.stable)?.version || versions[0]?.version || '';
}

function errorCode(error) {
  if (error && typeof error === 'object' && typeof error.code === 'string') return error.code;
  const text = String(error || '');
  const match = text.match(/(?:^|\b)(provider_unavailable|response_too_large|malformed_provider_response|empty_catalog|invalid_input|incompatible_fabric_selection|cache_unavailable|catalog_task_failed)(?:\b|:)/);
  return match?.[1] || '';
}

export function catalogErrorMessage(provider, error) {
  const code = errorCode(error);
  if (provider === 'mojang') {
    if (code === 'malformed_provider_response' || code === 'response_too_large') {
      return 'Mojang returned invalid version data. Retry.';
    }
    if (code === 'empty_catalog') return 'Mojang returned no supported Minecraft releases. Retry.';
    return 'Could not reach Mojang. Retry.';
  }
  if (code === 'malformed_provider_response' || code === 'response_too_large') {
    return 'Fabric Meta returned invalid loader data. Retry.';
  }
  return 'Could not reach Fabric Meta. Retry.';
}

export class CatalogSelectionState {
  constructor() {
    this.minecraft = '';
    this.fabricLoader = '';
    this.minecraftVersions = [];
    this.fabricVersions = [];
    this.loadingMinecraft = false;
    this.loadingFabric = false;
  }

  beginMinecraftLoad() {
    this.loadingMinecraft = true;
    this.loadingFabric = false;
    this.minecraft = '';
    this.fabricLoader = '';
    this.minecraftVersions = [];
    this.fabricVersions = [];
  }

  setMinecraftCatalog(versions, preferred = '') {
    this.minecraftVersions = versions;
    this.minecraft = chooseMinecraftVersion(versions, preferred);
    this.loadingMinecraft = false;
    this.fabricLoader = '';
    this.fabricVersions = [];
    return this.minecraft;
  }

  changeMinecraft(minecraftVersion) {
    if (minecraftVersion !== this.minecraft) {
      this.minecraft = minecraftVersion;
      this.fabricLoader = '';
      this.fabricVersions = [];
    }
  }

  beginFabricLoad() {
    this.loadingFabric = true;
    this.fabricLoader = '';
    this.fabricVersions = [];
  }

  setFabricCatalog(minecraftVersion, versions, preferred = '') {
    if (minecraftVersion !== this.minecraft) return false;
    this.fabricVersions = versions;
    this.fabricLoader = chooseFabricLoader(versions, preferred);
    this.loadingFabric = false;
    return true;
  }

  failMinecraftLoad() {
    this.loadingMinecraft = false;
    this.minecraft = '';
    this.fabricLoader = '';
    this.minecraftVersions = [];
    this.fabricVersions = [];
  }

  failFabricLoad() {
    this.loadingFabric = false;
    this.fabricLoader = '';
    this.fabricVersions = [];
  }

  get ready() {
    return !this.loadingMinecraft && !this.loadingFabric && Boolean(this.minecraft && this.fabricLoader);
  }
}

function appendOption(documentRef, select, value, label) {
  const option = documentRef.createElement('option');
  option.value = value;
  option.textContent = label;
  select.append(option);
}

function responseNotice(response, fallback) {
  if (response?.origin === 'stale_cache') {
    return response.warning || `Using cached official ${fallback} data because the provider is currently unavailable.`;
  }
  if (response?.origin === 'fresh_cache') return `${fallback} versions loaded from the local official-source cache.`;
  return `${fallback} versions are up to date.`;
}

function upgradeInputToSelect(documentRef, input, describedBy) {
  if (input?.tagName?.toLowerCase() === 'select') return input;
  if (!input) return null;
  const select = documentRef.createElement('select');
  select.id = input.id;
  select.required = input.required;
  select.disabled = true;
  if (describedBy) select.setAttribute('aria-describedby', describedBy);
  input.replaceWith(select);
  return select;
}

export function ensureCatalogUi(documentRef) {
  const form = documentRef?.getElementById('createForm');
  let minecraft = documentRef?.getElementById('createMinecraft');
  let fabric = documentRef?.getElementById('createLoader');
  if (!form || !minecraft || !fabric) return null;

  let status = documentRef.getElementById('createCatalogStatus');
  let error = documentRef.getElementById('createCatalogError');
  let retry = documentRef.getElementById('createCatalogRetry');
  let refresh = documentRef.getElementById('createCatalogRefresh');
  let snapshots = documentRef.getElementById('createSnapshots');
  const details = minecraft.closest('details.form-details') || minecraft.closest('details');

  if (!status && details) {
    const minecraftField = minecraft.closest('.field');
    const fabricField = fabric.closest('.field');
    const selectorGrid = documentRef.createElement('div');
    selectorGrid.className = 'field-grid field-wide catalog-selector-grid';
    details.before(selectorGrid);
    if (minecraftField) selectorGrid.append(minecraftField);
    if (fabricField) selectorGrid.append(fabricField);

    const summary = details.querySelector('summary');
    if (summary) summary.textContent = 'Advanced compatibility settings';

    status = documentRef.createElement('p');
    status.id = 'createCatalogStatus';
    status.className = 'field-help field-wide';
    status.setAttribute('role', 'status');
    status.setAttribute('aria-live', 'polite');
    status.textContent = 'Fetching Minecraft versions...';

    error = documentRef.createElement('div');
    error.id = 'createCatalogError';
    error.className = 'form-error field-wide';
    error.setAttribute('role', 'alert');
    error.hidden = true;

    const actions = documentRef.createElement('div');
    actions.className = 'compact-actions field-wide';
    retry = documentRef.createElement('button');
    retry.id = 'createCatalogRetry';
    retry.className = 'button button-secondary button-small';
    retry.type = 'button';
    retry.textContent = 'Retry';
    retry.hidden = true;
    refresh = documentRef.createElement('button');
    refresh.id = 'createCatalogRefresh';
    refresh.className = 'button button-subtle button-small';
    refresh.type = 'button';
    refresh.textContent = 'Refresh versions';
    actions.append(retry, refresh);
    details.before(status, error, actions);

    const detailsFields = details.querySelector('.details-fields');
    if (detailsFields && !snapshots) {
      const snapshotLabel = documentRef.createElement('label');
      snapshotLabel.className = 'check-row field-wide';
      snapshotLabel.setAttribute('for', 'createSnapshots');
      snapshots = documentRef.createElement('input');
      snapshots.id = 'createSnapshots';
      snapshots.type = 'checkbox';
      const copy = documentRef.createElement('span');
      copy.textContent = 'Show Minecraft snapshots in the version selector.';
      snapshotLabel.append(snapshots, copy);
      detailsFields.append(snapshotLabel);
    }

    const helper = details.querySelector('.helper');
    if (helper) {
      helper.textContent = 'The selected exact Minecraft and Fabric Loader versions become signed world metadata. They determine which devices are eligible to host, not which devices may store replicas.';
    }
  }

  minecraft = upgradeInputToSelect(documentRef, documentRef.getElementById('createMinecraft'), 'createCatalogStatus');
  fabric = upgradeInputToSelect(documentRef, documentRef.getElementById('createLoader'), 'createCatalogStatus');
  status = documentRef.getElementById('createCatalogStatus');
  error = documentRef.getElementById('createCatalogError');
  retry = documentRef.getElementById('createCatalogRetry');
  refresh = documentRef.getElementById('createCatalogRefresh');
  snapshots = documentRef.getElementById('createSnapshots');

  return {
    form,
    minecraft,
    fabric,
    snapshots,
    retry,
    refresh,
    status,
    error,
    createButton: documentRef.getElementById('createWorld'),
  };
}

export function registerCatalogSelectors({ invoke, documentRef } = {}) {
  const tauriInvoke = invoke || globalThis.window?.__TAURI__?.core?.invoke;
  const doc = documentRef || globalThis.document;
  if (!doc) return null;

  const ui = ensureCatalogUi(doc);
  if (!ui?.form || !ui.minecraft || !ui.fabric || !ui.status || !ui.error || !ui.createButton) return null;
  const {
    form,
    minecraft: minecraftSelect,
    fabric: fabricSelect,
    snapshots: snapshotToggle,
    retry: retryButton,
    refresh: refreshButton,
    status,
    error,
    createButton,
  } = ui;

  const state = new CatalogSelectionState();
  let minecraftRequest = 0;
  let fabricRequest = 0;
  let lastFailure = 'mojang';

  const setError = (message = '') => {
    error.textContent = message;
    error.hidden = !message;
    if (retryButton) retryButton.hidden = !message;
  };

  const syncCreateButton = () => {
    createButton.disabled = !state.ready;
  };

  const clearSelect = (select, label) => {
    select.replaceChildren();
    appendOption(doc, select, '', label);
    select.value = '';
    select.disabled = true;
  };

  const renderMinecraft = () => {
    minecraftSelect.replaceChildren();
    for (const version of state.minecraftVersions) {
      appendOption(doc, minecraftSelect, version.id, version.type === 'snapshot' ? `${version.id} (snapshot)` : version.id);
    }
    minecraftSelect.value = state.minecraft;
    minecraftSelect.disabled = !state.minecraftVersions.length;
  };

  const renderFabric = () => {
    fabricSelect.replaceChildren();
    for (const loader of state.fabricVersions) {
      appendOption(doc, fabricSelect, loader.version, loader.stable ? loader.version : `${loader.version} (unstable)`);
    }
    fabricSelect.value = state.fabricLoader;
    fabricSelect.disabled = !state.fabricVersions.length;
  };

  const loadFabric = async ({ refresh = false, preferred = '' } = {}) => {
    const minecraftVersion = state.minecraft;
    const request = ++fabricRequest;
    state.beginFabricLoad();
    clearSelect(fabricSelect, minecraftVersion ? 'Loading compatible loaders…' : 'Choose Minecraft first');
    setError('');
    syncCreateButton();
    if (!minecraftVersion) {
      state.failFabricLoad();
      status.textContent = 'Choose a Minecraft version first.';
      return;
    }
    status.textContent = `Fetching compatible Fabric Loader versions for Minecraft ${minecraftVersion}...`;
    try {
      if (typeof tauriInvoke !== 'function') throw new Error('provider_unavailable: Desktop bridge unavailable');
      const response = await tauriInvoke('fabric_loader_versions', { minecraftVersion, refresh });
      if (request !== fabricRequest || minecraftVersion !== state.minecraft) return;
      const loaders = normalizeFabricCatalog(response, minecraftVersion);
      state.setFabricCatalog(minecraftVersion, loaders, preferred);
      renderFabric();
      if (!loaders.length) {
        lastFailure = 'fabric';
        setError(`Fabric does not publish a compatible loader for Minecraft ${minecraftVersion}. Choose another Minecraft version.`);
        status.textContent = 'No compatible Fabric Loader is available for this Minecraft version.';
      } else {
        status.textContent = responseNotice(response, 'Fabric Loader');
      }
    } catch (catalogError) {
      if (request !== fabricRequest || minecraftVersion !== state.minecraft) return;
      state.failFabricLoad();
      clearSelect(fabricSelect, 'Fabric Loader unavailable');
      lastFailure = 'fabric';
      setError(catalogErrorMessage('fabric', catalogError));
      status.textContent = 'Fabric Loader choices are unavailable.';
    }
    syncCreateButton();
  };

  const loadMinecraft = async ({ refresh = false, preferred = '' } = {}) => {
    const request = ++minecraftRequest;
    fabricRequest += 1;
    state.beginMinecraftLoad();
    clearSelect(minecraftSelect, 'Loading Minecraft versions…');
    clearSelect(fabricSelect, 'Choose Minecraft first');
    setError('');
    syncCreateButton();
    status.textContent = 'Fetching Minecraft versions...';
    try {
      if (typeof tauriInvoke !== 'function') throw new Error('provider_unavailable: Desktop bridge unavailable');
      const response = await tauriInvoke('minecraft_versions', {
        includeSnapshots: snapshotToggle?.checked === true,
        refresh,
      });
      if (request !== minecraftRequest) return;
      const versions = normalizeMinecraftCatalog(response).filter((version) => version.supported);
      state.setMinecraftCatalog(versions, preferred);
      renderMinecraft();
      if (!state.minecraft) {
        lastFailure = 'mojang';
        setError('Mojang returned no supported Minecraft releases. Retry.');
        status.textContent = 'Minecraft versions are unavailable.';
        syncCreateButton();
        return;
      }
      status.textContent = responseNotice(response, 'Minecraft');
      await loadFabric({ refresh, preferred: '' });
    } catch (catalogError) {
      if (request !== minecraftRequest) return;
      state.failMinecraftLoad();
      clearSelect(minecraftSelect, 'Minecraft versions unavailable');
      clearSelect(fabricSelect, 'Fabric Loader unavailable');
      lastFailure = 'mojang';
      setError(catalogErrorMessage('mojang', catalogError));
      status.textContent = 'Minecraft versions are unavailable.';
      syncCreateButton();
    }
  };

  minecraftSelect.addEventListener('change', () => {
    state.changeMinecraft(minecraftSelect.value);
    loadFabric({ refresh: false, preferred: '' });
  });

  snapshotToggle?.addEventListener('change', () => {
    const preferred = state.minecraft;
    loadMinecraft({ refresh: false, preferred });
  });

  refreshButton?.addEventListener('click', () => {
    const preferred = state.minecraft;
    loadMinecraft({ refresh: true, preferred });
  });

  retryButton?.addEventListener('click', () => {
    if (lastFailure === 'fabric' && state.minecraft) loadFabric({ refresh: true });
    else loadMinecraft({ refresh: true });
  });

  form.addEventListener('submit', (event) => {
    if (state.ready) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    setError('Choose a compatible Minecraft and Fabric Loader version before creating the world.');
    if (!state.minecraft) minecraftSelect.focus();
    else fabricSelect.focus();
  }, true);

  form.addEventListener('reset', () => {
    queueMicrotask(() => loadMinecraft({ refresh: false }));
  });

  loadMinecraft({ refresh: false });
  return { state, reload: loadMinecraft, reloadFabric: loadFabric };
}

if (typeof window !== 'undefined' && typeof document !== 'undefined') {
  const start = () => registerCatalogSelectors();
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
  else start();
}
