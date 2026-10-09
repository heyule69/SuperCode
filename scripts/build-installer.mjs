import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync, statSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { verifyRelease } from './release-contract.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const generated = join(root, 'installer', 'generated');
const output = join(root, 'release');
const prepareOnly = process.argv.includes('--prepare-only');
const reuseApp = process.argv.includes('--reuse-app');
const utf8 = new TextDecoder('utf-8', { fatal: true });
function readText(path) {
  const bytes = readFileSync(path);
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) throw new Error(`UTF-8 BOM: ${path}`);
  return utf8.decode(bytes);
}
function run(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, stdio: 'inherit', windowsHide: true, shell: false });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited ${result.status}`);
}
function hash(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
const nsis = process.env.SUPERCODE_NSIS || join(process.env.LOCALAPPDATA, 'tauri', 'NSIS', 'makensis.exe');
if (!existsSync(nsis)) throw new Error('NSIS 未找到。先运行 npm run desktop:build，或用 SUPERCODE_NSIS 指定 makensis.exe。');
if (process.platform !== 'win32') throw new Error('请在 Windows 上生成此安装器。');
mkdirSync(generated, { recursive: true }); mkdirSync(output, { recursive: true });
if (!reuseApp && !prepareOnly) {
  // Calling Node directly avoids a shell and allows spaces in the repository path.
  run(process.execPath, [join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), 'build', '--no-bundle']);
}
const app = join(root, 'src-tauri', 'target', 'release', 'supercode.exe');
if (!existsSync(app)) throw new Error('请先编译 SuperCode 的 release 主程序。');
const version = JSON.parse(readText(join(root, 'package.json'))).version;
const ico = join(root, 'src-tauri', 'icons', 'icon.ico');
const iconName = `supercode-icon-${hash(readFileSync(ico)).slice(0, 12)}.ico`;
const worker = join(generated, 'registration.exe');
run(nsis, ['/INPUTCHARSET', 'UTF8', `/DOUTPUT=${worker}`, `/DICON_SOURCE=${ico}`, `/DICON_NAME=${iconName}`, `/DVERSION=${version}`, `/DSIZE_KB=${Math.ceil(statSync(app).size / 1024)}`, join(root, 'installer', 'registration.nsi')]);
const entries = [{ name: 'supercode.exe', source: app }, { name: iconName, source: ico }, { name: 'registration.exe', source: worker }].map(entry => {
  const bytes = readFileSync(entry.source);
  return { ...entry, bytes: bytes.length, sha256: hash(bytes) };
});
writeFileSync(join(generated, 'manifest.json'), `${JSON.stringify({ version, entries: entries.map(({ source, ...entry }) => entry) }, null, 2)}\n`, 'utf8');
writeFileSync(join(generated, 'payload-sources.json'), JSON.stringify(entries, null, 2), 'utf8');
// Python's standard zipfile streams files; no platform-default text encoding is used.
run('python', ['-X', 'utf8', '-c', 'import json,pathlib,sys,zipfile; p=pathlib.Path(sys.argv[1]); entries=json.loads((p/"payload-sources.json").read_text(encoding="utf-8")); z=zipfile.ZipFile(p/"payload.zip","w",compression=zipfile.ZIP_DEFLATED,compresslevel=9); [z.write(e["source"],e["name"]) for e in entries]; z.close()', generated]);
const template = readText(join(root, 'installer', 'ui-template.html'));
if (template.split('__LOGO_DATA__').length !== 2) throw new Error('Logo 模板占位符不匹配。');
const logo = `data:image/png;base64,${readFileSync(join(root, 'resources', 'branding', 'supercode-mark-transparent-v1.png')).toString('base64')}`;
mkdirSync(join(root, 'installer', 'ui'), { recursive: true });
writeFileSync(join(root, 'installer', 'ui', 'index.html'), template.replace('__LOGO_DATA__', logo), 'utf8');
if (!prepareOnly) {
  run('cargo', ['build', '--release', '--locked', '--manifest-path', join(root, 'installer', 'Cargo.toml'), '--target-dir', join(root, 'src-tauri', 'target')]);
  const setup = join(output, `SuperCode_${version}_x64-setup.exe`);
  run(nsis, ['/INPUTCHARSET', 'UTF8', `/DOUTPUT=${setup}`, `/DICON_SOURCE=${ico}`, `/DGUI_SOURCE=${join(root, 'src-tauri', 'target', 'release', 'supercode-installer.exe')}`, `/DVERSION=${version}`, join(root, 'installer', 'bootstrapper.nsi')]);
  const metadata = { version, setup: setup.slice(root.length + 1), bytes: statSync(setup).size, sha256: hash(readFileSync(setup)), appSha256: entries[0].sha256, iconName, builtAt: new Date().toISOString() };
  writeFileSync(join(output, 'installer-build.json'), `${JSON.stringify(metadata, null, 2)}\n`, 'utf8');
  writeFileSync(`${setup}.sha256`, `${metadata.sha256}  SuperCode_${version}_x64-setup.exe\n`, 'utf8');
  const signingKey = process.env.TAURI_SIGNING_PRIVATE_KEY_PATH || join(process.env.USERPROFILE, '.supercode-signing', 'update.key');
  if (existsSync(signingKey)) {
    run(process.execPath, [join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), 'signer', 'sign', '-f', signingKey, '-p', process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '', '--app-version', version, setup]);
    const notesPath = join(root, 'docs', 'releases', `${version}.md`);
    const latest = { version, notes: existsSync(notesPath) ? readText(notesPath).trim() : `SuperCode ${version}`, pub_date: new Date().toISOString(), platforms: {
      'windows-x86_64': { url: `https://github.com/heyule69/SuperCode/releases/download/v${version}/SuperCode_${version}_x64-setup.exe`, signature: readText(`${setup}.sig`).trim(), sha256: metadata.sha256, size: metadata.bytes },
    } };
    writeFileSync(join(output, 'latest.json'), `${JSON.stringify(latest, null, 2)}\n`, 'utf8');
    verifyRelease(output, root, version);
  } else {
    console.log('未配置更新签名私钥，仅生成本地安装包；发布前需要设置 TAURI_SIGNING_PRIVATE_KEY_PATH。');
  }
  console.log(`\n安装包：${setup}\nSHA256：${metadata.sha256}`);
}
