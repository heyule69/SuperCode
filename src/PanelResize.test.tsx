// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { decodePanelWidths, defaultWidths, panelLayout, usePanelResize } from './PanelResize';

let root: Root, container: HTMLDivElement;
let narrow = false;
function Layout({panel='workspace',left=true,right=true,expanded=false}:{panel?:'workspace'|'browser'|'conversation'|'content';left?:boolean;right?:boolean;expanded?:boolean}) {
  const panels = usePanelResize(left, right, false, panel, expanded);
  return <div ref={panels.root}><div {...panels.handle('left')}/><div {...panels.handle('right')}/></div>;
}
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); narrow = false;
  vi.spyOn(window, 'matchMedia').mockImplementation(() => ({ matches:narrow } as MediaQueryList));
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1400);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const handle = (side: string) => container.querySelector<HTMLDivElement>(`.${side}`)!;
async function pointer(side: string, type: string, x: number) {
  await act(async () => handle(side).dispatchEvent(new PointerEvent(type, { clientX:x, button:0, pointerId:1, bubbles:true })));
}
it('drags both boundaries in the correct direction and restores persisted widths', async () => {
  await act(async () => root.render(<Layout/>));
  await pointer('left', 'pointerdown', 288); await pointer('left', 'pointermove', 350); await pointer('left', 'pointerup', 350);
  await pointer('right', 'pointerdown', 1116); await pointer('right', 'pointermove', 1000); await pointer('right', 'pointerup', 1000);
  expect(JSON.parse(localStorage.getItem('supercode.panel-widths')!)).toEqual({ left:350, right:400 });
  expect(container.firstElementChild?.getAttribute('style')).toContain('350px minmax(0,1fr) 400px');
  await act(async () => root.unmount()); root = createRoot(container);
  await act(async () => root.render(<Layout/>));
  expect(handle('left').getAttribute('aria-valuenow')).toBe('350');
  expect(handle('right').getAttribute('aria-valuenow')).toBe('400');
});
it('cancels a drag, supports keyboard resizing and resets on double click', async () => {
  await act(async () => root.render(<Layout/>));
  await pointer('left', 'pointerdown', 288); await pointer('left', 'pointermove', 500); await pointer('left', 'pointercancel', 500);
  expect(handle('left').getAttribute('aria-valuenow')).toBe('288');
  await act(async () => handle('right').dispatchEvent(new KeyboardEvent('keydown', { key:'ArrowLeft', bubbles:true })));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('294');
  await act(async () => handle('right').dispatchEvent(new MouseEvent('dblclick', { bubbles:true })));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('284');
});
it('bounds saved values and preserves the chat at smaller window sizes', () => {
  expect(decodePanelWidths('bad')).toEqual(defaultWidths);
  expect(decodePanelWidths('{"left":5000,"right":-1}')).toEqual({ left:580, right:240 });
  const layout = panelLayout({ left:580, right:620 }, 1200, true, true);
  expect(1200 - layout.left - layout.right).toBeGreaterThanOrEqual(360);
  narrow = true;
  expect(panelLayout(defaultWidths, 900, true, true).overlay).toBe(true);
  expect(panelLayout({left:288,right:520}, 900, true, true).right).toBe(520);
  expect(panelLayout({left:288,right:620}, 600, true, true).right).toBe(568);
});
it('keeps browser widths separate while sharing the saved sidebar width', async()=>{
  await act(async()=>root.render(<Layout panel="browser"/>));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('520');
  await pointer('left','pointerdown',288);await pointer('left','pointermove',310);await pointer('left','pointerup',310);
  await pointer('right','pointerdown',880);await pointer('right','pointermove',840);await pointer('right','pointerup',840);
  expect(JSON.parse(localStorage.getItem('supercode.panel-widths')!)).toEqual({left:310,right:284});
  await act(async()=>root.render(<Layout/>));expect(handle('right').getAttribute('aria-valuenow')).toBe('284');
  await act(async()=>root.render(<Layout panel="browser"/>));expect(handle('right').getAttribute('aria-valuenow')).toBe('560');
});
it('gives conversation panes room to read without changing the file pane width', async () => {
  localStorage.setItem('supercode.panel-widths', JSON.stringify({left:310,right:280}));
  await act(async()=>root.render(<Layout panel="conversation"/>));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('520');
  expect(handle('right').getAttribute('aria-label')).toBe('调整对话面板宽度');
  await pointer('right','pointerdown',880);await pointer('right','pointermove',840);await pointer('right','pointerup',840);
  await act(async()=>root.render(<Layout/>));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('280');
  expect(handle('left').getAttribute('aria-valuenow')).toBe('310');
  await act(async()=>root.render(<Layout panel="conversation"/>));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('560');
  await act(async()=>handle('right').dispatchEvent(new MouseEvent('dblclick',{bubbles:true})));
  expect(handle('right').getAttribute('aria-valuenow')).toBe('520');
});
it('uses stable tracks to animate hidden panes and restores saved widths after expansion', async () => {
  await act(async () => root.render(<Layout left={false} right={false}/>));
  expect(container.firstElementChild?.getAttribute('style')).toContain('0px minmax(0,1fr) 0px');
  await act(async () => root.render(<Layout expanded/>));
  expect(container.firstElementChild?.getAttribute('style')).toContain('288px minmax(0,1fr) 752px');
  await act(async () => root.render(<Layout/>));
  expect(container.firstElementChild?.getAttribute('style')).toContain('288px minmax(0,1fr) 284px');
});
it('uses a shared reading width and reserves no column after the content dock closes',async()=>{
  await act(async()=>root.render(<Layout panel="content"/>));
  expect(handle('right').getAttribute('aria-label')).toBe('调整右侧栏宽度');
  expect(handle('right').getAttribute('aria-valuenow')).toBe('520');
  await act(async()=>root.render(<Layout panel="content" right={false}/>));
  expect(container.firstElementChild?.getAttribute('style')).toContain('288px minmax(0,1fr) 0px');
});
it('keeps a medium desktop split instead of overlaying content at the older workspace breakpoint',()=>{
  vi.spyOn(window,'matchMedia').mockImplementation(query=>({matches:query.includes('1100px')} as MediaQueryList));
  expect(panelLayout({left:288,right:520},1000,true,true,'content').overlay).toBe(false);
  expect(panelLayout({left:288,right:520},1000,true,true).overlay).toBe(true);
});
