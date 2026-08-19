const READY_WORDS = /ready|safe to shut down|direct connection|connected through relay/i;
const ACTION_WORDS = /blocked|missing|failed|unavailable|attention|conflict|keep this pc on|could not/i;
const WORKING_WORDS = /checking|preparing|saving|transferring|restoring|starting|waiting|syncing|installing|downloading|verifying/i;

export function deriveJourneyState({
  playDisabled = false,
  playDetail = '',
  hostReadiness = '',
  migration = '',
  connectivity = '',
} = {}) {
  const migrationText = String(migration || '').trim();
  const readinessText = String(hostReadiness || '').trim();
  const playText = String(playDetail || '').trim();
  const connectivityText = String(connectivity || '').trim();
  const combined = `${playText} ${readinessText} ${migrationText} ${connectivityText}`;

  if (migrationText && !/not active|sleeping/i.test(migrationText) && WORKING_WORDS.test(migrationText)) {
    return {
      kind: 'working',
      label: 'Host change in progress',
      detail: 'SwarmCraft is moving hosting safely. You can keep this window open while the handoff completes.',
    };
  }
  if (ACTION_WORDS.test(combined)) {
    return {
      kind: 'action',
      label: 'Needs attention',
      detail: playText || readinessText || 'Something needs to be resolved before this world is fully ready.',
    };
  }
  if (playDisabled || WORKING_WORDS.test(playText)) {
    return {
      kind: 'working',
      label: 'Getting things ready',
      detail: playText || 'SwarmCraft is checking this device and the world before Play becomes available.',
    };
  }
  if (/sleeping/i.test(migrationText)) {
    return {
      kind: 'sleeping',
      label: 'World is sleeping',
      detail: 'The latest world state is saved. Play will use the normal safe wake path when needed.',
    };
  }
  if (READY_WORDS.test(combined) || !playDisabled) {
    return {
      kind: 'ready',
      label: 'Ready to play',
      detail: playText || 'This world is ready on this device.',
    };
  }
  return {
    kind: 'neutral',
    label: 'World status',
    detail: playText || 'SwarmCraft is waiting for an authoritative world status.',
  };
}

function byId(id) {
  return document.getElementById(id);
}

function textOf(id, fallback = '') {
  return String(byId(id)?.textContent || fallback).trim();
}

function setText(element, value) {
  if (element && element.textContent !== value) element.textContent = value;
}

