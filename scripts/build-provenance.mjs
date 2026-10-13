import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

function git(root, args) {
  const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', windowsHide: true, shell: false });
  if (result.error || result.status !== 0) throw new Error('Cannot determine build source');
  return result.stdout.trim();
}
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export const sourceTree = root => git(root, ['rev-parse', 'HEAD^{tree}']);
export function captureSource(root) {
  const paths = git(root, ['ls-files', '-z', '--cached', '--others', '--exclude-standard']).split('\0').filter(Boolean).sort();
  const digest = createHash('sha256');
  for (const path of paths) {
    digest.update(path).update('\0');
    try { digest.update(sha256(readFileSync(join(root, path)))); }
    catch (error) { if (error.code !== 'ENOENT') throw error; digest.update('deleted'); }
    digest.update('\0');
  }
  return { sourceCommit: git(root, ['rev-parse', 'HEAD']), sourceTree: git(root, ['rev-parse', 'HEAD^{tree}']), fingerprint: digest.digest('hex'), dirty: !!git(root, ['status', '--porcelain']) };
}
export function assertUnchanged(before, after) {
  if (before.sourceCommit !== after.sourceCommit || before.fingerprint !== after.fingerprint) throw new Error('Source changed during build; rebuild before publishing');
}
export function verifyAppBuild(stamp, source, appBytes) {
  assertUnchanged(stamp.source, source);
  if (stamp.appSha256 !== sha256(appBytes)) throw new Error('Reused executable does not match its build stamp');
}
export function verifySourceBinding(metadata, expectedCommit, expectedTree) {
  const source = metadata?.source;
  if (!source || source.dirty !== false || !/^[a-f0-9]{64}$/.test(source.fingerprint ?? '')
    || source.sourceCommit !== expectedCommit || source.sourceTree !== expectedTree) {
    throw new Error('Installer was not built from this clean source commit; commit source and rebuild');
  }
}
