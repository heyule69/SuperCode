import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

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
  const manifest = JSON.parse(readFileSync(join(directory, 'latest.json'), 'utf8'));
  const packageInfo = manifest.platforms?.['windows-x86_64'];
  const digest = createHash('sha256').update(bytes).digest('hex');
  const signature = packageInfo?.signature;
  if (manifest.version !== version || bytes.subarray(0, 2).toString() !== 'MZ' || bytes.length > 256 * 1024 * 1024
    || packageInfo?.size !== bytes.length || packageInfo?.sha256 !== digest || typeof signature !== 'string' || !signature.trim()
    || packageInfo?.url !== `https://github.com/heyule69/SuperCode/releases/download/v${version}/${name}`
  ) throw new Error('Release assets do not match their manifest');
  verifySignedBytes(bytes, signature, readFileSync(join(root, 'src-tauri/update-public-key.txt'), 'utf8'), version);
  return { name, digest, bytes: bytes.length, manifest, assets: [name, 'latest.json'] };
}
