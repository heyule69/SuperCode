// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import AgentSettings from './AgentSettings';
import type { AgentRelease } from './agentVersions';
const mocks=vi.hoisted(()=>({call:vi.fn(),listen:vi.fn()}));
vi.mock('./api',()=>({desktop:true,call:mocks.call}));
vi.mock('@tauri-apps/api/event',()=>({listen:mocks.listen}));
let root:Root,node:HTMLDivElement;
const agents=[{id:'codex',name:'Codex',installed:true,path:'codex.exe',connected:true,version:'codex-cli 0.160.1',phase:'ready'}, {id:'pi',name:'Pi',installed:false,path:null,connected:true,phase:'missing'}];
const releases:AgentRelease[]=[{id:'codex',latestVersion:'0.162.0',checkedAt:1,error:null},{id:'pi',latestVersion:'1.0.4',checkedAt:1,error:null}];
let releasePromise:Promise<AgentRelease[]>;
beforeEach(()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
  node=document.createElement('div');document.body.append(node);root=createRoot(node);
  releasePromise=Promise.resolve(releases);mocks.call.mockReset();mocks.listen.mockReset();
  mocks.listen.mockResolvedValue(()=>{});
  mocks.call.mockImplementation(async(command:string)=>command==='check_agent_updates'?releasePromise:agents);
});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();});
async function render(busy=false){await act(async()=>root.render(<AgentSettings agents={agents} busy={busy} loadMcp={false} setLoadMcp={()=>{}} saveMcp={async()=>{}}/>));}
const row=(name:string)=>node.querySelector<HTMLElement>(`section[aria-label="${name}"]`)!;
const button=(within:HTMLElement,text:string)=>[...within.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent===text)!;
it('shows latest versions and pins update/install clicks to the displayed release',async()=>{
  await render();expect(row('Codex').textContent).toContain('最新版本 0.162.0');expect(row('Codex').textContent).toContain('可更新');
  await act(async()=>button(row('Codex'),'更新').click());expect(mocks.call).toHaveBeenCalledWith('update_agent',{id:'codex',version:'0.162.0'});
  await act(async()=>button(row('Pi'),'安装').click());expect(mocks.call).toHaveBeenCalledWith('install_agent',{id:'pi',repair:false,version:'1.0.4'});
});
it('does not block local use while a network check is pending',async()=>{
  let finish!:(v:AgentRelease[])=>void;releasePromise=new Promise(resolve=>{finish=resolve;});
  await render();expect(row('Codex').textContent).toContain('codex-cli 0.160.1');expect(button(row('Codex'),'测试连接').disabled).toBe(false);expect(button(row('Pi'),'安装').disabled).toBe(true);
  await act(async()=>finish(releases));expect(button(row('Codex'),'更新').disabled).toBe(false);
});
it('keeps local testing available after a failed check and retries explicitly',async()=>{
  releasePromise=Promise.resolve(releases.map(r=>({...r,latestVersion:null,error:'离线'})));await render();
  expect(row('Codex').textContent).toContain('可使用');expect(row('Codex').textContent).not.toContain('已是最新');expect(button(row('Codex'),'测试连接').disabled).toBe(false);
  await act(async()=>button(row('Codex'),'重试').click());expect(mocks.call).toHaveBeenCalledWith('check_agent_updates',{force:true});
});
it('does not offer a downgrade and blocks installation during an active task',async()=>{
  releasePromise=Promise.resolve([{...releases[0],latestVersion:'0.159.0'},releases[1]]);await render(true);
  expect(row('Codex').textContent).toContain('本机版本更新');expect(button(row('Codex'),'更新')).toBeUndefined();expect(button(row('Pi'),'安装').disabled).toBe(true);
});
it('keeps the old usable version after a rejected update',async()=>{
  mocks.call.mockImplementation(async(command:string)=>{
    if(command==='check_agent_updates')return releases;
    if(command==='update_agent')throw new Error('更新失败');
    return [{...agents[0],phase:'updateFailed',message:'更新未完成，仍使用原版本'},agents[1]];
  });
  await render();await act(async()=>button(row('Codex'),'更新').click());
  expect(row('Codex').textContent).toContain('可使用');expect(row('Codex').textContent).toContain('0.160.1');expect(button(row('Codex'),'更新').disabled).toBe(false);
});
