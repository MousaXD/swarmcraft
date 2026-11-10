import { nativeBridge } from './native-bridge';

export function resolveSignedDiscovery(world: string): Promise<unknown> {
  // Discovery is not membership. Rust alone verifies signed announcements.
  return nativeBridge().invoke('discovery_resolve', { world });
}

type Tone = 'safe' | 'warning' | 'danger';
export type DiscoveryView = Readonly<{ message: string; tone: Tone }>;
const notice = ' A World ID does not grant membership; a signed invite may still be required.';
const record = (x: unknown): Record<string, unknown> | null =>
  x && typeof x === 'object' && !Array.isArray(x) ? x as Record<string, unknown> : null;
const clean = (x: unknown): string => {
  if (typeof x !== 'string') return '';
  if (/authorization|bearer|private.?key|password|ipc.?token|invite.?token|secret/i.test(x)) return '';
  return x.trim().slice(0, 180);
};
export function formatDiscoveryResult(response: unknown): DiscoveryView {
  const r = record(response);
  if (!r) return {tone:'warning',message:'Invalid discovery response.' + notice};
  if (r.state === 'found') {
    const world = record(r.world);
    if (!world || !clean(world.name) || !['public','unlisted','private'].includes(clean(world.visibility))) {
      return {tone:'warning',message:'Incomplete signed announcement.' + notice};
    }
    return {tone:'safe',message:clean(world.name)+' is '+clean(world.visibility)+
      '. Discovery verified its signed announcement.'+notice};
  }
  return {tone:'warning',message:(clean(r.detail)||('World resolution: '+(clean(r.state)||'unavailable')))+notice};
}
export const discoveryUnavailable = (): DiscoveryView => ({
  tone:'danger',message:'Signed discovery could not be verified. Check networking and retry.'+notice,
});
