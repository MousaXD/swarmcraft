import type { WorldSummary } from './world-list';

/** Read the existing human-readable Rust CLI output without inventing fields. */
export function parseStatusLines(text: string): Readonly<Record<string, string>> {
  const values: Record<string, string> = Object.create(null);
  for (const line of String(text ?? '').split('\n')) {
    const index = line.indexOf(':');
    if (index <= 0) continue;
    const key = line.slice(0, index).trim();
    if (!key) continue;
    values[key] = line.slice(index + 1).trim();
  }
  return values;
}

export type WorldSafety = 'canonical' | 'degraded' | 'conflict' | 'unknown';
export interface WorldModel extends WorldSummary {
  readonly status: Readonly<Record<string, string>>;
  readonly compatibility: Readonly<Record<string, string>>;
  readonly statusAvailable: boolean;
  readonly compatibilityAvailable: boolean;
}

export function worldSafety(status: Readonly<Record<string, string>>): WorldSafety {
  const safety = String(status.Safety ?? '').trim().toLowerCase();
  if (safety.includes('conflict')) return 'conflict';
  if (safety.includes('solo') || safety.includes('degraded')) return 'degraded';
  if (safety.includes('canonical') || safety.includes('quorum')) return 'canonical';
  return 'unknown';
}

/** UI availability hint only; Rust makes the final authority admission decision. */
export function playAvailability(world: WorldModel | null):
  Readonly<{ enabled: boolean; reason: string; safety: WorldSafety }> {
  if (!world) return { enabled: false, reason: 'Choose a world first.', safety: 'unknown' };
  const safety = worldSafety(world.status);
  if (safety === 'conflict') {
    return {
      enabled: false, safety,
      reason: 'Play is unavailable while divergent history needs review.',
    };
  }
  if (!world.statusAvailable || !world.compatibilityAvailable) {
    return {
      enabled: false, safety: 'unknown',
      reason: 'World status or compatibility is unavailable. Refresh before attempting to host.',
    };
  }
  const eligibility = String(world.compatibility['Authority eligibility'] ?? '').trim();
  if (eligibility === 'Compatible') {
    return { enabled: true, safety, reason: 'This device is eligible to request hosting; the Rust backend performs final admission.' };
  }
  if (eligibility.toLowerCase().includes('not authority eligible') || eligibility.toLowerCase().startsWith('replica only')) {
    return { enabled: false, safety, reason: 'This device can keep a replica but cannot host the world.' };
  }
  return { enabled: false, safety, reason: 'Play is unavailable until authority eligibility is verified.' };
}

export function worldModel(
  summary: WorldSummary,
  results: readonly [PromiseSettledResult<string>, PromiseSettledResult<string>],
): WorldModel {
  const [status, compatibility] = results;
  return {
    ...summary,
    status: status.status === 'fulfilled' ? parseStatusLines(status.value) : {},
    compatibility: compatibility.status === 'fulfilled' ? parseStatusLines(compatibility.value) : {},
    statusAvailable: status.status === 'fulfilled' && !!String(status.value).trim(),
    compatibilityAvailable: compatibility.status === 'fulfilled' && !!String(compatibility.value).trim(),
  };
}
