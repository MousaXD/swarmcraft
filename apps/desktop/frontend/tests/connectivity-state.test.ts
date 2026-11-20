import { describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { createNativeConnectivityController, parseNativeConnectivity } from '../src/lib/connectivity-state';

describe('Svelte native connectivity contract', () => {
  it('uses only approved Rust network-state labels and never claims hosting authority', () => {
    for (const state of ['direct_reachable', 'hole_punched', 'relay_connected']) {
      const view = parseNativeConnectivity(JSON.stringify({
        state,
        last_failure: 'bearer secret scinvite:token',
        local_addresses: ['/ip4/10.0.0.10/tcp/1234'],
      }));
      expect(view.tone).toBe('safe');
      expect(JSON.stringify(view)).not.toMatch(/bearer|scinvite|10\.0\.0\.10|authori[sz]ed/i);
      expect(view.detail).toMatch(/not hosting|not authority|does not grant/i);
    }
  });
  it('preserves explicit negative and unknown states without implying network readiness', () => {
    expect(parseNativeConnectivity({ state: 'no_viable_path' })).toMatchObject({ tone: 'danger', label: 'No viable peer path' });
    expect(parseNativeConnectivity({ connectivity_state: 'nat_status_unknown' })).toMatchObject({
      tone: 'warning', label: 'Network path not verified',
    });
    expect(parseNativeConnectivity({ state: 'future_new_state', error: 'private-key-secret' })).toMatchObject({
      tone: 'warning', label: 'Unknown native network state',
    });
    expect(JSON.stringify(parseNativeConnectivity({ state: 'future', error: 'private-key-secret' }))).not.toContain('private-key-secret');
  });
  it('rejects a malformed native payload and exposes a generic, retryable error', async () => {
    const read = vi.fn(async () => 'not-json /private/path scinvite:secret');
    const controller = createNativeConnectivityController(read);
    await controller.refresh();
    expect(get(controller.state)).toMatchObject({
      phase: 'unavailable', busy: false, verifiedReadCount: 0,
    });
    expect(JSON.stringify(get(controller.state))).not.toMatch(/not-json|private\/path|scinvite:/);
    expect(read).toHaveBeenCalledTimes(1);
  });
  it('coalesces duplicate real Rust reads and allows explicit new refresh', async () => {
    let resolve!: (value: unknown) => void;
    let calls = 0;
    const read = vi.fn(() => {
      calls += 1;
      return calls === 1
        ? new Promise<unknown>(r => { resolve = r; })
        : Promise.resolve({state:'relay_required'});
    });
    const controller = createNativeConnectivityController(read);
    const a = controller.refresh();
    const b = controller.refresh();
    await Promise.resolve();
    expect(a).toBe(b);
    expect(read).toHaveBeenCalledTimes(1);
    expect(get(controller.state).phase).toBe('checking');
    resolve({state:'relay_required'});
    await a;
    expect(get(controller.state)).toMatchObject({
      phase: 'checked', label: 'Relay required', busy: false, verifiedReadCount: 1,
    });
    await controller.refresh();
    expect(read).toHaveBeenCalledTimes(2);
    expect(get(controller.state).verifiedReadCount).toBe(2);
  });
  it('times out a truly hung IPC, ignores late success, and retries safely', async () => {
    vi.useFakeTimers();
    try {
      let resolveOld!: (value: unknown) => void;
      let count = 0;
      const controller = createNativeConnectivityController(() => {
        count += 1;
        if (count === 1) return new Promise<unknown>(r => { resolveOld = r; });
        return Promise.resolve({state:'direct_reachable'});
      }, 40);
      const stalled = controller.refresh();
      await vi.advanceTimersByTimeAsync(41);
      await stalled;
      expect(get(controller.state)).toMatchObject({
        phase: 'unavailable', busy: false, verifiedReadCount: 0,
      });
      resolveOld({state:'direct_reachable', last_failure:'scinvite:never-show'});
      await Promise.resolve();
      expect(get(controller.state).verifiedReadCount).toBe(0);
      await controller.refresh();
      expect(get(controller.state)).toMatchObject({
        phase: 'checked', busy: false, verifiedReadCount: 1, label: 'Direct path available',
      });
    } finally {
      vi.useRealTimers();
    }
  });
});
