import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { verifySignedBytes } from './release-contract.mjs';
const bytes = readFileSync(new URL('../src-tauri/testdata/update-fixture.txt', import.meta.url));
const signature = readFileSync(new URL('../src-tauri/testdata/update-fixture.txt.sig', import.meta.url), 'utf8');
const publicKey = readFileSync(new URL('../src-tauri/update-public-key.txt', import.meta.url), 'utf8');
test('CI verifies the same Ed25519 signatures as the native updater', () => verifySignedBytes(bytes, signature, publicKey, '0.2.0'));
test('rejects a tampered package and incorrect release version', () => {
  assert.throws(() => verifySignedBytes(Buffer.from('tampered'), signature, publicKey, '0.2.0'));
  assert.throws(() => verifySignedBytes(bytes, signature, publicKey, '0.3.0'));
});
test('rejects a forged trusted comment even when the file signature is unchanged', () => {
  const forged = Buffer.from(Buffer.from(signature.trim(), 'base64').toString('utf8').replace('version:0.2.0', 'version:0.3.0'), 'utf8').toString('base64');
  assert.throws(() => verifySignedBytes(bytes, forged, publicKey, '0.3.0'));
});
