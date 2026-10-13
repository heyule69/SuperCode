import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Unit tests get their own build inputs. Never create or overwrite release
// payloads in installer/generated, installer/ui, or release.
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
if (process.platform !== 'win32') throw new Error('These installer tests require Windows.');
const testBase = join(root, '.supercode');
mkdirSync(testBase, { recursive: true });
const outside = relative(realpathSync(root), realpathSync(testBase));
if (outside.startsWith('..') || isAbsolute(outside)) throw new Error('Test directory escapes the workspace.');
const stage = mkdtempSync(join(testBase, 'installer-unit-tests-'));
const textDecoder = new TextDecoder('utf-8', { fatal: true });
function readText(path) {
  const bytes = readFileSync(path);
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) throw new Error(`UTF-8 BOM: ${path}`);
  return textDecoder.decode(bytes);
}
function writeText(path, text) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text, 'utf8');
}
function copyText(path) { writeText(join(stage, path), readText(join(root, path))); }
function copyTextDirectory(path) {
  for (const entry of readdirSync(join(root, path), { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) copyTextDirectory(child);
    else if (entry.isFile()) copyText(child);
    else throw new Error(`Unexpected linked source: ${child}`);
  }
}
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', windowsHide: true, shell: false, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited ${result.status}`);
}
function hash(bytes, algorithm = 'sha256') { return createHash(algorithm).update(bytes).digest('hex'); }

copyTextDirectory('installer/src');
copyTextDirectory('installer/capabilities');
for (const path of ['installer/Cargo.toml', 'installer/Cargo.lock', 'installer/build.rs', 'installer/tauri.conf.json', 'installer/registration.nsi', 'src-tauri/src/installation_exit_protocol.rs']) copyText(path);
for (const file of ['icon.ico', '32x32.png', '128x128.png']) {
  const path = join('src-tauri', 'icons', file);
  mkdirSync(dirname(join(stage, path)), { recursive: true });
  copyFileSync(join(root, path), join(stage, path));
}
const version = JSON.parse(readText(join(root, 'package.json'))).version;
const logo = `data:image/png;base64,${readFileSync(join(root, 'resources/branding/logo.png')).toString('base64')}`;
const ui = readText(join(root, 'installer/ui-template.html'));
if (ui.split('__LOGO_DATA__').length !== 2) throw new Error('Installer logo placeholder changed.');
writeText(join(stage, 'installer/ui/index.html'), ui.replace('__LOGO_DATA__', logo));

const nsisCandidates = [process.env.SUPERCODE_NSIS, process.env.LOCALAPPDATA && join(process.env.LOCALAPPDATA, 'tauri/NSIS/makensis.exe'), process.env['ProgramFiles(x86)'] && join(process.env['ProgramFiles(x86)'], 'NSIS/makensis.exe')].filter(Boolean);
let nsis = process.argv.includes('--download-nsis') ? undefined : nsisCandidates.find(existsSync);
if (!nsis) {
  // Same pinned archive and digest as the official Tauri NSIS toolchain:
  // https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs
  const url = 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.13/nsis-3.13.zip';
  const response = await fetch(url, { signal: AbortSignal.timeout(60000) });
  if (!response.ok) throw new Error(`NSIS download returned HTTP ${response.status}`);
  const chunks = [];
  let length = 0;
  for await (const chunk of response.body) {
    length += chunk.length;
    if (length > 64 * 1024 * 1024) throw new Error('NSIS archive exceeds the download limit.');
    chunks.push(chunk);
  }
  const archive = Buffer.concat(chunks);
  if (hash(archive, 'sha1') !== 'db14f8af2ea346b786c6ae19343f61aaf5e09b8e') throw new Error('NSIS archive digest mismatch.');
  const zip = join(stage, 'nsis.zip');
  writeFileSync(zip, archive);
  run('python', ['-X', 'utf8', '-c', 'import pathlib,stat,sys,zipfile; root=pathlib.Path(sys.argv[2]).resolve(); z=zipfile.ZipFile(sys.argv[1]); entries=z.infolist(); assert all(not pathlib.PurePosixPath(e.filename.replace(chr(92),"/")).is_absolute() and ".." not in pathlib.PurePosixPath(e.filename.replace(chr(92),"/")).parts and not stat.S_ISLNK(e.external_attr >> 16) and pathlib.Path(root/e.filename).resolve().is_relative_to(root) for e in entries); z.extractall(root)', zip, join(stage, 'tools')]);
  nsis = join(stage, 'tools/nsis-3.13/makensis.exe');
  if (!existsSync(nsis)) throw new Error('Verified NSIS archive lacks makensis.exe.');
}

const generated = join(stage, 'installer/generated');
mkdirSync(generated, { recursive: true });
const icon = join(stage, 'src-tauri/icons/icon.ico');
const iconName = `supercode-icon-${hash(readFileSync(icon)).slice(0, 12)}.ico`;
const worker = join(generated, 'registration.exe');
run(nsis, ['/INPUTCHARSET', 'UTF8', '/OUTPUTCHARSET', 'UTF8', `/DOUTPUT=${worker}`, `/DICON_SOURCE=${icon}`, `/DICON_NAME=${iconName}`, `/DVERSION=${version}`, '/DSIZE_KB=1', join(stage, 'installer/registration.nsi')]);

// This deliberately non-executable byte fixture is never a release app and is
// never started. The registration worker is compiled from the real NSIS source.
writeFileSync(join(generated, 'supercode.exe'), Buffer.from('SUPERCODE INSTALLER UNIT TEST FIXTURE; NOT AN EXECUTABLE', 'utf8'));
copyFileSync(icon, join(generated, iconName));
const entries = ['supercode.exe', iconName, 'registration.exe'].map(name => {
  const bytes = readFileSync(join(generated, name));
  return { name, bytes: bytes.length, sha256: hash(bytes) };
});
writeText(join(generated, 'manifest.json'), `${JSON.stringify({ version, testFixture: true, entries }, null, 2)}\n`);
run('python', ['-X', 'utf8', '-c', 'import json,pathlib,sys,zipfile; p=pathlib.Path(sys.argv[1]); entries=json.loads((p/"manifest.json").read_text(encoding="utf-8"))["entries"]; z=zipfile.ZipFile(p/"payload.zip","w",compression=zipfile.ZIP_DEFLATED); [z.write(p/e["name"],e["name"]) for e in entries]; z.close()', generated]);
writeText(join(stage, 'test-inputs.json'), `${JSON.stringify({ testFixture: true, source: root, stage, nsis, productionInputsChanged: false }, null, 2)}\n`);
console.log(`Isolated installer unit-test inputs: ${stage}`);
run('cargo', ['test', '--locked', '--manifest-path', join(stage, 'installer/Cargo.toml'), '--target-dir', join(root, 'src-tauri/target')]);
