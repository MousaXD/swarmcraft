import { writable } from 'svelte/store';

const initial = 'SwarmCraft is starting.';

/**
 * Activity is presentation-only and never holds world authority or signed
 * commands. Both the original app and runtime wizard emit formatted entries.
 */
export const activityText = writable(initial);

export function appendActivity(entry: string): boolean {
  const incoming = String(entry);
  if (!incoming.trim()) return false;
  activityText.update(current => {
    const previous = current.trim();
    if (['SwarmCraft is ready.', 'SwarmCraft is starting.', 'Activity cleared.'].includes(previous)) {
      return incoming;
    }
    return previous ? previous + '\n\n' + incoming : incoming;
  });
  return true;
}

export function clearActivity(): void {
  activityText.set('Activity cleared.');
}

export const activityBridge = Object.freeze({
  append: appendActivity,
  clear: clearActivity,
});
