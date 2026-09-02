export const DURABLE_STOP_COPY = 'SwarmCraft requests a Minecraft save barrier, publishes the canonical checkpoint, and waits for durable sleeping state before Stop succeeds. Background replica storage can continue separately.';

export function applyPlayerCopy(documentRef = globalThis.document) {
  const stopCopy = documentRef?.querySelector?.('#sleepDialog .dialog-card > div:first-child p');
  if (stopCopy) stopCopy.textContent = DURABLE_STOP_COPY;
  return Boolean(stopCopy);
}

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => applyPlayerCopy(), { once: true });
  else applyPlayerCopy();
}
