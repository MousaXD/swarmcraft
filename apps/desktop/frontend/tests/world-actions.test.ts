import { describe, expect, it, vi } from 'vitest';
import { createWorldActions, parseCreatedWorld, validateImport, WorldFormError, type WorldActionsBackend } from '../src/lib/world-actions';

const create = { name:'My world', minecraft:'26.1.2', fabricLoader:'0.19.3', compatibility:'vanilla-fabric', visibility:'private' as const };
const importing = { source:'/world', name:'Imported world', minecraft:'26.1.2', fabricLoader:'0.19.3', visibility:'unlisted' as const, serverMods:'/mods/a.jar\n/mods/b.jar', noServerMods:false };
function backend(overrides: Partial<WorldActionsBackend> = {}): WorldActionsBackend {
  return {
    createWorld: vi.fn(async () => 'Created world:\nWorld ID: scworld:abc\n'),
    importWorld: vi.fn(async () => JSON.stringify({ world_id:'scworld:imported' })),
    joinWorld: vi.fn(async () => 'Joined'),
    createInvite: vi.fn(async () => 'signed-invite'),
    ...overrides,
  };
}

describe('world actions preserve native contracts', () => {
  it('creates with the exact existing payload and canonical ID returned by Rust', async () => {
    const native = backend(); const actions = createWorldActions(native);
    await expect(actions.create(create)).resolves.toBe('scworld:abc');
    expect(native.createWorld).toHaveBeenCalledWith(create);
    expect(() => parseCreatedWorld('Success but no ID')).toThrow(/canonical World ID/);
  });
  it('imports explicit mods without EULA or invented host admission', async () => {
    const native = backend(); const actions = createWorldActions(native);
    await expect(actions.import(importing)).resolves.toMatchObject({ worldId:'scworld:imported' });
    expect(native.importWorld).toHaveBeenCalledWith({
      source:'/world', name:'Imported world', minecraft:'26.1.2',
      fabricLoader:'0.19.3', visibility:'unlisted',
      serverMods:['/mods/a.jar','/mods/b.jar'], noServerMods:false,
    });
    const request = vi.mocked(native.importWorld).mock.calls[0][0];
    expect(Object.keys(request)).not.toContain('acceptEula');
    expect(Object.keys(request)).not.toContain('authorityEligible');
  });
  it('fails closed when server-mod declaration is incomplete or conflicting', () => {
    expect(() => validateImport({ ...importing, serverMods:[], noServerMods:false })).toThrow(/explicitly confirm/);
    expect(() => validateImport({ ...importing, noServerMods:true })).toThrow(/Choose either/);
  });
  it('propagates backend error without a success or world-id placeholder', async () => {
    const native = backend({ importWorld: async () => { throw new Error('source path rejected by Rust'); } });
    await expect(createWorldActions(native).import(importing)).rejects.toThrow(/source path rejected/);
    expect(() => validateImport({ ...importing, source:'' })).toThrow(WorldFormError);
  });
  it('leaves signed membership checks to Rust and passes invite unchanged except whitespace', async () => {
    const native = backend();const actions = createWorldActions(native);
    expect(await actions.join(' signed-token ')).toBe('Joined');
    expect(native.joinWorld).toHaveBeenCalledWith('signed-token');
  });
  it('creates an invite using current Rust payload and rejects invalid duration', async () => {
    const native = backend();const actions = createWorldActions(native);
    await actions.invite('scworld:abc', 60, [' /ip4/127.0.0.1/tcp/123 ']);
    expect(native.createInvite).toHaveBeenCalledWith({
      world:'scworld:abc', expiresMinutes:60, bootstrapAddrs:['/ip4/127.0.0.1/tcp/123'],
    });
    await expect(actions.invite('scworld:abc', 0, [])).rejects.toThrow(/valid invite expiry/);
  });
  it('rejects concurrent duplicate destructive/create operations', async () => {
    let resolve!: (value:string) => void;
    const stalled = new Promise<string>(r=>{resolve=r});
    const native = backend({ createWorld: async () => stalled });
    const actions = createWorldActions(native);
    const first = actions.create(create);
    await expect(actions.create(create)).rejects.toThrow(/already running/);
    resolve('World ID: scworld:done');
    expect(await first).toBe('scworld:done');
  });
});
