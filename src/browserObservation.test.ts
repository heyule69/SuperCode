// @vitest-environment happy-dom
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

// Exercise the actual scripts embedded in WebView2, with geometry/hit testing stubbed.
const snapshotScript = readFileSync('src-tauri/src/browser_page.js', 'utf8');
const targetScript = readFileSync('src-tauri/src/browser_target.js', 'utf8');
const snapshot = () => new Function(`return (\n${snapshotScript}\n)`)();
const target = (ref = 'e1') => new Function('ref', `return (${targetScript})(ref)`)(ref);
const rect = (x = 20, y = 30, width = 100, height = 40) => new DOMRect(x, y, width, height);
let hit: Element | null;
beforeEach(() => {
  document.body.innerHTML = '<button id="download">下载图片</button>';
  hit = document.querySelector('button');
  vi.stubGlobal('innerWidth', 800); vi.stubGlobal('innerHeight', 600);
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(() => rect());
  vi.spyOn(document, 'elementFromPoint').mockImplementation(() => hit);
});
afterEach(() => {
  delete (globalThis as any).__supercodeBrowserObservation;
  vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.innerHTML = '';
});

it('follows the same control after a large lazy-image layout shift', () => {
  snapshot();
  vi.spyOn(hit!, 'getBoundingClientRect').mockReturnValue(rect(260, 290, 140, 55));
  expect(target()).toMatchObject({ status: 'ready', x: 330, y: 317.5, generation: 0 });
});

it('does not invalidate the target when unrelated page content changes', () => {
  snapshot(); document.body.append(document.createElement('img'));
  expect(target().status).toBe('ready');
});

it('rebinds a uniquely identified replacement and changes its generation', () => {
  snapshot();
  const replacement = hit!.cloneNode(true) as Element;
  hit!.replaceWith(replacement); hit = replacement;
  expect(target()).toMatchObject({ status: 'ready', generation: 1 });
  expect(target().generation).toBe(1);
});

it('refuses to guess between duplicate replacement targets', () => {
  snapshot(); hit!.remove();
  document.body.innerHTML = '<button id="download">下载图片</button><button id="download">下载图片</button>';
  expect(target().status).toBe('ambiguous');
});

it('does not rebind a removed button using its text alone', () => {
  hit!.removeAttribute('id'); snapshot();
  const replacement = hit!.cloneNode(true) as Element;
  hit!.replaceWith(replacement); hit = replacement;
  expect(target().status).toBe('missing');
});

it('refuses a recycled node whose destination changed but label did not', () => {
  document.body.innerHTML = '<a href="/image-one">查看图片</a>'; hit = document.querySelector('a');
  snapshot(); hit!.setAttribute('href', '/image-two');
  expect(target().status).toBe('changed');
});

it('distinguishes a full label even when its public name is truncated', () => {
  hit!.textContent = '图'.repeat(260) + '甲'; snapshot();
  hit!.textContent = '图'.repeat(260) + '乙';
  expect(target().status).toBe('changed');
});

it('rejects virtual-list item recycling even with the same visible label', () => {
  hit!.setAttribute('data-id', 'rabbit-one'); snapshot();
  hit!.setAttribute('data-id', 'rabbit-two'); expect(target().status).toBe('changed');
});

it('bounds stored identities and never puts input values in observations', () => {
  document.body.innerHTML = '<button>' + '长'.repeat(9000) + '</button><input aria-label="密码" type="password" value="PRIVATE">';
  const value = snapshot(); expect(value.elements).toHaveLength(1);
  expect(JSON.stringify(value)).not.toContain('PRIVATE');
  const identities = [...(globalThis as any).__supercodeBrowserObservation.refs.values()];
  expect(JSON.stringify(identities.map(v => v.identity))).not.toContain('PRIVATE');
});

it('rejects a target moved to a different form', () => {
  document.body.innerHTML = '<form id="first" action="/one"><input name="text" aria-label="搜索"></form><form id="second" action="/two"></form>';
  hit = document.querySelector('input'); snapshot(); document.querySelector('#second')!.append(hit!);
  expect(target().status).toBe('changed');
});

it('rejects an observation after a same-document navigation', () => {
  snapshot(); (globalThis as any).__supercodeBrowserObservation.url += '#new';
  expect(target().status).toBe('changed');
});

it('reports a temporary overlay and becomes ready after it disappears', () => {
  snapshot(); const control = hit;
  hit = document.createElement('div'); document.body.append(hit);
  expect(target().status).toBe('blocked'); hit.remove(); hit = control;
  expect(target().status).toBe('ready');
});

it('does not dispatch events while resolving a target', () => {
  const click = vi.fn(); hit!.addEventListener('click', click); snapshot();
  target(); target(); expect(click).not.toHaveBeenCalled();
});

it('uses the center of the visible part and rejects an offscreen target', () => {
  snapshot(); vi.spyOn(hit!, 'getBoundingClientRect').mockReturnValue(rect(-70, 30, 100, 40));
  expect(target().x).toBe(15);
  vi.spyOn(hit!, 'getBoundingClientRect').mockReturnValue(rect(801, 30));
  expect(target().status).toBe('hidden');
});

it('honors native disabled and inherited ARIA disabled states', () => {
  hit!.setAttribute('aria-disabled', 'true'); snapshot(); expect(target().status).toBe('disabled');
  hit!.removeAttribute('aria-disabled'); expect(target().status).toBe('ready');
  (hit as HTMLButtonElement).disabled = true; expect(target().status).toBe('disabled');
});

it('tracks editable focus and allows clicks on readonly fields', () => {
  document.body.innerHTML = '<input id="field" aria-label="搜索" readonly>'; hit = document.querySelector('input');
  snapshot(); expect(target()).toMatchObject({ status: 'ready', editable: false, focused: false });
  (hit as HTMLInputElement).readOnly = false; (hit as HTMLInputElement).focus();
  expect(target()).toMatchObject({ status: 'ready', editable: true, focused: true });
});

it('checks an overlay outside a shadow root', () => {
  document.body.innerHTML = '<div id="host"></div>';
  const host = document.querySelector('#host')!, shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<button id="download">下载图片</button>';
  const control = shadow.querySelector('button')!; hit = host;
  Object.defineProperty(shadow, 'elementFromPoint', { value: () => control });
  snapshot(); expect(target().status).toBe('ready');
  hit = document.createElement('div'); document.body.append(hit); expect(target().status).toBe('blocked');
});

it('reports only currently displayed HTTP images with bounded data', () => {
  document.body.innerHTML = '<img alt="兔子" src="https://example.com/rabbit.jpg"><img src="data:image/png;base64,AA"><img src="blob:https://example.com/a"><img src="file:///C:/private.png">';
  const image = document.querySelector('img')!;
  Object.defineProperty(image, 'currentSrc', { value: 'https://example.com/rabbit-small.jpg' });
  const value = snapshot();
  expect(value.images).toEqual([expect.objectContaining({ url: 'https://example.com/rabbit-small.jpg', alt: '兔子', width: 100, height: 40 })]);
  expect(JSON.stringify(value)).not.toContain('file:///');
});

it('bounds image lists and still collects images after the interactive cap', () => {
  document.body.innerHTML = '<button>button</button>'.repeat(220) + '<img src="https://example.com/a.png">'.repeat(100);
  const value = snapshot();
  expect(value.elements).toHaveLength(200); expect(value.images).toHaveLength(80); expect(value.truncated).toBe(true);
});
