import { describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { createNativeHostReadinessController, parseNativeHostReadiness } from '../src/lib/host-readiness-state';

describe('Svelte read-only native Rust shutdown readiness', () => {
  it('reports safe shutdown only when native state and native safe_to_shutdown explicitly agree', () => {
    expect(parseNativeHostReadiness({state:'safe',safe_to_shutdown:true})).toMatchObject({
      title:'Safe to shut down this PC',tone:'safe',
    });
    expect(parseNativeHostReadiness({state:'sleeping',safe_to_shutdown:true})).toMatchObject({
      title:'Safe to shut down this PC',tone:'safe',
    });
    for (const response of [
      {state:'safe'}, {state:'sleeping',safe_to_shutdown:false},
      {state:'safe',safeToShutdown:null}, {state:'safe',safe_to_shutdown:'true'},
      {state:'safe',safe_to_shutdown:false,safeToShutdown:true},
    ]) {
      expect(parseNativeHostReadiness(response).tone).not.toBe('safe');
    }
  });
  it('keeps all non-safe Rust states visible without inventing automatic takeover', () => {
    for (const state of [
      'world_will_stop','syncing','blocked_by_quorum','blocked_by_mods',
      'blocked_by_runtime','degraded_safety','not_current_host','conflict',
    ]) {
      const view = parseNativeHostReadiness({state,safe_to_shutdown:false});
      expect(view.tone).not.toBe('safe');
      expect(view.title).not.toBe('Safe to shut down this PC');
    }
    expect(parseNativeHostReadiness({state:'conflict'}).tone).toBe('danger');
    expect(parseNativeHostReadiness({state:'__proto__'}).tone).toBe('warning');
    expect(parseNativeHostReadiness({state:'future_state'}).tone).toBe('warning');
  });
  it('never renders sensitive peer addresses, errors, or arbitrary Rust detail text', () => {
    const result = parseNativeHostReadiness({
      state:'blocked_by_quorum',
      detail:'private identity scinvite:token /home/user/.keys',
      peers:[{peer_id:'sensitive-peer'}],
      successor_peer_id:'private-successor',
    });
    expect(JSON.stringify(result)).not.toMatch(/scinvite:|\/home\/user|sensitive-peer|private-successor/);
    expect(result.title).toBe('World will go offline');
    expect(() => parseNativeHostReadiness('not-json scinvite:secret')).toThrow();
  });
  it('guards against older status completion after switching to another world', async () => {
    let resolveOld!: (value: unknown) => void;
    const read = vi.fn((world: string) => world === 'scworld:old'
      ? new Promise<unknown>(r => {resolveOld=r;})
      : Promise.resolve(JSON.stringify({state:'world_will_stop',safe_to_shutdown:false})));
    const controller = createNativeHostReadinessController(read);
    const old = controller.select('scworld:old');
    await Promise.resolve();
    await controller.select('scworld:new');
    resolveOld({state:'safe',safe_to_shutdown:true});
    await old;
    expect(get(controller.state)).toMatchObject({
      world:'scworld:new', title:'World will go offline', tone:'warning',
      verifiedReadCount:1,
    });
  });
  it('clears readiness immediately when selected world is removed', async () => {
    const controller = createNativeHostReadinessController(
      async () => ({state:'safe',safe_to_shutdown:true}),
    );
    await controller.select('scworld:abc');
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
          : Promise.resolve({state:'blocked_by_quorum',safe_to_shutdown:false});
      },25);
      const pending=controller.select('scworld:abc');
      await vi.advanceTimersByTimeAsync(26);
      await pending;
      expect(get(controller.state)).toMatchObject({
        phase:'unavailable',tone:'warning',busy:false,verifiedReadCount:0,
      });
      resolveOld({state:'safe',safe_to_shutdown:true});
      await Promise.resolve();
      expect(get(controller.state).tone).toBe('warning');
      await controller.select('scworld:abc');
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
    const first=controller.select('scworld:abc');
    const second=controller.select('scworld:abc');
    expect(first).toBe(second);
    await Promise.resolve();
    expect(fn).toHaveBeenCalledTimes(1);
    resolve({state:'not_current_host'});
    await first;
    expect(get(controller.state)).toMatchObject({phase:'ready',verifiedReadCount:1});
  });
});
