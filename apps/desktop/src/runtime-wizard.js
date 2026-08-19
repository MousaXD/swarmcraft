const WIZARD_MARK = 'swarmcraftRuntimeWizardRegistered';

const PHASE_COPY = Object.freeze({
  checking: 'Checking runtime',
  downloading_java: 'Downloading Java',
  downloading_server: 'Downloading Minecraft server',
  installing_fabric: 'Preparing Fabric',
  installing_fabric_api: 'Installing Fabric API',
  installing_swarmcraft_mod: 'Installing SwarmCraft integration',
  preparing_directories: 'Preparing world directories',
  verifying: 'Verifying runtime',
  ready: 'Ready',
  failed: 'Setup failed',
});

function byId(id) {
  return document.getElementById(id);
}

function setTopStatus(message, tone = 'neutral') {
  const status = byId('status');
  const dot = byId('statusDot');
  if (status) status.textContent = message;
  if (dot) dot.className = `status-dot ${tone}`;
}

function setWorldNotice(message, tone = 'neutral') {
  const notice = byId('worldNotice');
  if (!notice) return;
  notice.textContent = message;
  notice.hidden = !message;
  notice.dataset.tone = tone;
}

function setActivity(message, label) {
  const output = byId('output');
  if (!output) return;
  const stamp = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  const entry = `${stamp}  ${label}\n${message}`;
  output.textContent = output.textContent.trim() === 'SwarmCraft is ready.' || output.textContent.trim() === 'Activity cleared.'
    ? entry
    : `${output.textContent.trim()}\n\n${entry}`;
}

