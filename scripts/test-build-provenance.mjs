import test from 'node:test';
import assert from 'node:assert/strict';
import { verifySourceBinding, assertUnchanged, verifyAppBuild, sha256 } from './build-provenance.mjs';
const source = { sourceCommit: 'a'.repeat(40), sourceTree: 'b'.repeat(40), fingerprint: 'c'.repeat(64), dirty: false };
test('rejects an old installer relabeled as a later source commit', () => {
  verifySourceBinding({ source }, source.sourceCommit, source.sourceTree);
  assert.throws(() => verifySourceBinding({ source }, 'd'.repeat(40), source.sourceTree));
  assert.throws(() => verifySourceBinding({ source: { ...source, dirty: true } }, source.sourceCommit, source.sourceTree));
});
test('rejects source changes during compilation and stale reused executable', () => {
  const app = Buffer.from('fixture executable');
  verifyAppBuild({ source, appSha256: sha256(app) }, source, app);
  assert.throws(() => assertUnchanged(source, { ...source, fingerprint: 'e'.repeat(64) }));
  assert.throws(() => verifyAppBuild({ source, appSha256: sha256(app) }, source, Buffer.from('other executable')));
});
