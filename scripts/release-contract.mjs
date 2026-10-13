import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const utf8 = new TextDecoder('utf-8', { fatal: true });
function decodeText(bytes, name) {
  if (bytes.subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf]))) throw new Error(`${name} must be UTF-8 without BOM`);
  return utf8.decode(bytes);
}
function freezeJson(value) {
  if (value && typeof value === 'object') {
    for (const child of Object.values(value)) freezeJson(child);
    Object.freeze(value);
  }
  return value;
}

export function verifySignedBytes(bytes, signatureBase64, publicKeyBase64, version) {
  const keyLines = Buffer.from(publicKeyBase64.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
  const sigLines = Buffer.from(signatureBase64.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
  const key = Buffer.from(keyLines[1] ?? '', 'base64');
  const sig = Buffer.from(sigLines[1] ?? '', 'base64');
  const global = Buffer.from(sigLines[3] ?? '', 'base64');
  const comment = sigLines[2]?.replace(/^trusted comment: /, '');
  if (key.length !== 42 || sig.length !== 74 || global.length !== 64 || sig.subarray(0, 2).toString() !== 'ED'
    || !key.subarray(2, 10).equals(sig.subarray(2, 10)) || !comment?.split('\t').includes(`version:${version}`)) {
    throw new Error('Invalid update signature or signed version');
  }
  const publicKey = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]), format: 'der', type: 'spki' });
  if (!verify(null, createHash('blake2b512').update(bytes).digest(), publicKey, sig.subarray(10))
    || !verify(null, Buffer.concat([sig.subarray(10), Buffer.from(comment, 'utf8')]), publicKey, global)) {
    throw new Error('Update signature verification failed');
  }
}
export function verifyRelease(directory, root, version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Only stable release versions are supported');
  const name = `SuperCode_${version}_x64-setup.exe`;
  const bytes = readFileSync(join(directory, name));
  const manifestBytes = readFileSync(join(directory, 'latest.json'));
  const metadataBytes = readFileSync(join(directory, 'installer-build.json'));
  const metadataSignatureBytes = readFileSync(join(directory, 'installer-build.json.sig'));
  const publicKey = decodeText(readFileSync(join(root, 'src-tauri/update-public-key.txt')), 'Update public key');
  const manifest = JSON.parse(decodeText(manifestBytes, 'Update manifest'));
  const packageInfo = manifest.platforms?.['windows-x86_64'];
  const digest = createHash('sha256').update(bytes).digest('hex');
  const signature = packageInfo?.signature;
  if (manifest.version !== version || bytes.subarray(0, 2).toString() !== 'MZ' || bytes.length > 256 * 1024 * 1024
    || packageInfo?.size !== bytes.length || packageInfo?.sha256 !== digest || typeof signature !== 'string' || !signature.trim()
    || packageInfo?.url !== `https://github.com/heyule69/SuperCode/releases/download/v${version}/${name}`
  ) throw new Error('Release assets do not match their manifest');
  verifySignedBytes(bytes, signature, publicKey, version);
  verifySignedBytes(metadataBytes, decodeText(metadataSignatureBytes, 'Build metadata signature'), publicKey, version);
  const metadata = JSON.parse(decodeText(metadataBytes, 'Build metadata'));
  if (metadata.version !== version || metadata.sha256 !== digest || metadata.bytes !== bytes.length) throw new Error('Signed build metadata does not match the installer');
  // Keep the exact verified bytes private. Buffer itself is mutable, so each
  // consumer receives a copy; neither disk changes nor a caller can alter the
  // snapshot subsequently used for Git staging or upload.
  const snapshot = new Map([[name, bytes], ['latest.json', manifestBytes], ['installer-build.json', metadataBytes], ['installer-build.json.sig', metadataSignatureBytes]]);
  const readAsset = asset => {
    if (!snapshot.has(asset)) throw new Error('Unknown verified release asset');
    return Buffer.from(snapshot.get(asset));
  };
  return Object.freeze({ name, digest, bytes: bytes.length, manifest: freezeJson(manifest), metadata: freezeJson(metadata), assets: Object.freeze([...snapshot.keys()]), readAsset });
}
