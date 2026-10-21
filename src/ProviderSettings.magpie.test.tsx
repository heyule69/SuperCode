// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ProviderSettings from './ProviderSettings';
import type { AgentProfile } from './types';
const mocks=vi.hoisted(()=>({call:vi.fn()}));
vi.mock('./api',()=>({desktop:true,call:mocks.call}));
let root:Root,node:HTMLDivElement;
const existing:AgentProfile={id:'custom',agent:'claude',name:'原来的连接',model:'saved',hasCredential:true,current:true};
const models=Array.from({length:5000},(_,i)=>`vendor/model-${String(i).padStart(4,'0')}`);
const managed=()=>['claude','codex','opencode','pi','gemini'].map(agent=>({id:`magpie:${agent}`,agent,name:'Magpie',model:models[0],models,hasCredential:true,current:true,providerId:'magpie',source:'magpie'} satisfies AgentProfile));
const menu=()=>document.querySelector<HTMLElement>('[aria-label="Magpie 操作"]')!;
const menuButton=(label:string)=>[...menu().querySelectorAll<HTMLButtonElement>('button')].find(b=>b.querySelector('strong')?.textContent===label)!;
const trigger=()=>node.querySelector<HTMLButtonElement>('button[aria-label="Magpie"]')!;
async function render(profiles:AgentProfile[]=[existing],updated: (agent?:string,prefer?:boolean)=>Promise<void>=async()=>{},runningConnections:Record<string,string[]>={}){await act(async()=>root.render(<ProviderSettings profiles={profiles} officialAgents={[]} runningConnections={runningConnections} updated={updated}/>))}
beforeEach(()=>{(globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;node=document.createElement('div');document.body.append(node);root=createRoot(node);mocks.call.mockReset();mocks.call.mockImplementation(async(command:string)=>command==='list_provider_accounts'?[]:undefined)});
afterEach(async()=>{await act(async()=>root.unmount());node.remove()});
it('places Magpie in the toolbar and removes its API preset from every Agent picker',async()=>{
 await render();expect(trigger()).toBeTruthy();
 for(const agent of ['claude','codex','opencode','pi','gemini']){
  await act(async()=>node.querySelector<HTMLButtonElement>(`#providers-${agent}-tab`)!.click());
  await act(async()=>[...node.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent==='添加')!.click());
  expect([...node.querySelectorAll('.provider-preset strong')].map(n=>n.textContent)).not.toContain('Magpie');
 }
 expect(mocks.call.mock.calls.some(([command])=>command==='get_provider_profile')).toBe(false);
});
it('only calls native actions on a user click and restores keyboard focus after Escape',async()=>{
 await render();expect(mocks.call.mock.calls.map(c=>c[0])).toEqual(['list_provider_accounts']);
 await act(async()=>trigger().click());expect(menu().querySelectorAll('[role="menuitem"]')).toHaveLength(3);
 expect(document.activeElement).toBe(menu().querySelector('a'));
 await act(async()=>menu().dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true})));expect(document.activeElement).toBe(menuButton('打开 Magpie'));
 await act(async()=>menu().dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));expect(menu()).toBeNull();expect(document.activeElement).toBe(trigger());
 await act(async()=>trigger().click());await act(async()=>menuButton('打开 Magpie').click());expect(mocks.call).toHaveBeenCalledWith('open_magpie');
 await act(async()=>trigger().click());await act(async()=>menu().querySelector('a')!.click());expect(mocks.call).toHaveBeenCalledWith('open_external_link',{url:'https://github.com/yetone/magpie-releases/releases/latest'});
});
it('adds all managed routes without any API form and keeps the full large catalog out of the settings DOM',async()=>{
 const all=managed();const updated=vi.fn(async()=>{});mocks.call.mockImplementation(async(command:string)=>command==='list_provider_accounts'?[]:command==='use_magpie'?{models:5000,added:5}:undefined);
 await render([existing],updated);await act(async()=>trigger().click());await act(async()=>menuButton('使用 Magpie').click());
 expect(mocks.call).toHaveBeenCalledWith('use_magpie');expect(updated).toHaveBeenCalledWith('claude',true);
 await render([existing,...all],updated);
 const row=node.querySelector<HTMLElement>('[data-managed="magpie"]')!;expect(row.textContent).toContain('5000 个模型');expect(row.textContent).toContain('Magpie 管理');
 expect(row.querySelectorAll('button')).toHaveLength(1);expect(row.querySelector('button')?.textContent).toBe('移除');expect(row.querySelectorAll('input')).toHaveLength(0);
 expect(node.querySelector('#provider-url')).toBeNull();expect(node.querySelector('#provider-key')).toBeNull();expect(node.querySelectorAll('.provider-model-row')).toHaveLength(0);
 expect(mocks.call.mock.calls.some(([c])=>['save_provider_profile','fetch_provider_models_for_settings','get_provider_profile'].includes(c))).toBe(false);
 await act(async()=>trigger().click());expect(menuButton('已添加到 SuperCode').disabled).toBe(true);
});
it('removes the managed integration once and leaves ordinary suppliers alone',async()=>{
 const updated=vi.fn(async()=>{});await render([existing,...managed()],updated);
 expect(node.querySelectorAll('.provider-row-select')).toHaveLength(1);
 await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="移除 Magpie"]')!.click());
 expect(mocks.call).toHaveBeenCalledWith('remove_magpie');expect(mocks.call.mock.calls.some(([c])=>c==='delete_provider_profiles')).toBe(false);expect(updated).toHaveBeenCalled();
 expect(node.textContent).toContain('聊天记录保留');
});
it('protects the entire integration when another Agent is using its managed connection',async()=>{
 await render([existing,...managed()],async()=>{},{gemini:['magpie:gemini']});
 const remove=node.querySelector<HTMLButtonElement>('[aria-label="移除 Magpie"]')!;expect(remove.disabled).toBe(true);expect(node.textContent).toContain('任务使用中');
 await act(async()=>remove.click());expect(mocks.call.mock.calls.some(([c])=>c==='remove_magpie')).toBe(false);
});
it('shows an old Magpie profile as read-only but removes only that legacy route',async()=>{
 const legacy={...managed()[0],id:'legacy-gateway',source:'manual',name:'旧 Magpie'};await render([existing,legacy]);
 const row=node.querySelector<HTMLElement>('[data-managed="magpie"]')!;expect(row.querySelector('.saved-provider')?.tagName).toBe('DIV');expect(row.querySelector('[aria-label*="上移"]')).toBeNull();
 await act(async()=>row.querySelector<HTMLButtonElement>('button')!.click());expect(mocks.call).toHaveBeenCalledWith('delete_provider_profiles',{agent:'claude',ids:['legacy-gateway']});
});
it('keeps a failed add retryable without opening an API editor or changing suppliers',async()=>{
 mocks.call.mockImplementation(async(command:string)=>{if(command==='use_magpie')throw Error('请先打开 Magpie');return command==='list_provider_accounts'?[]:undefined});const updated=vi.fn(async()=>{});
 await render([existing],updated);await act(async()=>trigger().click());await act(async()=>menuButton('使用 Magpie').click());
 expect(node.querySelector('[role="alert"]')?.textContent).toContain('请先打开 Magpie');expect(node.querySelector('[data-managed="magpie"]')).toBeNull();expect(updated).not.toHaveBeenCalled();
 await act(async()=>trigger().click());expect(menuButton('使用 Magpie').disabled).toBe(false);expect(node.querySelector('#provider-url')).toBeNull();
});
it('retains a busy or concurrently changed connection when native removal rejects',async()=>{
 mocks.call.mockImplementation(async(command:string)=>{if(command==='remove_magpie')throw Error('连接正被任务使用');return command==='list_provider_accounts'?[]:undefined});
 await render(managed());await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="移除 Magpie"]')!.click());
 expect(node.querySelector('[data-managed="magpie"]')).not.toBeNull();expect(node.querySelector('[role="alert"]')?.textContent).toContain('正被任务使用');
});
it('disables repeated actions while a real add request is still pending',async()=>{
 let finish!:(value:unknown)=>void;mocks.call.mockImplementation(async(command:string)=>command==='list_provider_accounts'?[]:command==='use_magpie'?new Promise(resolve=>finish=resolve):undefined);
 await render();await act(async()=>trigger().click());await act(async()=>menuButton('使用 Magpie').click());expect(trigger().disabled).toBe(true);expect(menu()).toBeNull();
 await act(async()=>trigger().click());expect(mocks.call.mock.calls.filter(([c])=>c==='use_magpie')).toHaveLength(1);
 await act(async()=>finish({models:5000,added:5}));expect(trigger().disabled).toBe(false);
});
