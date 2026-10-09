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
  mocks.call.mockReset();mocks.call.mockImplementation(async(command:string)=>command==='list_provider_accounts'?accounts:command==='list_official_accounts'?[{id:'@official',agent:'claude',name:'本机账号',loggedIn:false,current:false}]:undefined);
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
  await render();await act(async()=>button('添加').click());expect(providerCard('Claude 官方登录')).toBeDefined();expect(providerCard('ChatGPT 官方登录')).toBeUndefined();expect(node.querySelector('#provider-key')).toBeNull();expect(button('使用 API Key')).toBeUndefined();
});
it('opens account management first and only starts login after naming the additional account',async()=>{
  const original=mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async(command:string,args:unknown)=>command==='start_official_account_login'?{accountId:'new-account',message:'请在浏览器完成登录'}:original(command,args));
  await render();await act(async()=>button('添加').click());await act(async()=>providerCard('Claude 官方登录').click());
  expect(mocks.call).toHaveBeenCalledWith('list_official_accounts',{agent:'claude',force:false});expect(node.querySelector('#provider-key')).toBeNull();expect(node.querySelector('#provider-url')).toBeNull();
  expect(mocks.call.mock.calls.some(([command])=>command==='start_official_account_login')).toBe(false);
  await act(async()=>button('添加账号').click());await act(async()=>{const input=node.querySelector<HTMLInputElement>('#official-account-name')!;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'工作账号');input.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>node.querySelector('.official-account-add')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  expect(mocks.call).toHaveBeenCalledWith('start_official_account_login',{agent:'claude',name:'工作账号'});expect(node.textContent).toContain('等待浏览器登录');
});
it('keeps the selected Agent while adding and exposes only its official login',async()=>{
  await render();await act(async()=>button('添加').click());await act(async()=>tab('codex').click());
  expect(providerCard('ChatGPT 官方登录')).toBeDefined();expect(providerCard('Claude 官方登录')).toBeUndefined();expect(providerCard('Anthropic API')).toBeUndefined();
  await act(async()=>providerCard('自定义 / 兼容 API').click());expect(node.querySelector('.provider-engine-note')?.textContent).toContain('Codex');expect(node.querySelector('#provider-protocol')?.textContent).toBe('OpenAI Responses');
});
it('uses fixed Anthropic Messages for Claude third-party connections',async()=>{
  await render();await act(async()=>button('添加').click());expect(providerCard('OpenAI API')).toBeUndefined();expect(providerCard('Groq')).toBeUndefined();await act(async()=>providerCard('Kimi / Moonshot').click());
  expect(node.querySelector('#provider-key')).not.toBeNull();expect(node.querySelector('#provider-protocol')?.textContent).toBe('Anthropic Messages');expect(node.querySelector('.provider-engine-note')?.textContent).toContain('Claude Code');
  expect(mocks.call.mock.calls.some(([command])=>command==='start_official_login')).toBe(false);
});
it('keeps OpenCode and Pi API connections with their own engine rather than using another agent login',async()=>{
  await render();await act(async()=>tab('opencode').click());await act(async()=>button('添加').click());
  expect(providerCard('ChatGPT 官方登录')).toBeUndefined();await act(async()=>providerCard('OpenAI API').click());
  expect(node.querySelector('.provider-engine-note')?.textContent).toContain('OpenCode');expect(node.querySelector('#provider-key')).not.toBeNull();
  await act(async()=>button('所有连接').click());await act(async()=>tab('pi').click());await act(async()=>button('添加').click());await act(async()=>providerCard('Anthropic API').click());
  expect(node.querySelector('.provider-engine-note')?.textContent).toContain('Pi');expect(mocks.call.mock.calls.some(([command])=>command==='start_official_login')).toBe(false);
});
it('does not authorize managed accounts from a different logged-in native account',async()=>{
  const managed:AgentProfile={id:'account:one',accountId:'one',agent:'codex',name:'工作账号',officialAccount:true,model:'test-model',current:false,hasCredential:false};
  await act(async()=>root.render(<ProviderSettings profiles={[...profiles,managed]} officialAgents={['codex']} order={order} busy={false} updated={async()=>{}}/>));
  expect(tab('codex').textContent).toContain('1');await act(async()=>tab('codex').click());expect(node.querySelector('.saved-provider-list')?.textContent).not.toContain('工作账号');
});
