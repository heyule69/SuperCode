// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ProviderSettings from './ProviderSettings';
import type { AgentProfile } from './types';
const mocks=vi.hoisted(()=>({call:vi.fn()}));
vi.mock('./api',()=>({desktop:true,call:mocks.call}));
let root:Root,node:HTMLDivElement;
const profiles:AgentProfile[]=[{id:'kimi',agent:'claude',name:'Kimi',model:'k3',hasCredential:true,current:false},{id:'glm',agent:'claude',name:'GLM',model:'glm-5.3',hasCredential:true,current:false}];
const order={claude:['@local','kimi','@official','glm'],codex:['@official'],opencode:['@local'],pi:['@local']};
let accounts:Promise<unknown[]>;
const button=(label:string)=>[...node.querySelectorAll<HTMLButtonElement>('button')].find(item=>item.textContent===label)!;
const tab=(agent:string)=>node.querySelector<HTMLButtonElement>(`#providers-${agent}-tab`)!;
const providerCard=(name:string)=>[...node.querySelectorAll<HTMLButtonElement>('.provider-preset')].find(item=>item.querySelector('strong')?.textContent===name)!;
beforeEach(()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
  sessionStorage.clear();
  node=document.createElement('div');document.body.append(node);root=createRoot(node);
  accounts=Promise.resolve([{agent:'codex',loggedIn:true,method:'chatgpt'},{agent:'claude',loggedIn:false}]);
  mocks.call.mockReset();mocks.call.mockImplementation(async(command:string)=>command==='list_provider_accounts'?accounts:undefined);
});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();});
async function render(){await act(async()=>root.render(<ProviderSettings profiles={profiles} officialAgents={['codex','claude']} order={order} busy={false} updated={async()=>{}}/>));}
it('counts only configured suppliers and confirmed official logins, with no CLI placeholder',async()=>{
  await render();expect(tab('claude').textContent).toContain('2');expect(tab('codex').textContent).toContain('1');expect(tab('opencode').textContent).toContain('0');expect(tab('pi').textContent).toContain('0');
  expect(node.textContent).not.toContain('本机 CLI 配置');expect(node.textContent).not.toContain('Claude 官方账号');
  await act(async()=>tab('opencode').click());expect(node.textContent).toContain('暂无已配置连接');expect(node.querySelector('.provider-sort-row')).toBeNull();
});
it('leaves configured API connections visible while native login checks are pending',async()=>{
  let finish!:(value:unknown[])=>void;accounts=new Promise(resolve=>{finish=resolve;});
  await render();expect(node.querySelectorAll('.provider-sort-row')).toHaveLength(2);expect(tab('codex').textContent).toContain('0');
  await act(async()=>finish([{agent:'codex',loggedIn:true},{agent:'claude',loggedIn:false}]));expect(tab('codex').textContent).toContain('1');
});
it('does not relabel a filtered first row as default, and preserves hidden routes when sorting',async()=>{
  await render();expect(node.querySelector('.saved-provider-list')?.textContent).not.toContain('新聊天默认');
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="下移 Kimi"]')!.click());
  expect(mocks.call).toHaveBeenCalledWith('reorder_provider_connections',{agent:'claude',ids:['glm','kimi','@local','@official']});
});
it('hides an unavailable official account, retains API connections, and supports retry',async()=>{
  accounts=Promise.resolve([{agent:'codex',loggedIn:false,error:'账号检查失败'},{agent:'claude',loggedIn:false}]);
  await render();expect(tab('codex').textContent).toContain('0');expect(node.querySelectorAll('.provider-sort-row')).toHaveLength(2);expect(node.querySelector('[role="alert"]')?.textContent).toContain('账号检查失败');
  accounts=Promise.resolve([{agent:'codex',loggedIn:true},{agent:'claude',loggedIn:false}]);await act(async()=>button('刷新连接').click());
  expect(mocks.call).toHaveBeenCalledWith('list_provider_accounts',{force:true});expect(tab('codex').textContent).toContain('1');
});
it('keeps official login setup inside Add without listing an unconfigured connection',async()=>{
  await render();await act(async()=>button('添加').click());expect(providerCard('Claude 官方账号')).toBeDefined();expect(providerCard('ChatGPT 官方账号')).toBeDefined();expect(node.querySelector('#provider-key')).toBeNull();
});
it('starts native browser login from the official card without opening an API form',async()=>{
  mocks.call.mockImplementation(async(command:string)=>command==='list_provider_accounts'?accounts:command==='start_official_login'?{message:'请在浏览器完成登录'}:undefined);
  await render();await act(async()=>button('添加').click());await act(async()=>providerCard('Claude 官方账号').click());
  expect(mocks.call).toHaveBeenCalledWith('start_official_login',{agent:'claude'});expect(node.querySelector('#provider-key')).toBeNull();expect(node.querySelector('#provider-url')).toBeNull();
  expect(node.textContent).toContain('等待浏览器登录');expect(providerCard('ChatGPT 官方账号').disabled).toBe(true);
  mocks.call.mockImplementation(async(command:string)=>command==='official_account_status'?{loggedIn:true}:undefined);
  await act(async()=>button('检查登录').click());await act(async()=>tab('claude').click());
  expect(tab('claude').textContent).toContain('3');expect(node.querySelector('.saved-provider-list')?.textContent).toContain('Claude 官方账号');expect(node.textContent).not.toContain('等待浏览器登录');
});
it('reuses a confirmed local ChatGPT login instead of requesting another sign-in',async()=>{
  await render();await act(async()=>button('添加').click());await act(async()=>providerCard('ChatGPT 官方账号').click());
  expect(mocks.call).toHaveBeenCalledWith('use_official_account',{agent:'codex'});expect(mocks.call.mock.calls.some(([command])=>command==='start_official_login')).toBe(false);
  expect(node.querySelector('.provider-native-heading')?.textContent).toContain('ChatGPT 官方账号');expect(node.querySelector('#provider-key')).toBeNull();
});
it('only opens a key form when an explicit API connection is selected',async()=>{
  await render();await act(async()=>button('添加').click());await act(async()=>button('使用 API Key').click());
  expect(providerCard('ChatGPT 官方账号')).toBeUndefined();await act(async()=>providerCard('OpenAI API').click());
  expect(node.querySelector('#provider-key')).not.toBeNull();expect(node.querySelector<HTMLInputElement>('#provider-url')?.value).toBe('https://api.openai.com/v1');
  expect(mocks.call.mock.calls.some(([command])=>command==='start_official_login')).toBe(false);
});
it('keeps OpenCode and Pi API connections with their own engine rather than using another agent login',async()=>{
  await render();await act(async()=>tab('opencode').click());await act(async()=>button('添加').click());
  expect(providerCard('ChatGPT 官方账号')).toBeUndefined();await act(async()=>providerCard('OpenAI API').click());
  expect(node.querySelector('.provider-engine-note')?.textContent).toContain('OpenCode');expect(node.querySelector('#provider-key')).not.toBeNull();
  await act(async()=>button('所有连接').click());await act(async()=>tab('pi').click());await act(async()=>button('添加').click());await act(async()=>providerCard('Anthropic API').click());
  expect(node.querySelector('.provider-engine-note')?.textContent).toContain('Pi');expect(mocks.call.mock.calls.some(([command])=>command==='start_official_login')).toBe(false);
});
it('reports native login failure without creating a key form or a connected account',async()=>{
  mocks.call.mockImplementation(async(command:string)=>{if(command==='list_provider_accounts')return accounts;if(command==='start_official_login')throw new Error('无法打开浏览器');});
  await render();await act(async()=>button('添加').click());await act(async()=>providerCard('Claude 官方账号').click());
  expect(node.querySelector('[role="alert"]')?.textContent).toContain('无法打开浏览器');expect(node.querySelector('#provider-key')).toBeNull();expect(sessionStorage.getItem('supercode.pendingLogin')).toBeNull();
  await act(async()=>button('返回').click());expect(tab('claude').textContent).toContain('2');
});
