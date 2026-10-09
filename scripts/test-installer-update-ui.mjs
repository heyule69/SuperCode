import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Window } from 'happy-dom';
const html = readFileSync(new URL('../installer/ui/index.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
const window = new Window({ url: 'http://tauri.localhost' });
const calls = [], frames = new Map(), timers = [];
let id = 0, time = 0, finish, progress;
const context = new Proxy({}, { get: () => () => {} });
window.HTMLCanvasElement.prototype.getContext = () => context;
window.HTMLElement.prototype.getBoundingClientRect = () => ({ width: 700, height: 454, left: 0, top: 0 });
window.requestAnimationFrame = callback => { frames.set(++id, callback); return id; };
window.cancelAnimationFrame = id => frames.delete(id);
window.setTimeout = callback => { timers.push(callback); return timers.length; };
Object.defineProperty(window.performance, 'now', { value: () => time });
window.ResizeObserver = class { observe() {} disconnect() {} };
window.Image = class { naturalWidth=1254; naturalHeight=1254; set src(value) { this.onload?.(); } };
window.matchMedia = () => ({ matches: false, addEventListener() {} });
window.__TAURI__ = {
  core: { invoke: async (command, args) => {
    calls.push({ command, args });
    if (command === 'installation_path') return 'D:\\已安装\\SuperCode';
    if (command === 'show_installer') return true;
    if (command === 'begin_install') return new Promise(resolve => { finish=resolve; });
  } },
  event: { listen: async (_, callback) => { progress=callback; return () => {}; } },
  window: { getCurrentWindow: () => ({ startDragging: async () => {} }) },
};
window.document.write(html.replace(/<script>[\s\S]*?<\/script>/, ''));
window.eval(script);
const flush = async () => { for(let i=0;i<15;i++) await Promise.resolve(); };
try {
  await flush(); assert.equal(window.supercodeInstaller.getState().phase, 'installing');
  assert.equal(calls.filter(c => c.command === 'begin_install').length, 1);
  assert.equal(calls.find(c => c.command === 'begin_install').args.path, 'D:\\已安装\\SuperCode');
  assert.equal(window.document.getElementById('folder').disabled, true);
  progress({payload:{percent:100}}); await flush(); assert.equal(timers.length,0);
  finish({path:'D:\\已安装\\SuperCode', version:'0.1.0'}); await flush();
  for(let i=0;i<100;i++) { time+=100; const pending=[...frames.values()]; frames.clear(); pending.forEach(callback => callback(time)); }
  assert.equal(window.supercodeInstaller.getState().phase,'complete'); assert.equal(timers.length,1);
  timers[0](); await flush(); assert.equal(calls.filter(c => c.command === 'launch_installed').length,1);
  console.log('自动更新 UI 状态验证通过：锁定原目录、实际完成后重启（模拟 IPC）。');
} finally { await window.happyDOM.close(); }
