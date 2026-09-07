import test from 'node:test';
import assert from 'node:assert/strict';
import { canonicalPackageFromDownloaded, errorText } from '../src/launcher-controller.js';

test('downloaded provider artifact maps runtime identity and exact provider provenance', () => {
  const selected = new Map([['dep', 'dep-version']]);
  const result = canonicalPackageFromDownloaded({
    provider: 'modrinth',
    version: {
      project_id: 'root',
      version_id: 'root-version',
      dependencies: [
        { kind: 'required', project_id: 'dep', version_id: null },
        { kind: 'optional', project_id: 'not-selected', version_id: null },
      ],
    },
    file: { filename: 'root.jar' },
    downloaded: { path: '/tmp/root.jar', filename: 'root.jar', size: 42, hashes: { sha512: 'ab'.repeat(64) } },
    inspection: { mod_id: 'real_fabric_mod_id', version: '1.2.3', environment: 'universal' },
    selectedByProject: selected,
  });
  assert.equal(result.artifactId, 'real_fabric_mod_id');
  assert.equal(result.version, '1.2.3');
  assert.equal(result.side, 'both');
  assert.equal(result.projectId, 'root');
  assert.equal(result.versionId, 'root-version');
  assert.deepEqual(result.dependencies, [{ kind: 'required', projectId: 'dep', versionId: 'dep-version' }]);
  assert.equal(result.providerHashes[0].algorithm, 'sha512');
});

test('missing exact required dependency fails closed', () => {
  assert.throws(
    () =>
      canonicalPackageFromDownloaded({
        provider: 'curseforge',
        version: { project_id: '1', version_id: '2', dependencies: [{ kind: 'required', project_id: '3' }] },
        file: { file_name: 'x.jar', hashes: [{ algorithm: 'sha1', value: 'ab'.repeat(20) }] },
        downloaded: { destination: '/tmp/x.jar', bytes: 1 },
        inspection: { mod_id: 'x', version: '1', environment: 'server' },
        selectedByProject: new Map(),
      }),
    /did not resolve to an exact artifact/,
  );
});

test('structured backend error remains actionable', () => {
  assert.equal(errorText({ error: { message: 'CurseForge API key is not configured.' } }), 'CurseForge API key is not configured.');
});


test('MD5-only provider provenance is canonicalized as manual-required', () => {
  const result = canonicalPackageFromDownloaded({
    provider: 'curseforge',
    version: { project_id: '1', version_id: '2', dependencies: [] },
    file: { file_name: 'x.jar', hashes: [{ algorithm: 'md5', value: 'ab'.repeat(16) }] },
    downloaded: { destination: '/tmp/x.jar', bytes: 1 },
    inspection: { mod_id: 'x', version: '1', environment: 'server' },
    selectedByProject: new Map(),
  });
  assert.equal(result.retrieval, 'manual_required');
});
