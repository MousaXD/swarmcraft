import { afterEach, describe, expect, it, vi } from 'vitest';
import { discoveryUnavailable, formatDiscoveryResult, resolveSignedDiscovery } from '../src/lib/discovery';

afterEach(() => vi.unstubAllGlobals());

describe('Svelte World ID discovery uses Rust signed-announcement verification', () => {
  it('calls exactly discovery_resolve with the supplied world ID, never join_world', async () => {
    const invoke = vi.fn(async () => ({ state: 'not_found', detail: 'No signed announcement' }));
    vi.stubGlobal('window', { __TAURI__: { core: { invoke } } });
    const report = await resolveSignedDiscovery('scworld:abc');
    expect(invoke).toHaveBeenCalledExactlyOnceWith('discovery_resolve', { world: 'scworld:abc' });
    expect(formatDiscoveryResult(report).message).toContain('does not grant membership');
  });
  it('displays a Rust-verified signed public announcement without inventing membership', () => {
    const r = formatDiscoveryResult({ state: 'found', world: {name:'Friends', visibility:'public'} });
    expect(r.tone).toBe('safe');
    expect(r.message).toContain('signed announcement');
    expect(r.message).toContain('does not grant membership');
  });
  it('refuses false success on malformed and partial backend responses', () => {
    for (const report of [undefined, {state:'found'}, {state:'found',world:{}},
      {state:'found',world:{name:'Unverified',visibility:'unknown'}}]) {
      expect(formatDiscoveryResult(report).tone).toBe('warning');
    }
  });
  it('redacts credential-looking native diagnostic content', () => {
    const text = formatDiscoveryResult({state:'failed',detail:'bearer private key secret'}).message;
    expect(text).not.toContain('private key secret');
    expect(discoveryUnavailable()).toMatchObject({
      tone:'danger',message:expect.stringContaining('does not grant membership'),
    });
  });
  it('never silently claims discovery when native Tauri IPC is missing', async () => {
    vi.stubGlobal('window', {});
    await expect(resolveSignedDiscovery('scworld:abc')).rejects.toThrow(/Tauri is unavailable/);
  });
});
