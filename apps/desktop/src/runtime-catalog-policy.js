// Provider availability, the bridge's declared range and field qualification
// are separate contracts. Only the native pinned baseline is selectable.
const states = new Set(['pinned_baseline', 'adapter_compatible_unqualified', 'unsupported_adapter']);

export function catalogSelectionState(entry) {
  if (!entry || typeof entry.adapter_supported !== 'boolean'
      || !states.has(entry.selection_state)
      || entry.adapter_supported !== (entry.selection_state !== 'unsupported_adapter')) {
    throw new Error('Version catalog contained an invalid shipped-adapter compatibility flag or qualification state.');
  }
  return entry.selection_state;
}

export function catalogChoiceLabel(label, state, available = true) {
  if (!available) return label + ' (not available from provider)';
  if (state === 'pinned_baseline') return label + ' (pinned baseline)';
  if (state === 'adapter_compatible_unqualified') return label + ' (not yet qualified)';
  return label + ' (incompatible adapter)';
}

export const baselineNotice = 'Pinned baseline only. Real Fabric launch, save/restore and authenticated client acceptance remain pending.';
