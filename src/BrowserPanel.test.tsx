// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import BrowserPanel, { browserAddress } from './BrowserPanel';
import { call } from './api';
const listeners=vi.hoisted(()=>new Map<string,(e:{payload:unknown})=>void>());
vi.mock('./api',()=>({desktop:true,call:vi.fn(async(command:string)=>command==='browser_open'?{sessionId:'s',url:'',title:'',status:'idle',action:''}:undefined)}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async(name:string,handler:(e:{payload:unknown})=>void)=>{listeners.set(name,handler);return()=>listeners.delete(name);})}));
let root:Root,node:HTMLDivElement;
const close=vi.fn();
beforeEach(()=>{(globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;vi.mocked(call).mockClear();close.mockClear();listeners.clear();node=document.createElement('div');document.body.append(node);root=createRoot(node);});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();});
async function render(visible=true){await act(async()=>{root.render(<BrowserPanel sessionId="s" visible={visible} close={close} resizeHandle={null} expanded={false} expand={()=>{}}/>);await new Promise(resolve=>setTimeout(resolve,30));});}
it('only accepts web addresses and excludes embedded credentials',()=>{expect(browserAddress('example.com')).toBe('https://example.com/');for(const url of ['javascript:alert(1)','file:///C:/secret','https://user:secret@example.com'])expect(()=>browserAddress(url)).toThrow();});
it('waits for native creation before sending layout, including a visibility change while opening',async()=>{
  let finish!:(value:unknown)=>void;
  const pending=new Promise<unknown>(resolve=>{finish=resolve;});
  vi.mocked(call).mockReturnValueOnce(pending);
  await render();await render(false);
  expect(call).toHaveBeenCalledWith('browser_open',expect.objectContaining({sessionId:'s'}));
  expect(vi.mocked(call).mock.calls.some(([command])=>command==='browser_layout')).toBe(false);
  await act(async()=>{finish({sessionId:'s',url:'about:blank',title:'新标签页',status:'idle',action:''});await pending;await new Promise(resolve=>setTimeout(resolve,30));});
  await new Promise(resolve=>setTimeout(resolve,30));
  expect(call).toHaveBeenCalledWith('browser_layout',expect.objectContaining({sessionId:'s',visible:false}));
  expect(node.querySelector('[role="alert"]')).toBeNull();
});
it('shows real operation state, stops on Esc and keeps other chats isolated',async()=>{
  await render();
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'other',status:'working',action:'OTHER'}}));expect(node.textContent).not.toContain('OTHER');
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'s',status:'working',action:'正在点击'}}));expect(node.textContent).toContain('正在点击');expect(node.textContent).toContain('Esc 接管');
  await act(async()=>window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
  expect(call).toHaveBeenCalledWith('browser_takeover',{sessionId:'s'});
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'s',status:'stopped',action:'已由你接管'}}));expect(node.textContent).toContain('你正在控制浏览器');expect(node.querySelector('.browser-takeover')).toBeNull();
});
it('hides the native surface behind app modals and closes only the panel while working',async()=>{
  await render();await render(false);await new Promise(resolve=>setTimeout(resolve,30));
  expect(call).toHaveBeenCalledWith('browser_layout',expect.objectContaining({sessionId:'s',visible:false,mountId:expect.any(String)}));
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'s',status:'working',action:'正在点击'}}));
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="关闭浏览器"]')!.click());expect(call).toHaveBeenCalledWith('browser_close',{sessionId:'s',mountId:expect.any(String)});expect(close).toHaveBeenCalledOnce();
  expect(call).not.toHaveBeenCalledWith('browser_takeover',expect.anything());
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
});
it('does not let a delayed close from an unmounted panel close its replacement',async()=>{
  await render();
  let finish!:(value:unknown)=>void;
  vi.mocked(call).mockReturnValueOnce(new Promise(resolve=>{finish=resolve;}));
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="关闭浏览器"]')!.click());
  await act(async()=>root.render(null));
  await render();
  await act(async()=>finish(undefined));
  expect(close).not.toHaveBeenCalled();
});
it('closes an opened native mount when the shared tab is removed without interrupting the agent',async()=>{
  await render();vi.mocked(call).mockClear();
  await act(async()=>root.render(null));
  expect(call).toHaveBeenCalledWith('browser_close',{sessionId:'s',mountId:expect.any(String)});
  expect(call).not.toHaveBeenCalledWith('browser_takeover',expect.anything());
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
});
it('releases its native mount if the tab was closed before opening finished',async()=>{
  let finish!:(value:unknown)=>void;
  const pending=new Promise(resolve=>{finish=resolve;});
  vi.mocked(call).mockReturnValueOnce(pending);
  await render();
  const mountId=vi.mocked(call).mock.calls.find(([command])=>command==='browser_open')?.[1]?.mountId;
  await act(async()=>root.render(null));
  await act(async()=>finish({sessionId:'s',url:'about:blank',title:'新标签页',status:'idle',action:''}));
  expect(call).toHaveBeenCalledWith('browser_close',{sessionId:'s',mountId});
  expect(close).not.toHaveBeenCalled();
});
it('does not intercept Escape or take over a working browser while another tab or menu is in front',async()=>{
  await render();
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'s',status:'working',action:'正在读取网页'}}));
  await render(false);vi.mocked(call).mockClear();
  await act(async()=>window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
  expect(call).not.toHaveBeenCalledWith('browser_takeover',expect.anything());
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
});
it('takes over only browser control and keeps navigation failures in the browser panel',async()=>{
  await render();
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'s',status:'working',action:'正在读取网页'}}));
  expect(node.querySelector('.browser-takeover')?.getAttribute('title')).toBe('停止浏览器操作，其他任务继续');
  await act(async()=>node.querySelector<HTMLButtonElement>('.browser-takeover')!.click());
  expect(call).toHaveBeenCalledWith('browser_takeover',{sessionId:'s'});
  vi.mocked(call).mockRejectedValueOnce(new Error('网页响应超时'));
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="刷新网页"]')!.click());
  expect(call).toHaveBeenCalledWith('browser_manual',{sessionId:'s',action:'reload',address:null});
  expect(node.querySelector('[role="alert"]')?.textContent).toContain('网页响应超时');
  expect(close).not.toHaveBeenCalled();
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
  expect(node.querySelector<HTMLButtonElement>('[aria-label="刷新网页"]')!.disabled).toBe(false);
});