function ensureStylesheet() {
  if (document.querySelector('link[data-player-experience]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = new URL('./player-experience.css', import.meta.url).href;
  link.dataset.playerExperience = 'true';
  document.head.append(link);
}

function installAdvancedNavigation() {
  const nav = document.querySelector('.nav-stack');
  const sidebarNode = document.querySelector('.sidebar-node');
  const activity = byId('navActivity');
  const diagnostics = byId('navDiagnostics');
  if (!nav || !sidebarNode || !activity || !diagnostics || byId('advancedNavigation')) return;

  const details = document.createElement('details');
  details.id = 'advancedNavigation';
  details.className = 'advanced-navigation';
  const summary = document.createElement('summary');
  summary.innerHTML = '<span class="nav-glyph" aria-hidden="true">•••</span><span>Tools</span>';
  const body = document.createElement('div');
  body.className = 'advanced-navigation-body';
  activity.classList.add('advanced-nav-item');
  diagnostics.classList.add('advanced-nav-item');
  body.append(activity, diagnostics);
  details.append(summary, body);
  sidebarNode.before(details);
}

function createJourneyOverview(hero) {
  let overview = byId('playerJourneyOverview');
  if (overview) return overview;
  overview = document.createElement('section');
  overview.id = 'playerJourneyOverview';
  overview.className = 'player-journey-overview';
  overview.setAttribute('aria-live', 'polite');
  overview.innerHTML = `
    <div class="journey-heading">
      <div>
        <span class="section-label">Right now</span>
        <h3 id="playerJourneyTitle">Checking world…</h3>
        <p id="playerJourneyDetail">SwarmCraft is checking whether this world is ready to play.</p>
      </div>
      <span id="playerJourneyChip" class="journey-chip neutral">Checking</span>
    </div>
    <div class="journey-facts" aria-label="Player status">
      <div class="journey-fact">
        <span>Connection</span>
        <strong id="playerConnectionSummary">Checking…</strong>
      </div>
      <div class="journey-fact">
        <span>This PC</span>
        <strong id="playerShutdownSummary">Checking…</strong>
      </div>
      <div class="journey-fact">
        <span>World safety</span>
        <strong id="playerSafetySummary">Checking…</strong>
      </div>
    </div>`;
  hero.after(overview);
  return overview;
}

function installAdvancedWorldControls(selectionContent) {
  let details = byId('advancedWorldTools');
  if (details) return details;

  details = document.createElement('details');
  details.id = 'advancedWorldTools';
  details.className = 'advanced-world-tools';
  const summary = document.createElement('summary');
  summary.innerHTML = `
    <span>
      <strong>Advanced world controls</strong>
      <small id="advancedWorldSummary">Replication, hosting, mods and diagnostics</small>
    </span>
    <span class="advanced-chevron" aria-hidden="true">⌄</span>`;
  const body = document.createElement('div');
  body.className = 'advanced-world-body';

  const actions = document.createElement('section');
  actions.className = 'advanced-action-row';
  actions.innerHTML = '<div><h3>Hosting and maintenance</h3><p>Use these when you want to move the host, manage replicas, or inspect technical state.</p></div>';
  const actionButtons = document.createElement('div');
  actionButtons.className = 'compact-actions';
  const transfer = byId('transferHost');
  const transferAvailability = byId('transferAvailability');
  if (transfer) actionButtons.append(transfer);
  actions.append(actionButtons);
  if (transferAvailability) actions.append(transferAvailability);
  body.append(actions);

  const movable = [
    document.querySelector('.world-state-grid'),
    byId('safetyPanel'),
    document.querySelector('.player-section:not(#migrationCard):not(#modsPanel)'),
    byId('migrationCard'),
    byId('modsPanel'),
    document.querySelector('.details-panel'),
    document.querySelector('.danger-zone'),
  ].filter(Boolean);
  for (const node of movable) body.append(node);

  details.append(summary, body);
  selectionContent.append(details);
  return details;
}

function installPrimaryWorldLayout() {
  const selectionContent = byId('selectionContent');
  const hero = selectionContent?.querySelector('.world-hero');
  if (!selectionContent || !hero || selectionContent.dataset.playerExperience === 'true') return false;
  selectionContent.dataset.playerExperience = 'true';

  const heroActions = hero.querySelector('.hero-actions');
  const invite = byId('inviteWorld');
  const play = byId('playWorld');
  if (heroActions && play && invite) {
    heroActions.replaceChildren(play, invite);
  }

  const overview = createJourneyOverview(hero);
  const playAvailability = byId('playAvailability');
  if (playAvailability) overview.append(playAvailability);

  const hostReadiness = byId('hostReadinessPanel');
  if (hostReadiness) overview.after(hostReadiness);
  const notice = byId('worldNotice');
  if (notice && hostReadiness) hostReadiness.after(notice);

  installAdvancedWorldControls(selectionContent);
  return true;
}

function updateWorldExperience() {
  const play = byId('playWorld');
  const state = deriveJourneyState({
    playDisabled: Boolean(play?.disabled),
    playDetail: textOf('playAvailability'),
    hostReadiness: `${textOf('hostReadinessTitle')} ${textOf('hostReadinessDetail')}`,
    migration: `${textOf('migrationBadge')} ${textOf('migrationSummary')}`,
    connectivity: `${textOf('selectedConnectivity')} ${textOf('selectedConnectivityDetail')}`,
  });

  setText(byId('playerJourneyTitle'), state.label);
  setText(byId('playerJourneyDetail'), state.detail);
  const chip = byId('playerJourneyChip');
  if (chip) {
    chip.className = `journey-chip ${state.kind}`;
    setText(chip, state.label);
  }
  setText(byId('playerConnectionSummary'), textOf('selectedConnectivity', 'Checking…'));
  setText(byId('playerShutdownSummary'), textOf('hostReadinessTitle', 'Checking…'));
  setText(byId('playerSafetySummary'), textOf('selectedSafety', 'Checking…'));

  const technicalWarnings = [
    textOf('modsBadge'),
    textOf('selectedSafety'),
    textOf('migrationBadge'),
  ].filter((value) => ACTION_WORDS.test(value)).length;
  setText(
    byId('advancedWorldSummary'),
    technicalWarnings
      ? `${technicalWarnings} item${technicalWarnings === 1 ? '' : 's'} may need attention · replication, hosting, mods and diagnostics`
      : 'Replication, hosting, mods and diagnostics',
  );

  const details = byId('advancedWorldTools');
  if (details) details.classList.toggle('has-attention', technicalWarnings > 0);
}

function relabelPlayerLanguage() {
  setText(byId('openCreate')?.querySelector('span:last-child'), 'New world');
  setText(byId('quickCreate'), 'New world');
  setText(byId('openImport')?.querySelector('span:last-child'), 'Import world');
  setText(byId('seedOn'), 'Keep world available here');
  setText(byId('seedOff'), 'Stop keeping it available');
  setText(byId('verifyWorld'), 'Check saved copy');
  setText(byId('migrationTitle'), 'Hosting');
}

export function installPlayerExperience() {
  if (typeof document === 'undefined') return;
  ensureStylesheet();
  installAdvancedNavigation();
  installPrimaryWorldLayout();
  relabelPlayerLanguage();
  updateWorldExperience();

  if (!document.documentElement.dataset.playerExperienceObserver) {
    document.documentElement.dataset.playerExperienceObserver = 'true';
    let scheduled = false;
    const observer = new MutationObserver(() => {
      if (scheduled) return;
      scheduled = true;
      requestAnimationFrame(() => {
        scheduled = false;
        installPrimaryWorldLayout();
        relabelPlayerLanguage();
        updateWorldExperience();
      });
    });
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: ['hidden', 'disabled', 'class'],
    });
  }
}

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', installPlayerExperience, { once: true });
  } else {
    installPlayerExperience();
  }
}
