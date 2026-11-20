export interface WorldSummary { id: string; name: string; snapshots: number }

/** Exact current CLI text contract; parse only fully formed canonical IDs. */
export function parseWorldList(text: string): WorldSummary[] {
  return String(text ?? '')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.startsWith('scworld:'))
    .map((line) => {
      const match = line.match(/^(scworld:[^\s]+)\s+(.*?)\s+snapshots=(\d+)$/);
      return match ? { id: match[1], name: match[2], snapshots: Number(match[3]) } : null;
    })
    .filter((world): world is WorldSummary => world !== null);
}
