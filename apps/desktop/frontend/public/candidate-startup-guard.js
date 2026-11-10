// Candidate-only companion to PR #88's byte-for-byte preserved bootstrap.js.
// No Svelte import or native IPC is required for this code to execute.
(() => {
  const byId = (id) => document.getElementById(id);

  function showFallbackFailure() {
    if (document.documentElement.dataset.swarmcraftStartup !== 'failed') return;
    // When Svelte has mounted, the original #status exists; leave it alone.
    if (byId('status')) return;
    const fallback = byId('bootstrapFallback');
    if (!fallback) return;
    const status = document.createElement('span');
    status.id = 'status';
    status.textContent = 'Startup failed';
    fallback.replaceChildren(status);
    fallback.setAttribute('role', 'alert');
    fallback.setAttribute('aria-live', 'assertive');
    fallback.hidden = false;
  }

  // The original watchdog may fail due to a timeout instead of a module
  // onerror. Watch that exact fail-closed state to render a native-testable
  // status even when the Svelte component graph cannot create its own DOM.
  const observer = new MutationObserver(showFallbackFailure);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-swarmcraft-startup'],
  });

  // Vite emits hashed /assets/index-*.js, not app.js. The original bootstrap
  // intentionally retains its old app.js selector for the shipping legacy UI.
  const moduleScript = document.querySelector('script[type="module"]');
  moduleScript?.addEventListener('error', () => {
    const startup = window.__swarmcraftStartup;
    if (startup && !startup.failed) {
      startup.fail('module-load', 'The desktop module could not be loaded. Check the installation and retry.');
    }
    showFallbackFailure();
  });
  showFallbackFailure();
})();
