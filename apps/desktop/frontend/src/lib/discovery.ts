import { nativeBridge } from './native-bridge';
import { withDiscoveryDeadline } from '../../../src/discovery-diagnostics.js';
export { formatDiscoveryResult, discoveryUnavailable, normalizeWorldId } from '../../../src/discovery-diagnostics.js';

export async function resolveSignedDiscovery(world: string): Promise<unknown> {
  // Discovery is a read-only signed announcement lookup, never a join.
  return withDiscoveryDeadline(nativeBridge().invoke('discovery_resolve', { world }));
}
