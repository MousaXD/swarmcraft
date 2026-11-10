import { nativeBridge, type HostingContributionReadiness } from './native-bridge';

function requireWorld(world: string): string {
  if (!/^scworld:[0-9a-f]{64}$/.test(world)) {
    throw new Error('Select a canonical local world before changing hosting contribution.');
  }
  return world;
}

/** Local voluntary contribution, NEVER world authority or a guaranteed JVM RSS cap. */
export async function readHostingContribution(world: string): Promise<HostingContributionReadiness> {
  return nativeBridge().invoke('hosting_contribution_get', { world: requireWorld(world) });
}

/** Only an explicit caller may write 0 (withdrawal) or 512..65536 MiB. */
export async function saveHostingContribution(world: string, heapMib: number): Promise<HostingContributionReadiness> {
  const id = requireWorld(world);
  if (!Number.isSafeInteger(heapMib) || (heapMib !== 0 && (heapMib < 512 || heapMib > 65_536))) {
    throw new Error('Select 0 (player-only) or a whole-number Java heap between 512 and 65536 MiB.');
  }
  return nativeBridge().invoke('hosting_contribution_set', { world: id, heapMib });
}
