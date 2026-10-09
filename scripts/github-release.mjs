import { readFileSync, appendFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { verifyRelease } from './release-contract.mjs';
const root = process.cwd(), directory = resolve(process.argv[2]);
const tag = process.env.GITHUB_REF_NAME, sourceCommit = process.env.RELEASE_SOURCE_COMMIT;
if (!/^v\d+\.\d+\.\d+$/.test(tag ?? '') || !/^[a-f0-9]{40}$/.test(sourceCommit ?? '')) throw new Error('Invalid release tag or source commit');
const version = tag.slice(1);
const source = JSON.parse(readFileSync(join(directory, 'source.json'), 'utf8'));
const release = verifyRelease(directory, root, version);
if (source.version !== version || source.sourceCommit !== sourceCommit || source.sha256 !== release.digest) throw new Error('Assets do not belong to this source tag');
const repository = 'heyule69/SuperCode';
if (process.env.GITHUB_REPOSITORY !== repository) throw new Error('Release destination mismatch');
const token = process.env.GITHUB_TOKEN;
if (!token) throw new Error('GitHub Actions token is unavailable');
async function request(url, options={}) {
  if (!['api.github.com', 'uploads.github.com'].includes(new URL(url).host)) throw new Error('Unexpected GitHub API host');
  const response = await fetch(url, { ...options, headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28', ...options.headers } });
  if (!response.ok) throw new Error(`GitHub API returned ${response.status}: ${(await response.text()).slice(0, 500)}`);
  return response.status === 204 ? null : response.json();
}
const api = `https://api.github.com/repos/${repository}`;
let published;
const previous = await fetch(`${api}/releases/tags/${tag}`, { headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json' } });
if (previous.ok) { published = await previous.json(); if (!published.draft) throw new Error('This release is already published'); }
else if (previous.status === 404) {
  published = await request(`${api}/releases`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ tag_name: tag, target_commitish: sourceCommit, name: `SuperCode ${version}`, body: readFileSync(join(root, 'docs', 'releases', `${version}.md`), 'utf8'), draft: true, prerelease: false }) });
} else throw new Error(`Cannot inspect release: ${previous.status}`);
for (const name of release.assets) {
  const old = published.assets.find(asset => asset.name === name);
  if (old) await request(`${api}/releases/assets/${old.id}`, { method: 'DELETE' });
  await request(`${published.upload_url.split('{')[0]}?name=${encodeURIComponent(name)}`, {
    method: 'POST', headers: { 'Content-Type': name.endsWith('.exe') ? 'application/octet-stream' : name.endsWith('.json') ? 'application/json' : 'text/plain; charset=utf-8' }, body: readFileSync(join(directory, name)),
  });
}
const checked = await request(`${api}/releases/${published.id}`);
if (release.assets.some(name => !checked.assets.some(asset => asset.name === name && asset.state === 'uploaded'))) throw new Error('Release upload is incomplete');
published = await request(`${api}/releases/${published.id}`, { method: 'PATCH', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ draft: false, make_latest: 'true' }) });
console.log(`Published: ${published.html_url}`);
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `[SuperCode ${version}](${published.html_url})\n\nWindows x64: ${release.bytes} bytes. SHA256: \`${release.digest}\`\n`, 'utf8');
// This ref is release staging data only; deleting it does not remove a source branch or tag.
await request(`${api}/git/refs/heads/release-assets/${tag}`, { method: 'DELETE' });
