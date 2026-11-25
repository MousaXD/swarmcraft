// A read-only manual launcher handoff. This never opens a process, changes a
// launcher installation, obtains credentials, or authorizes a game route.
const issueLabels = Object.freeze({
  folder_unavailable_or_limit: 'The selected mods folder is unavailable or exceeds the inspection limits.',
  invalid_jar: 'A JAR is unreadable, invalid, or not a regular Fabric mod file.',
  server_only_or_unknown: 'This JAR is server-only or its client side is unknown.',
  duplicate_mod: 'Multiple JARs claim the same mod ID.',
  missing_required: 'Required client mod is missing.',
  version_mismatch: 'Installed mod version differs from the signed requirement.',
  hash_mismatch: 'Installed JAR bytes differ from the signed requirement.',
  additional_mod: 'Additional mod is not listed in the signed client profile; compatibility is not verified.',
});

const hash = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const token = value => typeof value === 'string' && value.length > 0 && value.length <= 256
  && !/[\x00-\x1f\x7f]/.test(value);

export function normalizeClientProfile(raw, world, checkedFolder) {
  const value = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (!value || value.world_id !== world || !hash(value.compatibility_fingerprint)
      || !token(value.minecraft_version) || !token(value.fabric_loader_version)
      || !Number.isSafeInteger(value.minimum_java_major) || value.minimum_java_major <= 0
      || !Array.isArray(value.required_mods) || !Array.isArray(value.issues)
      || !['not_checked', 'exact_match', 'needs_review'].includes(value.inventory)
      || (checkedFolder ? value.inventory === 'not_checked' : value.inventory !== 'not_checked')
      || value.required_mods.length > 2048 || value.issues.length > 4096
      || !value.required_mods.every(mod => token(mod?.mod_id) && token(mod?.version) && hash(mod?.artifact_hash))
      || !value.issues.every(issue => issue && Object.hasOwn(issueLabels, issue.code)
        && (issue.mod_id === null || token(issue.mod_id)))
      || (value.inventory === 'needs_review' ? value.issues.length === 0 : value.issues.length !== 0)
      || new Set(value.required_mods.map(mod => mod.mod_id)).size !== value.required_mods.length) {
    throw new Error('Invalid or mismatched native client profile.');
  }
  return value;
}

export function clientPreparationInstructions(profile, launcher) {
  const target = 'Minecraft ' + profile.minecraft_version + ' / Fabric Loader ' + profile.fabric_loader_version;
  const steps = {
    official: [
      'Use your existing Minecraft Launcher account. Create a separate Fabric installation and game directory with the official Fabric installer.',
      'Select exactly ' + target + ' in that installation.',
    ],
    prism: [
      'Use your existing Prism Launcher account. Create a separate instance rather than modifying an existing one.',
      'In the instance version settings, select exactly ' + target + '.',
    ],
    other: [
      'Use your existing Java Edition launcher account. Create a separate game instance with its own game directory.',
      'Configure exactly ' + target + ' in that instance.',
    ],
  };
  return [
    ...(steps[launcher] || steps.other),
    'Use the Java runtime recommended for that game version (at least Java ' + profile.minimum_java_major + ' for this adapter). Check the Java selected by your launcher, not merely your system Java.',
    'Obtain the exact required client JARs from authorized sources, place them in this instance’s mods folder, and check that folder below. Do not install the server-only SwarmCraft bridge on the client.',
    'Launch Java Edition yourself with the existing account. Once it is running, use Multiplayer and SwarmCraft’s current verified reconnect instructions. A matching inventory is not login, runtime, dependency or connection proof.',
  ].join('\n');
}

export function installClientPreparation(panel, backend) {
  const $ = id => panel.querySelector('#' + id);
  const launcher = $('clientLauncher');
  const folder = $('clientModsFolder');
  const check = $('checkClientProfile');
  const requirements = $('clientProfileRequirements');
  const instructions = $('clientLauncherInstructions');
  const status = $('clientInventoryStatus');
  let selected = '';
  let generation = 0;
  let report = null;
  let busy = false;
  const sync = () => {
    check.disabled = !selected || busy;
    check.setAttribute('aria-busy', String(busy));
  };
  const renderInstructions = () => {
    instructions.textContent = report ? clientPreparationInstructions(report, launcher.value) : '';
    instructions.hidden = !report;
  };
  const showStatus = (message, state = 'neutral') => {
    status.textContent = message;
    status.dataset.state = state;
  };
  const clear = () => {
    report = null;
    requirements.textContent = '';
    requirements.hidden = true;
    renderInstructions();
  };
  const invalidate = () => {
    generation += 1;
    busy = false;
    sync();
  };
  const load = async () => {
    if (!selected || !panel.open || busy) return;
    const world = selected;
    const modsDir = folder.value.trim() || null;
    const ticket = ++generation;
    busy = true;
    clear();
    showStatus('Checking the signed world requirements and selected client inventory…');
    sync();
    try {
      const raw = await backend.clientProfile(world, modsDir);
      if (ticket !== generation || world !== selected || !panel.open || modsDir !== (folder.value.trim() || null)) return;
      report = normalizeClientProfile(raw, world, Boolean(modsDir));
      requirements.textContent = [
        'Signed world profile: Minecraft ' + report.minecraft_version + ' · Fabric Loader ' + report.fabric_loader_version,
        'Minimum adapter Java: ' + report.minimum_java_major,
        report.required_mods.length ? 'Required client JARs (exact SwarmCraft artifact hashes):' : 'No third-party client JARs declared.',
        ...report.required_mods.map(mod => mod.mod_id + ' ' + mod.version + '\n' + mod.artifact_hash),
      ].join('\n');
      requirements.hidden = false;
      renderInstructions();
      if (report.inventory === 'exact_match') {
        showStatus('Selected top-level JAR inventory matches the signed client profile at the time of this check. Confirm this is the correct launcher instance. Java, game launch and login have not been verified.', 'checked');
      } else if (report.inventory === 'needs_review') {
        showStatus(['Client inventory needs review. Use a separate instance; do not delete mods from your existing profile.', ...report.issues.map(issue => (issue.mod_id ? issue.mod_id + ': ' : '') + issueLabels[issue.code])].join('\n'), 'warning');
      } else {
        showStatus('JAR inventory not checked. Enter the full path to the chosen instance’s mods folder, then check it.');
      }
    } catch {
      if (ticket === generation && world === selected) {
        clear();
        showStatus('Signed client requirements unavailable. Synchronize or refresh the world, then retry. Do not guess a matching profile.', 'warning');
      }
    } finally {
      if (ticket === generation) {
        busy = false;
        sync();
      }
    }
  };
  panel.addEventListener('toggle', () => {
    if (panel.open) void load();
    else {
      invalidate();
      clear();
      showStatus('Open to check the signed client requirements.');
    }
  });
  launcher.addEventListener('change', renderInstructions);
  folder.addEventListener('input', () => {
    invalidate();
    // Preserve the immutable requirements, but never display a stale inventory
    // result after a path change (even when switching back to the old path).
    showStatus('Client inventory not checked for this selection. Check the folder again.');
  });
  check.addEventListener('click', () => { void load(); });
  sync();
  return Object.freeze({
    select(world) {
      if (world === selected) return;
      selected = world;
      invalidate();
      clear();
      panel.open = false;
      folder.value = '';
      showStatus('Open to check the signed client requirements.');
      sync();
    },
  });
}
