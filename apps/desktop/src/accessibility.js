// Shared by the shipping and candidate frontends. These helpers govern
// focus/presentation only; native commands remain the authority for actions.
export function focusAvailable(element) {
  if (!element?.isConnected || element.disabled || element.closest('[hidden], [inert]')) return false;
  if (!element.getClientRects().length || getComputedStyle(element).visibility !== 'visible') return false;
  const modal = document.querySelector('dialog[open]');
  if (modal && !modal.contains(element)) return false;
  element.focus();
  return document.activeElement === element;
}

export function focusCurrentView() {
  const panel = document.querySelector('[data-view-panel]:not([hidden])');
  const heading = panel?.querySelector('h2') || document.getElementById('viewTitle');
  if (!heading) return false;
  heading.tabIndex = -1;
  return focusAvailable(heading);
}

export function actionPending(control) {
  return control?.dataset.busy === 'true' || control?.getAttribute('aria-busy') === 'true';
}

const openers = new WeakMap();
const registered = new WeakSet();

export function openTaskDialog(dialog, opener, initial = null) {
  if (!dialog || dialog.open) return false;
  if (!registered.has(dialog)) {
    registered.add(dialog);
    dialog.addEventListener('close', () => {
      if (dialog.open) return; // A queued close must not affect a reopened task.
      const trigger = openers.get(dialog);
      openers.delete(dialog);
      // Never steal focus from a subsequent task or from a different modal.
      if (document.querySelector('dialog[open]')) return;
      const active = document.activeElement;
      if (active && active !== document.body && active !== trigger && !dialog.contains(active)
          && active.getClientRects().length) return;
      if (!focusAvailable(trigger)) focusCurrentView();
    });
  }
  openers.set(dialog, opener || document.activeElement);
  dialog.showModal();
  focusAvailable(initial);
  return true;
}

// During a non-cancellable native operation, Escape and form submission
// cannot dismiss the task. Keep the reason visible and announced even if
// its initiating button becomes disabled. This is NOT cancellation of IPC.
export function guardDialogDismissal(dialog, pending, report) {
  const guard = event => {
    if (!pending()) return;
    event.preventDefault();
    report?.();
  };
  dialog.addEventListener('cancel', guard);
  dialog.querySelector('form[method="dialog"]')?.addEventListener('submit', guard);
}
