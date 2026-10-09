// Publish tested local assets through GitHub Actions using the existing Git SSH login.
// Only the installer, update manifest and source.json enter a temporary, parentless Git branch.
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';
import { verifyRelease } from './release-contract.mjs';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
function git(args, options={}) {
  const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', windowsHide: true, shell: false, ...options });
  if (result.error || result.status !== 0) throw new Error(result.stderr || result.error?.message || 'Git command failed');
  return result.stdout.trim();
}
if (git(['status', '--porcelain'])) throw new Error('Commit the tested source before publishing');
const version = JSON.parse(git(['show', 'HEAD:package.json'])).version;
const tag = `v${version}`, ref = `refs/heads/release-assets/${tag}`;
const sourceCommit = git(['rev-parse', 'HEAD']);
const directory = join(root, 'release');
const release = verifyRelease(directory, root, version);
const existing = git(['ls-remote', '--tags', 'origin', `refs/tags/${tag}`]);
if (existing) throw new Error(`${tag} already exists; do not replace a published release`);
const temporary = join(root, '.supercode', `release-index-${randomUUID()}`);
mkdirSync(dirname(temporary), { recursive: true });
const env = { ...process.env, GIT_INDEX_FILE: temporary };
try {
  git(['read-tree', '--empty'], { env });
  const source = join(directory, 'source.json');
  writeFileSync(source, `${JSON.stringify({ version, sourceCommit, sha256: release.digest }, null, 2)}\n`, 'utf8');
  for (const name of [...release.assets, 'source.json']) {
    const blob = git(['hash-object', '-w', join(directory, name)]);
    git(['update-index', '--add', '--cacheinfo', `100644,${blob},${name}`], { env });
  }
  const tree = git(['write-tree'], { env });
  const staged = git(['commit-tree', tree, '-m', `Temporary signed release assets for ${tag} (${sourceCommit})`]);
  git(['push', 'origin', `${staged}:${ref}`]);
  // The release tag always points at source, never at the binary staging commit.
  git(['tag', '-a', tag, sourceCommit, '-m', `SuperCode ${version}`]);
  git(['push', 'origin', `refs/tags/${tag}`]);
  console.log(`GitHub Actions is publishing ${tag}: https://github.com/heyule69/SuperCode/actions`);
} finally {
  rmSync(temporary, { force: true });
}
