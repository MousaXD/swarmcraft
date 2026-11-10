import { afterEach, describe, expect, it, vi } from 'vitest';
import { readHostingContribution, saveHostingContribution } from '../src/lib/hosting-contribution';

const WORLD = 'scworld:' + 'a'.repeat(64);
afterEach(() => vi.unstubAllGlobals());

describe('real native hosting contribution commands', () => {
  it('reads the Rust-backed local status without granting permission or writing preferences', async () => {
    const result = { world_id: WORLD, consented: false, selected_heap_mib: 0, may_host_now: false };
    const invoke = vi.fn(async () => result);
    const core = Object.freeze({ invoke });
    vi.stubGlobal('window', { __TAURI__: Object.freeze({ core }) });
    await expect(readHostingContribution(WORLD)).resolves.toEqual(result);
    expect(invoke).toHaveBeenCalledExactlyOnceWith('hosting_contribution_get', { world: WORLD });
    expect(core.invoke).toBe(invoke);
  });

  it('requires a deliberate nonzero budget and forwards the exact camelCase Tauri argument', async () => {
    const invoke = vi.fn(async () => ({ world_id: WORLD, consented: true, selected_heap_mib: 1024 }));
    vi.stubGlobal('window', { __TAURI__: { core: { invoke } } });
    await saveHostingContribution(WORLD, 1024);
    expect(invoke).toHaveBeenCalledExactlyOnceWith('hosting_contribution_set', { world: WORLD, heapMib: 1024 });
  });

  it('allows explicit zero withdrawal without assuming a safe host stop', async () => {
    const invoke = vi.fn(async () => ({ world_id: WORLD, consented: false, selected_heap_mib: 0 }));
    vi.stubGlobal('window', { __TAURI__: { core: { invoke } } });
    await saveHostingContribution(WORLD, 0);
    expect(invoke).toHaveBeenCalledExactlyOnceWith('hosting_contribution_set', { world: WORLD, heapMib: 0 });
  });

  it('rejects malformed IDs and invalid heap budgets before any native call', async () => {
    const invoke = vi.fn(async () => ({}));
    vi.stubGlobal('window', { __TAURI__: { core: { invoke } } });
    await expect(readHostingContribution('../escape')).rejects.toThrow(/canonical local world/);
    for (const invalid of [-1, 511, 65537, Number.MAX_SAFE_INTEGER, 512.5]) {
      await expect(saveHostingContribution(WORLD, invalid)).rejects.toThrow(/Java heap/);
    }
    expect(invoke).not.toHaveBeenCalled();
  });

  it('fails closed if Tauri is unavailable instead of storing browser-only consent', async () => {
    vi.stubGlobal('window', {});
    await expect(readHostingContribution(WORLD)).rejects.toThrow(/Tauri is unavailable/);
    await expect(saveHostingContribution(WORLD, 0)).rejects.toThrow(/Tauri is unavailable/);
  });
});
