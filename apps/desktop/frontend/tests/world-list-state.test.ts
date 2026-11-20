import { describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import {
  createNativeWorldListController, parseNativeWorldList, worldSafetyTone, nativeWorldSafetySummary,
  type NativeWorldListBackend,
} from '../src/lib/world-list-state';

const raw = 'scworld:abc Friends SMP snapshots=2\nscworld:def Another save snapshots=0\n';
function native(overrides: Partial<NativeWorldListBackend> = {}): NativeWorldListBackend {
  return {
    listWorlds: vi.fn(async () => raw),
    worldStatus: vi.fn(async () => 'Safety: canonical\nMinecraft: 26.1.2\nAuthority: member-one'),
    worldCompatibility: vi.fn(async () => 'Authority eligibility: Replica only'),
    ...overrides,
  };
}
describe('Svelte world browsing and native list contracts', () => {
  it('parses actual CLI line grammar, including multiword names and genuinely empty lists', () => {
    expect(parseNativeWorldList(raw).map(({ id, name, snapshots }) => ({ id, name, snapshots }))).toEqual([
      { id: 'scworld:abc', name: 'Friends SMP', snapshots: 2 },
      { id: 'scworld:def', name: 'Another save', snapshots: 0 },
    ]);
    expect(parseNativeWorldList('No local worlds yet\n')).toEqual([]);
    expect(parseNativeWorldList('')).toEqual([]);
  });
  it('fails closed on duplicated, malformed, non-text and overflowed native records', () => {
    expect(() => parseNativeWorldList('scworld:abc malformed')).toThrow(/malformed/);
    expect(() => parseNativeWorldList('scworld:abc Alpha snapshots=2\nscworld:abc Beta snapshots=3')).toThrow(/validation/);
    expect(() => parseNativeWorldList('scworld:abc Test snapshots=99999999999999999999999')).toThrow(/validation/);
    expect(() => parseNativeWorldList({worlds:[]})).toThrow(/not text/);
    expect(() => parseNativeWorldList(Array.from({length:257}, (_, i) =>
      `scworld:id${i} World ${i} snapshots=0`).join('\n'))).toThrow(/bounded/);
  });
  it('reads actual world status and compatibility in parallel and leaves host consent to Rust', async () => {
    const backend = native();
    const controller = createNativeWorldListController(backend);
    const worlds = await controller.refresh();
    expect(worlds).toHaveLength(2);
    expect(backend.listWorlds).toHaveBeenCalledTimes(1);
    expect(backend.worldStatus).toHaveBeenCalledWith('scworld:abc');
    expect(backend.worldCompatibility).toHaveBeenCalledWith('scworld:def');
    expect(get(controller.state)).toMatchObject({phase:'ready',busy:false,verifiedReadCount:1});
    expect(worlds[0].status.Safety).toBe('canonical');
    expect(worlds[0].compatibility['Authority eligibility']).toBe('Replica only');
    expect(Object.keys(worlds[0])).not.toContain('approvedHosting');
  });
  it('preserves partial/offline replicas and rejects prototype property poisoning', async () => {
    const controller = createNativeWorldListController(native({
      worldStatus: async () => { throw new Error('private /home/user/secret sidecar path'); },
      worldCompatibility: async () => '__proto__: admin\nconstructor: spoof\nAuthority eligibility: Replica only',
    }));
    const worlds = await controller.refresh();
    expect(worlds[0].status.Safety).toBeUndefined();
    expect(worlds[0].compatibility['Authority eligibility']).toBe('Replica only');
    expect(worlds[0].compatibility.__proto__).toBeUndefined();
    expect(Object.prototype.hasOwnProperty.call(worlds[0].compatibility, 'constructor')).toBe(false);
    expect(JSON.stringify(get(controller.state))).not.toMatch(/private|\/home\/user\/secret/);
  });
  it('coalesces duplicate native reads and enables a new refresh after completion', async () => {
    let complete!: (value:string) => void;
    let reads = 0;
    const backend = native({listWorlds: () => {
      reads += 1;
      return reads === 1 ? new Promise<string>(r => { complete = r; }) : Promise.resolve('');
    }});
    const controller = createNativeWorldListController(backend);
    const first = controller.refresh();
    const second = controller.refresh();
    expect(first).toBe(second);
    await Promise.resolve();
    expect(reads).toBe(1);
    complete(raw);
    await first;
    expect(get(controller.state).verifiedReadCount).toBe(1);
    await controller.refresh();
    expect(reads).toBe(2);
    expect(get(controller.state)).toMatchObject({phase:'ready',worlds:[],verifiedReadCount:2});
  });
  it('bounds a hung native list, reports redacted failure and allows retry', async () => {
    vi.useFakeTimers();
    try {
      let calls = 0;
      const controller = createNativeWorldListController(native({
        listWorlds: () => {
          calls += 1;
          return calls === 1 ? new Promise<string>(() => {}) : Promise.resolve(raw);
        },
      }), 25);
      const pending = controller.refresh();
      await vi.advanceTimersByTimeAsync(26);
      await expect(pending).rejects.toThrow(/Could not read local worlds/);
      expect(get(controller.state)).toMatchObject({phase:'unavailable',busy:false,worlds:[]});
      await controller.refresh();
      expect(calls).toBe(2);
      expect(get(controller.state)).toMatchObject({phase:'ready',verifiedReadCount:1});
    } finally {
      vi.useRealTimers();
    }
  });
  it('bounded metadata failures never discard an actually returned local world', async () => {
    vi.useFakeTimers();
    try {
      const controller = createNativeWorldListController(native({
        listWorlds: async () => 'scworld:real Existing save snapshots=1',
        worldStatus: () => new Promise<string>(() => {}),
      }), 20);
      const result = controller.refresh();
      await vi.advanceTimersByTimeAsync(22);
      const worlds = await result;
      expect(worlds).toHaveLength(1);
      expect(worlds[0].name).toBe('Existing save');
      expect(worlds[0].status.Safety).toBeUndefined();
      expect(get(controller.state)).toMatchObject({phase:'ready',busy:false,verifiedReadCount:1});
    } finally {
      vi.useRealTimers();
    }
  });
  it('caps overall refresh and ignores late queued metadata so startup never stays Loading', async () => {
    vi.useFakeTimers();
    try {
      let started = 0;
      const controller = createNativeWorldListController(native({
        listWorlds: async () => Array.from({length: 12}, (_, i) =>
          `scworld:id${i} World ${i} snapshots=0`).join('\n'),
        worldStatus: () => { started += 1; return new Promise<string>(() => {}); },
      }), 30);
      const task = controller.refresh();
      // At most four different world statuses should be launched at once.
      await vi.advanceTimersByTimeAsync(1);
      expect(started).toBe(4);
      await vi.advanceTimersByTimeAsync(61);
      await expect(task).rejects.toThrow(/Could not read local worlds/);
      expect(get(controller.state)).toMatchObject({
        phase: 'unavailable', busy: false, verifiedReadCount: 0, worlds: [],
      });
      const atTimeout = started;
      await vi.advanceTimersByTimeAsync(100);
      expect(started).toBe(atTimeout);
      expect(get(controller.state)).toMatchObject({
        phase: 'unavailable', busy: false, verifiedReadCount: 0,
      });
    } finally {
      vi.useRealTimers();
    }
  });
  it('renders only source-reported read-only world safety and never implies admission', () => {
    expect(nativeWorldSafetySummary('history conflict')).toMatch(/Divergent history/);
    expect(nativeWorldSafetySummary('solo degraded')).toMatch(/reduced safety guarantees/);
    expect(nativeWorldSafetySummary('canonical quorum')).toMatch(/Canonical world history/);
    expect(nativeWorldSafetySummary('')).toMatch(/not available yet/);
    for (const safety of ['', 'Solo', 'Canonical', 'Conflict']) {
      expect(nativeWorldSafetySummary(safety)).not.toMatch(/host approved|ready to play|admitted/i);
    }
  });
  it('maps safety only from Rust fields; does not treat unknown as safe', () => {
    expect(worldSafetyTone('canonical quorum')).toBe('safe');
    expect(worldSafetyTone('history conflict')).toBe('danger');
    expect(worldSafetyTone('solo degraded')).toBe('warning');
    expect(worldSafetyTone('')).toBe('neutral');
  });
});
