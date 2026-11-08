import { afterEach, describe, expect, it, vi } from 'vitest';
import { nativeWorldBackend } from '../src/lib/native-world-actions';

afterEach(() => vi.unstubAllGlobals());

describe('Svelte native world actions bridge', () => {
  it('uses the exact Rust create_world command/payload and never patches invoke', async () => {
    const invoke = vi.fn(async () => 'World ID: scworld:real');
    const core = Object.freeze({ invoke });
    vi.stubGlobal('window', { __TAURI__: Object.freeze({ core }) });
    const actions = nativeWorldBackend();
    const result = await actions.createWorld({
      name: 'Friends', minecraft: '26.1.2', fabricLoader: '0.19.3',
      compatibility: 'vanilla-fabric', visibility: 'private',
    });
    expect(result).toContain('scworld:real');
    expect(invoke).toHaveBeenCalledExactlyOnceWith('create_world', {
      name: 'Friends', minecraft: '26.1.2', fabricLoader: '0.19.3',
      compatibility: 'vanilla-fabric', visibility: 'private',
    });
    expect(core.invoke).toBe(invoke);
  });

  it('fails closed if Tauri IPC is unavailable instead of pretending creation worked', async () => {
    vi.stubGlobal('window', {});
    await expect(nativeWorldBackend().createWorld({
      name: 'Test', minecraft: '26.1.2', fabricLoader: '0.19.3',
      compatibility: 'vanilla-fabric', visibility: 'private',
    })).rejects.toThrow(/Tauri is unavailable/);
  });

  it('keeps join, import, and invite contracts byte-for-byte aligned with Rust', async () => {
    const invoke = vi.fn(async () => 'World ID: scworld:test');
    vi.stubGlobal('window', { __TAURI__: { core: { invoke } } });
    const backend = nativeWorldBackend();
    await backend.joinWorld('signed invite token');
    await backend.importWorld({
      source: '/tmp/world', name: 'Existing', minecraft: '26.1.2',
      fabricLoader: '0.19.3', visibility: 'private',
      serverMods: ['/tmp/mod.jar'], noServerMods: false,
    });
    await backend.createInvite({world:'scworld:real',expiresMinutes:60,bootstrapAddrs:[]});
    expect(invoke.mock.calls.map(call=>call[0])).toEqual(['join_world','import_world','create_invite']);
    expect(invoke.mock.calls[0][1]).toEqual({invite:'signed invite token'});
    expect(invoke.mock.calls[1][1]).not.toHaveProperty('acceptEula');
    expect(invoke.mock.calls[2][1]).toEqual({
      world:'scworld:real',expiresMinutes:60,bootstrapAddrs:[],
    });
  });
});