function ensureStyles() {
  if (document.querySelector('link[data-runtime-wizard-style]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = './runtime-wizard.css';
  link.dataset.runtimeWizardStyle = 'true';
  document.head.append(link);
}

function ensureDialog() {
  let dialog = byId('runtimeSetupDialog');
  if (dialog) return dialog;

  dialog = document.createElement('dialog');
  dialog.id = 'runtimeSetupDialog';
  dialog.className = 'dialog runtime-setup-dialog';
  dialog.setAttribute('aria-labelledby', 'runtimeSetupTitle');
  dialog.innerHTML = `
    <div class="runtime-wizard-shell">
      <header class="runtime-wizard-header">
        <div>
          <span class="runtime-wizard-kicker">Minecraft setup</span>
          <h2 id="runtimeSetupTitle">Set up this world</h2>
          <p id="runtimeSetupIntro">SwarmCraft will check the server runtime required by this world.</p>
        </div>
        <button id="runtimeSetupClose" class="icon-button" type="button" aria-label="Close setup">×</button>
      </header>

      <div id="runtimeWizardCapability" class="runtime-capability" role="status" aria-live="polite"></div>

      <ol id="runtimeComponentList" class="runtime-component-list" aria-label="Runtime components"></ol>

      <section id="runtimeEulaStep" class="runtime-eula-step" hidden aria-labelledby="runtimeEulaTitle">
        <h3 id="runtimeEulaTitle">Minecraft server EULA</h3>
        <p>Minecraft requires accepting its server EULA before SwarmCraft may start the server.</p>
        <label class="runtime-eula-check" for="runtimeEulaAccept">
          <input id="runtimeEulaAccept" type="checkbox" />
          <span>I accept the Minecraft server EULA.</span>
        </label>
        <p class="runtime-wizard-note">SwarmCraft only records acceptance after you choose to accept it here.</p>
      </section>

      <section id="runtimeFailureStep" class="runtime-failure-step" hidden aria-labelledby="runtimeFailureTitle">
        <h3 id="runtimeFailureTitle">Setup couldn't finish</h3>
        <p id="runtimeFailureMessage"></p>
        <dl class="runtime-safety-grid">
          <div><dt>World data</dt><dd id="runtimeWorldSafety">Not reported</dd></div>
          <div><dt>Retry</dt><dd id="runtimeRetrySafety">Not reported</dd></div>
        </dl>
        <details class="runtime-advanced-details">
          <summary>Advanced details</summary>
          <pre id="runtimeFailureDetails" tabindex="0"></pre>
        </details>
      </section>

      <footer class="runtime-wizard-actions">
        <button id="runtimeAdvancedSetup" class="button button-subtle" type="button">Advanced setup</button>
        <div class="runtime-wizard-primary-actions">
          <button id="runtimeRetrySetup" class="button button-secondary" type="button" hidden>Retry</button>
          <button id="runtimeSetupContinue" class="button button-primary" type="button">Continue</button>
        </div>
      </footer>
    </div>`;
  document.body.append(dialog);
  return dialog;
}

function stateGlyph(state) {
  if (state === 'ready') return '✓';
  if (state === 'working') return '●';
  if (state === 'missing') return '○';
  if (state === 'incompatible' || state === 'corrupt' || state === 'failed') return '!';
  return '·';
}

function componentStateLabel(component) {
  const state = component.state || 'unknown';
  if (state === 'ready') return component.version ? `Ready · ${component.version}` : 'Ready';
  if (state === 'working') return component.detail || 'Preparing…';
  if (state === 'missing') return component.detail || 'Missing';
  if (state === 'incompatible') return component.detail || 'Incompatible';
  if (state === 'corrupt') return component.detail || 'Needs repair';
  return component.detail || 'Checking…';
}

function renderComponents(status) {
  const list = byId('runtimeComponentList');
  if (!list) return;
  list.replaceChildren();
  for (const component of status?.components || []) {
    const item = document.createElement('li');
    item.className = `runtime-component runtime-component-${component.state || 'unknown'}`;
    const glyph = document.createElement('span');
    glyph.className = 'runtime-component-glyph';
    glyph.setAttribute('aria-hidden', 'true');
    glyph.textContent = stateGlyph(component.state);
    const copy = document.createElement('span');
    copy.className = 'runtime-component-copy';
    const label = document.createElement('strong');
    label.textContent = component.label;
    const detail = document.createElement('span');
    detail.textContent = componentStateLabel(component);
    copy.append(label, detail);
    item.append(glyph, copy);
    list.append(item);
  }
}

function displaySafety(value, positive, negative) {
  if (value === true) return positive;
  if (value === false) return negative;
  return 'Not reported by backend';
}

function renderFailure(error, status = null) {
  byId('runtimeFailureStep').hidden = false;
  byId('runtimeEulaStep').hidden = true;
  byId('runtimeRetrySetup').hidden = status?.retrySafe !== true;
  byId('runtimeSetupContinue').hidden = true;
  byId('runtimeFailureMessage').textContent = status?.detail || String(error || 'Runtime setup failed.');
  byId('runtimeWorldSafety').textContent = displaySafety(
    status?.worldDataSafe,
    'Backend reports world data is safe',
    'Backend reports world data needs attention',
  );
  byId('runtimeRetrySafety').textContent = displaySafety(
    status?.retrySafe,
    'Backend reports retry is safe',
    'Do not retry automatically',
  );
  byId('runtimeFailureDetails').textContent = status?.diagnosticDetail || String(error || 'No additional details were reported.');
  byId('runtimeWizardCapability').textContent = 'Minecraft was not started.';
  byId('runtimeWizardCapability').dataset.tone = 'danger';
}

function renderUnavailable(error) {
  byId('runtimeFailureStep').hidden = false;
  byId('runtimeEulaStep').hidden = true;
  byId('runtimeRetrySetup').hidden = true;
  byId('runtimeSetupContinue').hidden = true;
  byId('runtimeFailureMessage').textContent = 'Automatic Minecraft setup is not available in this build.';
  byId('runtimeWorldSafety').textContent = 'No automatic setup was attempted';
  byId('runtimeRetrySafety').textContent = 'Use Advanced setup or install a build with runtime installer support';
  byId('runtimeFailureDetails').textContent = String(error || 'Runtime installer capability is unavailable.');
  byId('runtimeWizardCapability').textContent = 'Automatic setup unavailable';
  byId('runtimeWizardCapability').dataset.tone = 'warning';
}

function showEula(status) {
  renderComponents(status);
  byId('runtimeFailureStep').hidden = true;
  byId('runtimeEulaStep').hidden = false;
  const checkbox = byId('runtimeEulaAccept');
  checkbox.checked = Boolean(status?.eulaAccepted);
  byId('runtimeSetupContinue').hidden = false;
  byId('runtimeSetupContinue').disabled = !checkbox.checked;
  byId('runtimeSetupContinue').textContent = 'Continue';
  byId('runtimeWizardCapability').textContent = 'EULA acceptance required';
  byId('runtimeWizardCapability').dataset.tone = 'warning';
  checkbox.focus();
}

function showPreparation(status) {
  renderComponents(status);
  byId('runtimeFailureStep').hidden = true;
  byId('runtimeEulaStep').hidden = true;
  byId('runtimeRetrySetup').hidden = true;
  byId('runtimeSetupContinue').hidden = false;
  byId('runtimeSetupContinue').disabled = false;
  byId('runtimeSetupContinue').textContent = 'Prepare server';
  const phase = PHASE_COPY[status?.phase] || 'Setup required';
  byId('runtimeWizardCapability').textContent = status?.detail || phase;
  byId('runtimeWizardCapability').dataset.tone = 'neutral';
}

function showInstalling(status) {
  renderComponents(status);
  byId('runtimeFailureStep').hidden = true;
  byId('runtimeEulaStep').hidden = true;
  byId('runtimeRetrySetup').hidden = true;
  const button = byId('runtimeSetupContinue');
  button.hidden = false;
  button.disabled = true;
  button.setAttribute('aria-busy', 'true');
  button.textContent = 'Preparing…';
  const phase = PHASE_COPY[status?.phase] || status?.detail || 'Preparing server';
  byId('runtimeWizardCapability').textContent = phase;
  byId('runtimeWizardCapability').dataset.tone = 'working';
}

function resetBusy() {
  const button = byId('runtimeSetupContinue');
  if (!button) return;
  button.removeAttribute('aria-busy');
}

function openAdvancedSetup(dialog) {
  dialog.close();
  const diagnostics = document.querySelector('.nav-item[data-view="diagnostics"]');
  diagnostics?.click();
  requestAnimationFrame(() => {
    byId('runtimeSection')?.scrollIntoView({ block: 'start', behavior: 'smooth' });
    byId('java')?.focus({ preventScroll: true });
    const notice = byId('runtimeNotice');
    if (notice) {
      notice.textContent = 'Advanced setup uses explicit local runtime paths. Automatic compatibility decisions remain in the backend.';
      notice.hidden = false;
      notice.dataset.tone = 'warning';
    }
  });
}

async function pollRuntimeStatus(backend, world, shouldStop, onStatus) {
  while (!shouldStop()) {
    await new Promise((resolve) => setTimeout(resolve, 650));
    if (shouldStop()) return;
    try {
      const status = await backend.runtime.status(world);
      onStatus(status);
    } catch (_) {
      // Installation owns the actionable error. Polling is best-effort progress only.
    }
  }
}

function markPlayBusy(button, busy) {
  if (!button) return;
  if (busy) {
    button.dataset.runtimeWizardBusy = 'true';
    button.setAttribute('aria-busy', 'true');
    button.dataset.runtimeWizardLabel = button.textContent;
    button.textContent = 'Checking…';
  } else {
    button.dataset.runtimeWizardBusy = 'false';
    button.removeAttribute('aria-busy');
    button.textContent = button.dataset.runtimeWizardLabel || 'Play';
  }
}

function currentWorldContext() {
  return {
    id: byId('world')?.value?.trim() || '',
    name: byId('selectedName')?.textContent?.trim() || 'this world',
    minecraft: byId('selectedMinecraft')?.textContent?.trim() || 'the selected Minecraft version',
  };
}

function setDialogContext(world) {
  byId('runtimeSetupTitle').textContent = `Set up ${world.name}`;
  byId('runtimeSetupIntro').textContent = `SwarmCraft will prepare the server runtime for Minecraft ${world.minecraft}.`;
  byId('runtimeEulaAccept').checked = false;
  byId('runtimeFailureStep').hidden = true;
  byId('runtimeEulaStep').hidden = true;
  byId('runtimeRetrySetup').hidden = true;
  byId('runtimeSetupContinue').hidden = false;
  byId('runtimeSetupContinue').disabled = true;
  byId('runtimeSetupContinue').textContent = 'Continue';
  byId('runtimeWizardCapability').textContent = 'Checking runtime…';
  byId('runtimeWizardCapability').dataset.tone = 'working';
}

async function launchPreparedWorld(backend, world, status, dialog = null) {
  if (!status.ready) throw new Error('Runtime launch was requested before the backend reported ready.');
  setTopStatus('Starting Minecraft…');
  try {
    const pid = await backend.runtime.launch(world.id);
    dialog?.close();
    setWorldNotice(`Minecraft started${pid ? ` · PID ${pid}` : ''}.`, 'safe');
    setActivity(`Minecraft runtime started${pid ? ` with PID ${pid}` : ''}.`, 'Play');
    setTopStatus('Ready', 'safe');
    return pid;
  } catch (error) {
    if (error?.code === 'SWARMCRAFT_CAPABILITY_UNAVAILABLE') throw error;
    setTopStatus('Minecraft could not start', 'danger');
    throw error;
  }
}

export function registerRuntimeWizard(backend) {
  if (typeof document === 'undefined' || !backend?.runtime) return;
  if (document.documentElement.dataset[WIZARD_MARK] === 'true') return;
  document.documentElement.dataset[WIZARD_MARK] = 'true';

  ensureStyles();
  const dialog = ensureDialog();
  let previousFocus = null;
  let world = null;
  let latestStatus = null;
  let acceptedEula = false;
  let installGeneration = 0;

  const closeDialog = () => {
    if (dialog.open) dialog.close();
    resetBusy();
    previousFocus?.focus?.({ preventScroll: true });
  };

  byId('runtimeSetupClose').addEventListener('click', closeDialog);
  dialog.addEventListener('cancel', (event) => {
    event.preventDefault();
    closeDialog();
  });
  dialog.addEventListener('close', () => previousFocus?.focus?.({ preventScroll: true }));
  byId('runtimeAdvancedSetup').addEventListener('click', () => openAdvancedSetup(dialog));
  byId('runtimeEulaAccept').addEventListener('change', (event) => {
    acceptedEula = event.currentTarget.checked;
    byId('runtimeSetupContinue').disabled = !acceptedEula;
  });

  const inspectForPlay = async (playButton) => {
    world = currentWorldContext();
    if (!world.id) return;
    previousFocus = playButton;
    markPlayBusy(playButton, true);
    setTopStatus('Checking Minecraft setup…');
    try {
      const status = await backend.runtime.status(world.id);
      latestStatus = status;
      if (status.ready) {
        await launchPreparedWorld(backend, world, status);
        return;
      }
      setDialogContext(world);
      dialog.showModal();
      renderComponents(status);
      if (status.eulaRequired && !status.eulaAccepted) showEula(status);
      else showPreparation(status);
      byId('runtimeSetupContinue').focus({ preventScroll: true });
    } catch (error) {
      setDialogContext(world);
      dialog.showModal();
      if (error?.code === 'SWARMCRAFT_CAPABILITY_UNAVAILABLE') renderUnavailable(error);
      else renderFailure(error, latestStatus);
      setTopStatus('Runtime setup needs attention', 'warning');
    } finally {
      markPlayBusy(playButton, false);
    }
  };

  const install = async () => {
    if (!world?.id) return;
    const generation = ++installGeneration;
    showInstalling(latestStatus);
    setTopStatus('Preparing Minecraft server…');
    let stopped = false;
    const polling = pollRuntimeStatus(
      backend,
      world.id,
      () => stopped || generation !== installGeneration,
      (status) => {
        latestStatus = status;
        showInstalling(status);
      },
    );
    try {
      latestStatus = await backend.runtime.install(world.id, { acceptEula: acceptedEula });
      stopped = true;
      await polling;
      latestStatus = await backend.runtime.verify(world.id);
      if (latestStatus.eulaRequired && !latestStatus.eulaAccepted) {
        showEula(latestStatus);
        resetBusy();
        return;
      }
      if (!latestStatus.ready) {
        showPreparation(latestStatus);
        resetBusy();
        return;
      }
      showInstalling({ ...latestStatus, phase: 'ready' });
      byId('runtimeWizardCapability').textContent = 'Runtime verified. Starting Minecraft…';
      await launchPreparedWorld(backend, world, latestStatus, dialog);
    } catch (error) {
      stopped = true;
      await polling;
      resetBusy();
      if (error?.code === 'SWARMCRAFT_CAPABILITY_UNAVAILABLE') renderUnavailable(error);
      else renderFailure(error, latestStatus);
      setTopStatus('Runtime setup needs attention', 'warning');
    }
  };

  byId('runtimeSetupContinue').addEventListener('click', async () => {
    if (byId('runtimeEulaStep').hidden === false && !acceptedEula) return;
    await install();
  });

  byId('runtimeRetrySetup').addEventListener('click', async () => {
    if (latestStatus?.retrySafe !== true) return;
    await install();
  });

  document.addEventListener('click', (event) => {
    const play = event.target.closest?.('#playWorld');
    const migrationSetup = event.target.closest?.('#migrationRuntimeSetup');
    if (!play && !migrationSetup) return;
    if (play?.disabled) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (migrationSetup) {
      world = currentWorldContext();
      previousFocus = migrationSetup;
      setDialogContext(world);
      dialog.showModal();
      backend.runtime.status(world.id).then((status) => {
        latestStatus = status;
        if (status.eulaRequired && !status.eulaAccepted) showEula(status);
        else showPreparation(status);
      }).catch((error) => {
        if (error?.code === 'SWARMCRAFT_CAPABILITY_UNAVAILABLE') renderUnavailable(error);
        else renderFailure(error, latestStatus);
      });
      return;
    }
    if (play.dataset.runtimeWizardBusy === 'true') return;
    inspectForPlay(play);
  }, true);
}
