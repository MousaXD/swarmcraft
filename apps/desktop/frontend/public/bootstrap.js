// Runs as a classic script before the application module. If the module never
// evaluates (including a failed import), the launcher must not stay on skeletons.
(() => {
  const STARTUP_WATCHDOG_MS = 60000;
  let watchdog;
  let complete = false;
  let failed = false;
  let phase = 'Loading desktop JavaScript';
  const errors = [];

  const byId = (id) => document.getElementById(id);
  const safeMessage = (error) => {
    const value = String(error?.message || error || 'Unknown error');
    // Diagnostics must not echo credentials, signed invites, or secret arguments.
    if (/authorization|bearer|private.?key|password|ipc.?token|invite.?token|secret/i.test(value)) {
      return 'A command failed. See the sanitized native startup diagnostics.';
    }
    return value.slice(0, 320);
  };

  function setPhase(next) {
    phase = next;
    document.documentElement.dataset.swarmcraftStartupPhase = next;
  }

  function stopWatchdog() {
    if (watchdog) clearTimeout(watchdog);
    watchdog = undefined;
  }

  function fail(kind, error) {
    stopWatchdog();
    complete = false;
    failed = true;
    const message = safeMessage(error);
    errors.push({ kind, phase, message });
    document.documentElement.dataset.swarmcraftStartup = 'failed';
    const status = byId('status');
    if (status) status.textContent = 'Startup failed';
    const dot = byId('statusDot');
    if (dot) dot.className = 'status-dot danger';
    const node = byId('nodeBadge');
    if (node && node.textContent === 'Checking…') node.textContent = 'Setup needs attention';
    const count = byId('worldCount');
    if (count && count.textContent === 'Loading…') count.textContent = 'Worlds unavailable';
    const cards = byId('worldCards');
    if (cards && cards.getAttribute('aria-busy') === 'true') {
      cards.setAttribute('aria-busy', 'false');
      cards.replaceChildren();
    }
    let banner = byId('startupFailure');
    if (!banner) {
      banner = document.createElement('div');
      banner.id = 'startupFailure';
      banner.className = 'page-notice notice-danger';
      banner.setAttribute('role', 'alert');
      const header = document.querySelector('.topbar');
      header?.insertAdjacentElement('afterend', banner);
      if (!header) document.body.prepend(banner);
    }
    banner.replaceChildren();
    const explanation = document.createElement('span');
    explanation.textContent = `SwarmCraft could not start (${kind}, ${phase}). ${message}`;
    const retry = document.createElement('button');
    retry.id = 'retryStartup';
    retry.type = 'button';
    retry.className = 'button button-secondary button-small';
    retry.textContent = 'Retry startup';
    retry.addEventListener('click', () => {
      if (typeof window.swarmcraftRetryStartup === 'function') {
        window.swarmcraftRetryStartup();
      } else {
        window.location.reload();
      }
    });
    banner.append(explanation, document.createTextNode(' '), retry);
    banner.hidden = false;
  }

  const api = {
    get errors() { return errors.slice(); },
    get phase() { return phase; },
    get failed() { return failed; },
    begin() {
      stopWatchdog();
      failed = false;
      complete = false;
      errors.length = 0;
      document.documentElement.dataset.swarmcraftStartup = 'starting';
      byId('startupFailure')?.remove();
      setPhase('Setting up device');
      watchdog = setTimeout(() => fail('timeout', 'Startup exceeded 60 seconds. Retry or restart SwarmCraft.'), STARTUP_WATCHDOG_MS);
    },
    step: setPhase,
    ready({ degraded = false } = {}) {
      if (failed) return;
      stopWatchdog();
      complete = true;
      document.documentElement.dataset.swarmcraftStartup = degraded ? 'degraded' : 'ready';
      setPhase(degraded ? 'Networking unavailable' : 'Ready');
    },
    fail,
  };

  window.__swarmcraftStartup = api;
  document.documentElement.dataset.swarmcraftStartup = 'loading';
  window.addEventListener('error', (event) => {
    if (!complete) fail('javascript', 'A JavaScript module failed to load or run. Check the installation and retry.');
    else fail('javascript', 'An unexpected application error occurred.');
  });
  window.addEventListener('unhandledrejection', () => {
    fail('javascript', 'An application operation failed unexpectedly. Retry or check diagnostics.');
  });
  // Start this timer *before* the ES module graph is evaluated. A blocked
  // module can prevent DOMContentLoaded from firing at all, so waiting for
  // that event would recreate the original infinite Loading / Checking state.
  watchdog = setTimeout(
    () => fail('module-load', 'The desktop JavaScript module did not initialize. Check the installation and retry.'),
    12000,
  );
  const moduleScript = document.querySelector('script[type="module"][src$="app.js"]');
  moduleScript?.addEventListener('error', () => {
    fail('module-load', 'The desktop JavaScript module or one of its dependencies could not be loaded.');
  });
  document.addEventListener('DOMContentLoaded', () => {
    // Keep the early timer active until app.js calls begin(), or until the
    // explicit ready/failure transition stops it.
  }, { once: true });
})();
