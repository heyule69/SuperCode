import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync, rmSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { createHash, generateKeyPairSync, sign } from 'node:crypto';
import { verifySignedBytes, verifyRelease } from './release-contract.mjs';
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
function withReleaseFixture(run) {
  const base = realpathSync(tmpdir());
  const directory = mkdtempSync(join(base, 'supercode-signed-build-'));
  try {
    mkdirSync(join(directory, 'src-tauri'));
    const pair = generateKeyPairSync('ed25519');
    const keyId = Buffer.from('fixture1');
    const rawKey = pair.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
    const encodedKey = Buffer.from(`untrusted comment: test public key\n${Buffer.concat([Buffer.from('Ed'), keyId, rawKey]).toString('base64')}\n`).toString('base64');
    writeFileSync(join(directory, 'src-tauri/update-public-key.txt'), encodedKey, 'utf8');
    const signFixture = data => {
      const fileSignature = sign(null, createHash('blake2b512').update(data).digest(), pair.privateKey);
      const comment = 'timestamp:0\tfile:fixture\tversion:0.2.2';
      const global = sign(null, Buffer.concat([fileSignature, Buffer.from(comment, 'utf8')]), pair.privateKey);
      return Buffer.from(`untrusted comment: test fixture\n${Buffer.concat([Buffer.from('ED'), keyId, fileSignature]).toString('base64')}\ntrusted comment: ${comment}\n${global.toString('base64')}\n`).toString('base64');
    };
    // Deliberately non-runnable fixture: verifies the release contract, not installation.
    const installer = Buffer.from('MZ non-runnable release fixture');
    const digest = createHash('sha256').update(installer).digest('hex');
    writeFileSync(join(directory, 'SuperCode_0.2.2_x64-setup.exe'), installer);
    writeFileSync(join(directory, 'latest.json'), JSON.stringify({ version:'0.2.2', platforms:{ 'windows-x86_64':{ url:'https://github.com/heyule69/SuperCode/releases/download/v0.2.2/SuperCode_0.2.2_x64-setup.exe', size:installer.length, sha256:digest, signature:signFixture(installer) } } }), 'utf8');
    const metadata = Buffer.from(JSON.stringify({ version:'0.2.2', bytes:installer.length, sha256:digest }), 'utf8');
    writeFileSync(join(directory, 'installer-build.json'), metadata);
    writeFileSync(join(directory, 'installer-build.json.sig'), signFixture(metadata), 'utf8');
    run({ directory, installer, digest, signFixture });
  } finally {
    assert.equal(dirname(realpathSync(directory)), base);
    assert.match(directory.split(/[\\/]/).at(-1), /^supercode-signed-build-/);
    rmSync(directory, { recursive:true, force:true });
  }
}

test('release verification binds signed build metadata to the actual installer bytes', () => withReleaseFixture(({ directory, installer, digest, signFixture }) => {
  assert.equal(verifyRelease(directory, directory, '0.2.2').metadata.sha256, digest);
  writeFileSync(join(directory, 'installer-build.json'), JSON.stringify({ version:'0.2.2', bytes:installer.length, sha256:'0'.repeat(64) }), 'utf8');
  assert.throws(() => verifyRelease(directory, directory, '0.2.2'));
  const mismatched = Buffer.from(JSON.stringify({ version:'0.2.2', bytes:installer.length, sha256:'0'.repeat(64) }), 'utf8');
  writeFileSync(join(directory, 'installer-build.json.sig'), signFixture(mismatched), 'utf8');
  assert.throws(() => verifyRelease(directory, directory, '0.2.2'), /metadata does not match/);
}));

test('verified publish snapshot survives every disk asset being rewritten and caller buffer mutation', () => withReleaseFixture(({ directory }) => {
  const release = verifyRelease(directory, directory, '0.2.2');
  const originals = new Map(release.assets.map(name => [name, readFileSync(join(directory, name))]));
  for (const name of release.assets) writeFileSync(join(directory, name), `changed after validation: ${name}`, 'utf8');
  assert.throws(() => verifyRelease(directory, directory, '0.2.2'));
  for (const name of release.assets) {
    assert.deepEqual(release.readAsset(name), originals.get(name));
    // Publication receives its own byte buffer, including the signature text
    // actually used during validation. Changing a returned buffer is harmless.
    release.readAsset(name).fill(0);
    assert.deepEqual(release.readAsset(name), originals.get(name));
  }
  assert.throws(() => release.readAsset('../unknown'), /Unknown verified/);
  assert.throws(() => release.assets.push('unverified'));
  assert.throws(() => { release.metadata.sha256 = '0'.repeat(64); });
}));
