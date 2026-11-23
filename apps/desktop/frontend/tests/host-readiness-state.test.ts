import { describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { createNativeHostReadinessController, parseNativeHostReadiness } from '../src/lib/host-readiness-state';

const WORLD = `scworld:${'a'.repeat(64)}`;
const OTHER = `scworld:${'b'.repeat(64)}`;
const fresh = (body: Record<string, unknown>, world = WORLD) => ({
  world_id: world, generated_unix_ms: Date.now() - 100, ...body,
});

describe('Svelte read-only native Rust shutdown readiness', () => {
  it('reports safe shutdown only when native state and native safe_to_shutdown explicitly agree', () => {
    expect(parseNativeHostReadiness(fresh({state:'safe',safe_to_shutdown:true}), WORLD)).toMatchObject({
      title:'Safe to shut down this PC',tone:'safe',
    });
    expect(parseNativeHostReadiness(fresh({state:'sleeping',safe_to_shutdown:true}), WORLD)).toMatchObject({
      title:'Safe to shut down this PC',tone:'safe',
    });
    for (const response of [
      {state:'safe'}, {state:'sleeping',safe_to_shutdown:false},
      {state:'safe',safeToShutdown:null}, {state:'safe',safe_to_shutdown:'true'},
      {state:'safe',safe_to_shutdown:false,safeToShutdown:true},
    ]) {
      expect(parseNativeHostReadiness(fresh(response), WORLD).tone).not.toBe('safe');
    }
  });
  it('keeps all non-safe Rust states visible without inventing automatic takeover', () => {
    for (const state of [
      'world_will_stop','syncing','blocked_by_quorum','blocked_by_mods',
      'blocked_by_runtime','degraded_safety','not_current_host','conflict',
    ]) {
      const view = parseNativeHostReadiness(fresh({state,safe_to_shutdown:false}), WORLD);
      expect(view.tone).not.toBe('safe');
      expect(view.title).not.toBe('Safe to shut down this PC');
    }
    expect(parseNativeHostReadiness(fresh({state:'conflict'}), WORLD).tone).toBe('danger');
    expect(parseNativeHostReadiness(fresh({state:'__proto__'}), WORLD).tone).toBe('warning');
    expect(parseNativeHostReadiness(fresh({state:'future_state'}), WORLD).tone).toBe('warning');
  });
  it('never renders sensitive peer addresses, errors, or arbitrary Rust detail text', () => {
    const result = parseNativeHostReadiness(fresh({
      state:'blocked_by_quorum',
      detail:'private identity scinvite:token /home/user/.keys',
      peers:[{peer_id:'sensitive-peer'}],
      successor_peer_id:'private-successor',
    }), WORLD);
    expect(JSON.stringify(result)).not.toMatch(/scinvite:|\/home\/user|sensitive-peer|private-successor/);
    expect(result.title).toBe('World will go offline');
    expect(() => parseNativeHostReadiness('not-json scinvite:secret', WORLD)).toThrow();
  });
  it('requires exact world identity and a live six-second Rust proof for any safe state', () => {
    const now = Date.now();
    const base = fresh({state:'safe', safe_to_shutdown:true});
    expect(parseNativeHostReadiness(base, WORLD, now).tone).toBe('safe');
    for (const invalid of [
      {...base,world_id:OTHER}, {...base,world_id:undefined},
      {...base,generated_unix_ms:now-6001},
      {...base,generated_unix_ms:now+3000},
      {...base,generated_unix_ms:'not-a-time'},
      {...base,safe_to_shutdown:false},
    ]) {
      expect(parseNativeHostReadiness(invalid, WORLD, now).tone).not.toBe('safe');
    }
    expect(parseNativeHostReadiness(base, OTHER, now).tone).not.toBe('safe');
  });
  it('guards against older status completion after switching to another world', async () => {
    let resolveOld!: (value: unknown) => void;
    const read = vi.fn((world: string) => world === WORLD
      ? new Promise<unknown>(r => {resolveOld=r;})
      : Promise.resolve(JSON.stringify(fresh({state:'world_will_stop',safe_to_shutdown:false}, OTHER))));
    const controller = createNativeHostReadinessController(read);
    const old = controller.select(WORLD);
    await Promise.resolve();
    await controller.select(OTHER);
    resolveOld(fresh({state:'safe',safe_to_shutdown:true}));
    await old;
    expect(get(controller.state)).toMatchObject({
      world:OTHER, title:'World will go offline', tone:'warning',
      verifiedReadCount:1,
    });
  });
  it('clears readiness immediately when selected world is removed', async () => {
    const controller = createNativeHostReadinessController(
      async () => fresh({state:'safe',safe_to_shutdown:true}),
    );
    await controller.select(WORLD);
    expect(get(controller.state).tone).toBe('safe');
    controller.clear();
    expect(get(controller.state)).toMatchObject({
      world:'',phase:'unchecked',tone:'neutral',busy:false,
    });
    await controller.select('invalid');
    expect(get(controller.state).world).toBe('');
  });
  it('bounds hung Rust IPC, permits retry, and ignores late contradictory success', async () => {
    vi.useFakeTimers();
    try {
      let resolveOld!: (value:unknown) => void;
      let calls=0;
      const controller=createNativeHostReadinessController(() => {
        calls+=1;
        return calls===1 ? new Promise<unknown>(r=>{resolveOld=r;})
          : Promise.resolve(fresh({state:'blocked_by_quorum',safe_to_shutdown:false}));
      },25);
      const pending=controller.select(WORLD);
      await vi.advanceTimersByTimeAsync(26);
      await pending;
      expect(get(controller.state)).toMatchObject({
        phase:'unavailable',tone:'warning',busy:false,verifiedReadCount:0,
      });
      resolveOld(fresh({state:'safe',safe_to_shutdown:true}));
      await Promise.resolve();
      expect(get(controller.state).tone).toBe('warning');
      await controller.select(WORLD);
      expect(get(controller.state)).toMatchObject({
        phase:'ready',title:'World will go offline',verifiedReadCount:1,tone:'warning',
      });
    } finally {
      vi.useRealTimers();
    }
  });
  it('coalesces duplicate reads of the same selected world while busy', async () => {
    let resolve!: (value:unknown) => void;
    const fn=vi.fn(() => new Promise<unknown>(r=>{resolve=r;}));
    const controller=createNativeHostReadinessController(fn);
    const first=controller.select(WORLD);
    const second=controller.select(WORLD);
    expect(first).toBe(second);
    await Promise.resolve();
    expect(fn).toHaveBeenCalledTimes(1);
    resolve(fresh({state:'not_current_host'}));
    await first;
    expect(get(controller.state)).toMatchObject({phase:'ready',verifiedReadCount:1});
  });
  it('retracts an authentic safe report when its freshness window expires', async () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date('2026-11-23T12:00:00.000Z'));
      const controller = createNativeHostReadinessController(
        async () => fresh({state:'safe',safe_to_shutdown:true}),
      );
      await controller.select(WORLD);
      expect(get(controller.state)).toMatchObject({phase:'ready',tone:'safe'});
      await vi.advanceTimersByTimeAsync(5901);
      expect(get(controller.state)).toMatchObject({
        phase:'unavailable',tone:'warning',title:'Shutdown safety expired',
      });
      controller.clear();
      expect(get(controller.state).tone).toBe('neutral');
    } finally {
      vi.useRealTimers();
    }
  });
});
