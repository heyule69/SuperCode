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
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'s',status:'working',action:'正在点击'}}));expect(node.textContent).toContain('正在点击');expect(node.textContent).toContain('Esc 停止');
  await act(async()=>window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
  expect(call).toHaveBeenCalledWith('browser_takeover',{sessionId:'s'});
  await act(async()=>listeners.get('browser-state')?.({payload:{sessionId:'s',status:'stopped',action:'已由你接管'}}));expect(node.textContent).toContain('你正在控制浏览器');expect(node.querySelector('.browser-takeover')).toBeNull();
});
it('hides the native surface behind app modals and releases it on close',async()=>{
  await render();await render(false);await new Promise(resolve=>setTimeout(resolve,30));
  expect(call).toHaveBeenCalledWith('browser_layout',expect.objectContaining({sessionId:'s',visible:false,mountId:expect.any(String)}));
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="关闭浏览器"]')!.click());expect(call).toHaveBeenCalledWith('browser_close',{sessionId:'s'});expect(close).toHaveBeenCalledOnce();
});
