// Advisory native import review shared by the shipping and Svelte forms.
// The Rust importer is still the final authority for source locking,
// compatibility, storage admission and canonical publication.
const blockers = Object.freeze({
  invalid_details: 'Enter the world name and exact Minecraft/Fabric versions.',
  invalid_source: 'Select the actual Minecraft world folder containing a readable, non-empty level.dat.',
  unsupported_runtime: 'This Minecraft/Fabric combination is not supported by the installed SwarmCraft adapter.',
  unqualified_runtime: 'This combination is within the declared Fabric adapter range but is not offered for new worlds. Choose the pinned baseline only if it matches your actual save; otherwise keep the original save unchanged.',
  mod_requirements: 'Check every required server mod JAR, or explicitly confirm that no third-party server mods are required.',
  source_not_quiescent: 'Close Minecraft or the source server. The existing session.lock must be accessible and unlocked.',
  source_unreadable: 'The save contains an unreadable entry, symbolic link or special file. Review the source folder.',
});

function token(value) {
  return typeof value === 'string' && value.length > 0 && value.length <= 256
    && !/[\x00-\x1f\x7f]/.test(value);
}

export function normalizeImportPreview(raw, request) {
  const value = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (value?.state === 'blocked' && Object.hasOwn(blockers, value.blocker)) {
    return { state: 'blocked', message: blockers[value.blocker] };
  }
  if (value?.state !== 'ready'
      || value.minecraft_version !== request.minecraft
      || value.fabric_loader_version !== request.fabricLoader
      || !Number.isSafeInteger(value.minimum_java_major) || value.minimum_java_major <= 0
      || !Number.isSafeInteger(value.source_files) || value.source_files < 2
      || !Number.isSafeInteger(value.source_bytes) || value.source_bytes <= 0
      || !Array.isArray(value.required_server_mods)
      || value.required_server_mods.length !== request.serverMods.length
      || !value.required_server_mods.every(mod => token(mod?.artifact_id) && token(mod?.version))
      || !/^[a-f0-9]{64}$/.test(value.compatibility_fingerprint ?? '')) {
    throw new Error('Invalid native import review.');
  }
  const mods = value.required_server_mods.map(mod => mod.artifact_id + ' ' + mod.version);
  if (new Set(value.required_server_mods.map(mod => mod.artifact_id)).size !== mods.length) {
    throw new Error('Duplicate native server-mod requirements.');
  }
  const size = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value.source_bytes / 1048576);
  return {
    state: 'ready',
    fingerprint: value.compatibility_fingerprint,
    message: [
      'Source check passed: ' + value.source_files.toLocaleString() + ' files, approximately ' + size + ' MiB before compression.',
      'Declared runtime: Minecraft ' + value.minecraft_version + ', Fabric Loader ' + value.fabric_loader_version
        + ', Java ' + value.minimum_java_major + ' or newer (minimum adapter requirement).',
      mods.length ? 'Required server mods: ' + mods.join(', ') + '. Matching JARs still need runtime setup.'
        : 'No third-party server mods declared.',
      'Confirm these are the actual versions and mods used by the save. They are not inferred from world bytes.',
      'Import checks the source again. It does not install Java, accept the server EULA, enable hosting, or reserve disk or memory capacity.',
    ].join('\n'),
  };
}

export function createImportReview() {
  let generation = 0;
  let reviewed = null;
  const key = request => JSON.stringify([
    request.source, request.name, request.minecraft, request.fabricLoader, request.visibility,
    request.serverMods, request.noServerMods,
  ]);
  function invalidate() {
    generation += 1;
    reviewed = null;
  }
  function fingerprint(request) {
    return reviewed && reviewed.key === key(request) ? reviewed.fingerprint : null;
  }
  return Object.freeze({
    invalidate,
    begin(request) {
      invalidate();
      return { generation, key: key(request) };
    },
    finish(ticket, raw, request) {
      if (ticket.generation !== generation || ticket.key !== key(request)) return null;
      const report = normalizeImportPreview(raw, request);
      if (report.state === 'ready') reviewed = { key: ticket.key, fingerprint: report.fingerprint };
      return report;
    },
    fingerprint,
    take(request) {
      const result = fingerprint(request);
      if (!result) throw new Error('Check this import before committing. Review the source and requirements again.');
      invalidate();
      return result;
    },
  });
}

export function installImportReview(form, backend, readRequest, clearImportError) {
  const source = form.querySelector('#importSource');
  const picker = form.querySelector('#pickImportFolder');
  const check = form.querySelector('#checkImport');
  const commit = form.querySelector('#importWorld');
  const summary = form.querySelector('#importReview');
  const review = createImportReview();
  let busy = false;
  const importing = () => form.dataset.busy === 'true' || form.getAttribute('aria-busy') === 'true';
  const current = () => {
    try { return readRequest(); } catch { return null; }
  };
  const sync = () => {
    const request = current();
    picker.disabled = check.disabled = busy || importing();
    commit.disabled = busy || importing() || !request || !review.fingerprint(request);
  };
  const show = (message, state = 'neutral') => {
    summary.textContent = message;
    summary.dataset.state = state;
    summary.hidden = !message;
  };
  const reset = () => {
    review.invalidate();
    show('');
    sync();
  };
  for (const event of ['input', 'change', 'reset']) form.addEventListener(event, reset);
  picker.addEventListener('click', async () => {
    if (busy || importing()) return;
    const before = source.value;
    busy = true;
    sync();
    try {
      const selected = await backend.pickImportFolder();
      // Cancel preserves the manual path. An out-of-date picker response
      // cannot overwrite a newer edit.
      if (typeof selected === 'string' && selected.length > 0 && source.value === before) {
        source.value = selected;
        source.dispatchEvent(new Event('input', { bubbles: true }));
        clearImportError();
      }
    } catch {
      reset();
      show('Native folder picker unavailable. Paste or type the world folder path instead.', 'blocked');
    } finally {
      busy = false;
      sync();
    }
  });
  check.addEventListener('click', async () => {
    if (busy || importing()) return;
    let request;
    try {
      request = readRequest();
    } catch (error) {
      reset();
      show(error.message, 'blocked');
      if (error.field) form.querySelector('#' + error.field)?.focus();
      return;
    }
    const ticket = review.begin(request);
    busy = true;
    clearImportError();
    show('Checking the source and declared runtime requirements…');
    sync();
    try {
      const raw = await backend.previewImport(request);
      const latest = current();
      const report = latest && review.finish(ticket, raw, latest);
      if (report) show(report.message, report.state);
    } catch {
      // An unavailable or malformed native reply never enables commit.
      const latest = current();
      if (latest && review.finish(ticket, { state: 'blocked', blocker: 'invalid_details' }, latest)) {
        show('Import review unavailable. Check the source and retry.', 'blocked');
      }
    } finally {
      busy = false;
      sync();
    }
  });
  sync();
  return Object.freeze({
    reset, sync,
    take(request) {
      if (busy) throw new Error('Wait for the folder selection or import check to finish.');
      const fingerprint = review.take(request);
      show('');
      sync();
      return fingerprint;
    },
  });
}
