const labels = Object.freeze({
  acquired: 'Downloaded and verified',
  already_verified: 'Already verified locally',
  manual_required: 'Supply the exact authorized JAR manually',
  unavailable: 'Provider unavailable or request failed; retry or supply the exact JAR manually',
  incompatible: 'Incompatible artifact; review the local files or obtain the exact required JAR',
});

export function describeModAcquisition(report, world) {
  const token = value => typeof value === 'string' && value.length > 0 && value.length <= 512
    && !/[\x00-\x1f\x7f]/.test(value);
  if (report?.world_id !== world || typeof report.ready !== 'boolean'
      || !Array.isArray(report.artifacts) || !Array.isArray(report.inventory_issues)
      || report.artifacts.length > 2048 || report.inventory_issues.length > 4096
      || !report.artifacts.every(a => a && token(a.mod_id) && token(a.version)
        && /^[a-f0-9]{64}$/.test(a.artifact_hash)
        && Object.hasOwn(labels, a.state)
        && (a.provider === null ? a.project_id === null && a.version_id === null && a.file_name === null
          : ['modrinth', 'curse_forge'].includes(a.provider) && token(a.project_id) && token(a.version_id) && token(a.file_name)))
      || !report.inventory_issues.every(token)
      || new Set(report.artifacts.map(a => a.mod_id)).size !== report.artifacts.length
      || (report.ready && (report.inventory_issues.length || report.artifacts.some(a => !['acquired','already_verified'].includes(a.state))))) {
    throw new Error('Invalid or mismatched mod acquisition report.');
  }
  return [
    report.ready ? 'Required server JAR inventory verified. This does not install the runtime or authorize hosting.'
      : 'Server mod inventory still needs attention. Verified files are retained; retry or supply the missing exact JARs.',
    ...report.artifacts.map(a => {
      const source = a.provider === null ? 'local/imported artifact'
        : (a.provider === 'modrinth' ? 'Modrinth' : 'CurseForge') + ' ' + a.project_id + '/' + a.version_id + ' · ' + a.file_name;
      return a.mod_id + ' ' + a.version + ': ' + labels[a.state] + '\n' + source + '\nExpected artifact hash: ' + a.artifact_hash;
    }),
    ...(report.inventory_issues.length ? ['Inventory issues: ' + report.inventory_issues.join(', ') + '. Refresh / Verify for details.'] : []),
  ].join('\n');
}
