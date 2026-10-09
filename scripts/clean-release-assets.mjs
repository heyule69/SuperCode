// Remove redundant sidecars only after verifying the published installer against latest.json.
import { appendFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { verifyRelease } from './release-contract.mjs';

const repository = 'heyule69/SuperCode';
const tag = process.env.RELEASE_TAG;
const dryRun = process.argv.includes('--dry-run');
if (process.env.GITHUB_REPOSITORY !== repository || !/^v\d+\.\d+\.\d+$/.test(tag ?? '')) {
  throw new Error('Invalid release destination or tag');
}
const token = process.env.GITHUB_TOKEN;
if (!dryRun && !token) throw new Error('GitHub Actions token is unavailable');
const api = `https://api.github.com/repos/${repository}`;
async function request(path, method = 'GET') {
  const response = await fetch(`${api}${path}`, {
    method,
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'SuperCode-release-maintenance',
    },
    signal: AbortSignal.timeout(30_000),
  });
  if (!response.ok) throw new Error(`GitHub API returned ${response.status}`);
  return response.status === 204 ? null : response.json();
}

const published = await request(`/releases/tags/${tag}`);
if (published.tag_name !== tag || published.draft || published.prerelease) throw new Error('Expected a published stable release');
const version = tag.slice(1), name = `SuperCode_${version}_x64-setup.exe`;
const redundant = [`${name}.sha256`, `${name}.sig`];
const retained = [name, 'latest.json'].map(assetName => {
  const asset = published.assets.find(item => item.name === assetName && item.state === 'uploaded');
  if (!asset) throw new Error(`Missing required release asset: ${assetName}`);
  return asset;
});

const temporary = mkdtempSync(join(tmpdir(), 'supercode-release-check-'));
try {
  for (const asset of retained) {
    const expectedUrl = `https://github.com/${repository}/releases/download/${tag}/${asset.name}`;
    const limit = asset.name === 'latest.json' ? 64 * 1024 : 256 * 1024 * 1024;
    if (asset.browser_download_url !== expectedUrl || !Number.isSafeInteger(asset.size) || asset.size < 1 || asset.size > limit) {
      throw new Error(`Invalid release asset: ${asset.name}`);
    }
    // Download public assets without sending the Actions token to download hosts.
    const response = await fetch(expectedUrl, { signal: AbortSignal.timeout(60_000) });
    if (!response.ok) throw new Error(`Cannot download ${asset.name}: ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length !== asset.size) throw new Error(`Unexpected download size: ${asset.name}`);
    writeFileSync(join(temporary, asset.name), bytes);
  }
  const verified = verifyRelease(temporary, process.cwd(), version);
  console.log(`Verified installer from embedded manifest: ${verified.digest}`);
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

for (const asset of published.assets.filter(item => redundant.includes(item.name))) {
  if (!Number.isSafeInteger(asset.id) || asset.id < 1) throw new Error('Invalid release asset ID');
  if (!dryRun) await request(`/releases/assets/${asset.id}`, 'DELETE');
  console.log(`${dryRun ? 'Would remove' : 'Removed'}: ${asset.name}`);
}
if (!dryRun) {
  const checked = await request(`/releases/${published.id}`);
  if (checked.assets.some(asset => redundant.includes(asset.name))
    || retained.some(asset => !checked.assets.some(item => item.id === asset.id && item.name === asset.name
      && item.size === asset.size && item.digest === asset.digest && item.state === 'uploaded'))) {
    throw new Error('Release cleanup verification failed');
  }
  console.log(`Cleaned release: ${checked.html_url}`);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `Removed redundant signature and checksum attachments from [${tag}](${checked.html_url}). Installer and update manifest verified and retained.\n`, 'utf8');
  }
}
