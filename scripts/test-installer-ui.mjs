import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { Window } from 'happy-dom';

const html = readFileSync(new URL('../installer/ui/index.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true, disableJavaScriptFileLoading: true, disableCSSFileLoading: true, enableImageFileLoading: false, navigation: { disableMainFrameNavigation: true, disableChildFrameNavigation: true } } });
let time = 0, id = 0, callbacks = new Map(), handler, resolveInstall, rejectInstall, unlistened = false;
const calls = [];
const context = { save() {}, restore() {}, beginPath() {}, closePath() {}, clip() {}, fill() {}, drawImage() {} };
for (const name of ['setTransform', 'clearRect', 'translate', 'scale', 'rotate', 'arc', 'lineTo']) context[name] = (...values) => values.forEach(value => { if (typeof value === 'number') assert.ok(Number.isFinite(value)); });
window.HTMLCanvasElement.prototype.getContext = () => context;
window.HTMLElement.prototype.getBoundingClientRect = () => ({ width: 700, height: 454, left: 0, top: 0 });
window.requestAnimationFrame = callback => { callbacks.set(++id, callback); return id; };
window.cancelAnimationFrame = identifier => callbacks.delete(identifier);
Object.defineProperty(window.performance, 'now', { value: () => time });
window.ResizeObserver = class { observe() {} disconnect() {} };
window.Image = class { naturalWidth = 1254; naturalHeight = 1254; set src(value) { assert.match(value, /^data:image\/png;base64,/); this.onload?.(); } };
window.matchMedia = () => ({ matches: false, addEventListener() {} });
window.__TAURI__ = {
  core: { invoke: async (command, args) => {
    calls.push({ command, args });
    if (command === 'installation_path') return 'D:\\Software\\SuperCode';
    if (command === 'choose_directory') return 'D:\\中文安装目录\\SuperCode';
    if (command === 'begin_install') return new Promise((resolve, reject) => { resolveInstall = resolve; rejectInstall = reject; });
  } },
  event: { listen: async (name, callback) => { assert.equal(name, 'installation-progress'); handler = callback; return () => { unlistened = true; }; } },
  window: { getCurrentWindow: () => ({ startDragging: async () => {} }) },
};
window.document.write(html.replace(/<script>[\s\S]*?<\/script>/, ''));
window.eval(script);
const el = id => window.document.getElementById(id);
const state = () => window.supercodeInstaller.getState();
const flush = async () => { for (let index = 0; index < 12; index++) await Promise.resolve(); };
const advance = delta => { time += delta; const pending = [...callbacks.values()]; callbacks.clear(); pending.forEach(callback => callback(time)); assert.ok(callbacks.size <= 1); };
const event = percent => handler({ payload: { percent, stage: 'extracting' } });
const checks = [];
const check = async (name, action) => { await action(); checks.push(name); };
try {
  await check('Native startup subscribes before enabling installation and never auto-installs', async () => {
    await flush(); assert.equal(state().phase, 'welcome'); assert.equal(state().pieceCount, 40);
    assert.equal(el('install-path').value, 'D:\\Software\\SuperCode'); assert.equal(el('action').disabled, false);
    assert.equal(calls.filter(c => c.command === 'show_installer').length, 1); assert.equal(calls.filter(c => c.command === 'begin_install').length, 0);
    advance(20000); el('mark').click(); event(80); assert.equal(state().progress, 0); assert.equal(el('progress-readout').hidden, true);
  });
  await check('Native folder selection supports Unicode paths', async () => { el('folder').click(); await flush(); assert.equal(el('install-path').value, 'D:\\中文安装目录\\SuperCode'); });
  await check('Install uses the chosen native path exactly once and disables conflicting controls', async () => {
    el('action').click(); el('action').click(); await flush(); assert.equal(state().phase, 'installing');
    assert.equal(calls.filter(c => c.command === 'begin_install').length, 1); assert.equal(calls.at(-1).args.path, 'D:\\中文安装目录\\SuperCode');
    for (const name of ['action', 'folder', 'replay', 'install-path']) assert.equal(el(name).disabled, true);
  });
  await check('Only measured native progress advances the number; elapsed time cannot advance installation', async () => {
    event(31); assert.equal(state().progress, 31); for (let i = 0; i < 30; i++) advance(64); assert.equal(state().progress, 31);
    event(4); event(NaN); assert.equal(state().progress, 31); assert.equal(state().phase, 'installing');
  });
  await check('A complete event cannot enable launch before the native install resolves', async () => {
    event(100); assert.equal(state().progress, 99); for (let i = 0; i < 40; i++) advance(64);
    assert.equal(state().phase, 'installing'); assert.equal(el('action').disabled, true); assert.equal(calls.filter(c => c.command === 'launch_installed').length, 0);
  });
  await check('Install failure exposes the error, returns to welcome, and permits retry', async () => {
    rejectInstall('文件被占用，请退出软件后重试。'); await flush(); assert.equal(state().phase, 'welcome'); assert.equal(el('error').hidden, false);
    assert.match(el('error').textContent, /文件被占用/); assert.equal(el('action').disabled, false); assert.equal(state().pendingFrames, 0);
  });
  await check('Success settles the logo and enables the real launch command', async () => {
    el('action').click(); await flush(); event(82); resolveInstall({ path: 'D:\\中文安装目录\\SuperCode', version: '0.1.0' }); await flush();
    assert.equal(state().progress, 100); for (let i = 0; i < 60; i++) advance(64);
    assert.equal(state().phase, 'complete'); assert.equal(state().pendingFrames, 0); assert.equal(el('action').disabled, false); assert.equal(el('install-path').readOnly, true);
    el('action').click(); await flush(); assert.equal(calls.filter(c => c.command === 'launch_installed').length, 1);
  });
  await check('The native UI includes no simulation timer and releases its subscription', async () => {
    assert.doesNotMatch(script, /duration\s*=|startPreview|launchPreview|setInterval|setTimeout/);
    assert.equal(window.document.querySelectorAll('iframe,script[src]').length, 0);
    window.dispatchEvent(new window.Event('pagehide')); assert.equal(unlistened, true); assert.equal(callbacks.size, 0);
  });
  console.log(`${checks.length} 安装器 IPC 与状态检查通过（模拟原生桥接与 Canvas，不替代原生视觉验证）。`);
} finally { await window.happyDOM.close(); }
