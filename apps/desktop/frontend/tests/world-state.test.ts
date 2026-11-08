import { describe, expect, it } from 'vitest';
import { parseStatusLines, worldModel, worldSafety, playAvailability } from '../src/lib/world-state';
import type { WorldSummary } from '../src/lib/world-list';

const base: WorldSummary = { id: 'scworld:abc', name: 'Demo', snapshots: 2 };
const fulfilled = (value: string): PromiseFulfilledResult<string> => ({ status: 'fulfilled', value });
const rejected = (): PromiseRejectedResult => ({ status: 'rejected', reason: new Error('backend unavailable') });

describe('Rust status presentation is fail-closed', () => {
  it('parses first colon only, without object-prototype injection', () => {
    const data = parseStatusLines('Safety: Canonical\nAuthority eligibility: Replica only\nNetwork: /ip4/127.0.0.1/tcp/444\n__proto__: forged');
    expect(data.Safety).toBe('Canonical');
    expect(data.Network).toBe('/ip4/127.0.0.1/tcp/444');
    expect(Object.getPrototypeOf(data)).toBe(null);
    expect(Object.prototype).not.toHaveProperty('forged');
  });
  it('never presents conflict as safe when status contains both words', () => {
    expect(worldSafety({ Safety: 'canonical with conflict' })).toBe('conflict');
    expect(worldSafety({ Safety: 'solo' })).toBe('degraded');
    expect(worldSafety({ Safety: 'canonical' })).toBe('canonical');
    expect(worldSafety({ Safety: '' })).toBe('unknown');
  });
  it('does not grant play from missing or failed Rust status/compatibility', () => {
    const unavailable = worldModel(base, [rejected(), fulfilled('Authority eligibility: Compatible')]);
    expect(playAvailability(unavailable)).toMatchObject({ enabled: false, safety: 'unknown' });
    const missing = worldModel(base, [fulfilled('Safety: canonical'), rejected()]);
    expect(playAvailability(missing).enabled).toBe(false);
  });
  it('disables Play on divergent canonical history even if compatibility says Compatible', () => {
    const model = worldModel(base, [fulfilled('Safety: conflict'), fulfilled('Authority eligibility: Compatible')]);
    expect(playAvailability(model)).toMatchObject({ enabled: false, safety: 'conflict' });
  });
  it('distinguishes replica-only and degraded/solo from canonical safety', () => {
    const replica = worldModel(base, [fulfilled('Safety: solo'), fulfilled('Authority eligibility: Replica only')]);
    expect(playAvailability(replica)).toMatchObject({ enabled: false, safety: 'degraded' });
    const compatible = worldModel(base, [fulfilled('Safety: canonical'), fulfilled('Authority eligibility: Compatible')]);
    expect(playAvailability(compatible)).toMatchObject({ enabled: true, safety: 'canonical' });
  });
  it('never treats unknown authority eligibility as verified', () => {
    const model = worldModel(base, [fulfilled('Safety: canonical'), fulfilled('Authority eligibility: Maybe')]);
    expect(playAvailability(model).enabled).toBe(false);
    expect(playAvailability(null).enabled).toBe(false);
  });
});

it('does not enable Play when compatibility is verified but safety is unknown', () => {
  const model = worldModel(
    { id: 'scworld:unknown', name: 'Unknown', snapshots: 1 },
    [
      { status: 'fulfilled', value: 'Members: 2' },
      { status: 'fulfilled', value: 'Authority eligibility: Compatible' },
    ],
  );
  expect(playAvailability(model)).toMatchObject({ enabled: false, safety: 'unknown' });
});
