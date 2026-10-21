// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ProviderSettings from './ProviderSettings';
import type { AgentProfile, ProviderSettings as Config } from './types';
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
async function render(runningConnections: Record<string, string[]> = {}, updated = async () => {}){await act(async()=>root.render(<ProviderSettings profiles={profiles} officialAgents={['codex','claude']} order={order} runningConnections={runningConnections} updated={updated}/>));}
it('counts only configured suppliers and confirmed official logins, with no CLI placeholder',async()=>{
  await render();expect(tab('claude').textContent).toContain('2');expect(tab('codex').textContent).toContain('1');expect(tab('opencode').textContent).toContain('0');expect(tab('pi').textContent).toContain('0');
  expect(node.textContent).not.toContain('本机 CLI 配置');expect(node.textContent).not.toContain('Claude 官方账号');
  await act(async()=>tab('opencode').click());expect(node.textContent).toContain('暂无已配置连接');expect(node.querySelector('.provider-sort-row')).toBeNull();
});
it('passes the active Agent to CC Switch import and switches its scope with the provider tab',async()=>{
  const original=mocks.call.getMockImplementation()!;
  const path='C:\\Users\\全新用户\\.cc-switch\\cc-switch.db';
  mocks.call.mockImplementation(async(command:string,args:any)=>command==='scan_ccswitch'?{path,warnings:[],profiles:[{id:`${args.agent}:own`,agent:args.agent,name:`${args.agent} 专用配置`,current:true}]}:original(command,args));
  await render();await act(async()=>tab('codex').click());await act(async()=>button('CC Switch 导入').click());
  await act(async()=>{await vi.dynamicImportSettled();});
  expect(mocks.call).toHaveBeenCalledWith('scan_ccswitch',{path:null,agent:'codex'});
  expect(node.querySelector('.provider-picker-heading h3')?.textContent).toBe('CC Switch 导入 · Codex');
  expect(node.querySelector('.cc-source-list')?.textContent).toContain('codex 专用配置');
  await act(async()=>tab('claude').click());
  expect(mocks.call).toHaveBeenLastCalledWith('scan_ccswitch',{path,agent:'claude'});
  expect(node.querySelector('.provider-picker-heading h3')?.textContent).toBe('CC Switch 导入 · Claude Code');
  expect(node.querySelector('.cc-source-list')?.textContent).not.toContain('codex 专用配置');
  expect(button('导入所选 (1)').disabled).toBe(false);
  for (const agent of ['opencode', 'pi']) {
    await act(async()=>tab(agent).click());
    expect(mocks.call).toHaveBeenLastCalledWith('scan_ccswitch',{path,agent});
    expect(node.querySelector('.provider-picker-heading h3')?.textContent).toBe(`CC Switch 导入 · ${agent==='pi'?'Pi':'OpenCode'}`);
    expect(node.querySelector('.cc-source-list')?.textContent).toContain(`${agent} 专用配置`);
    expect(button('导入所选 (1)').disabled).toBe(false);
  }
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
it.each([
  ['智谱 GLM','https://open.bigmodel.cn/api/v1','glm-5.3','https://docs.bigmodel.cn/cn/coding-plan/tool/codex'],
  ['DeepSeek','https://api.deepseek.com','deepseek-flash','https://api-docs.deepseek.com/quick_start/agent_integrations/codex/'],
])('configures %s on Codex with the documented Responses endpoint',async(name,url,model,docs)=>{
  await render();await act(async()=>tab('codex').click());await act(async()=>button('添加').click());
  await act(async()=>providerCard(name).click());
  expect(node.querySelector<HTMLInputElement>('#provider-url')!.value).toBe(url);
  expect(node.querySelector<HTMLInputElement>('#provider-default-model')!.value).toBe(model);
  expect(node.querySelector('#provider-protocol')?.textContent).toBe('OpenAI Responses');
  expect(node.querySelector('.provider-engine-note')?.textContent).toContain('Codex');
  expect([...node.querySelector<HTMLSelectElement>('#provider-plan')!.options].every(option=>option.textContent?.includes('Responses'))).toBe(true);
  await act(async()=>node.querySelector<HTMLAnchorElement>('.provider-plan-help a')!.click());
  expect(mocks.call).toHaveBeenCalledWith('open_external_link',{url:docs});
});
it('separates subscription and pay-as-you-go Codex endpoints when changing plans',async()=>{
  await render();await act(async()=>tab('codex').click());await act(async()=>button('添加').click());
  await act(async()=>providerCard('火山方舟 / 豆包').click());
  expect(node.querySelector<HTMLInputElement>('#provider-url')!.value).toBe('https://ark.cn-beijing.volces.com/api/coding/v3');
  expect(node.querySelector<HTMLInputElement>('#provider-default-model')!.value).toBe('ark-code-latest');
  await act(async()=>{const input=node.querySelector<HTMLInputElement>('#provider-key')!;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'subscription-key');input.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>{const select=node.querySelector<HTMLSelectElement>('#provider-plan')!;select.value='standard-responses';select.dispatchEvent(new Event('change',{bubbles:true}));});
  expect(node.querySelector<HTMLInputElement>('#provider-url')!.value).toBe('https://ark.cn-beijing.volces.com/api/v3');
  expect(node.querySelector<HTMLInputElement>('#provider-default-model')!.value).toBe('');
  expect(node.querySelector<HTMLInputElement>('#provider-key')!.value).toBe('');
  expect(node.querySelector('.provider-plan-help')?.textContent).toContain('按量 API Key');
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
  await act(async()=>root.render(<ProviderSettings profiles={[...profiles,managed]} officialAgents={['codex']} order={order} runningConnections={{}} updated={async()=>{}}/>));
  expect(tab('codex').textContent).toContain('1');await act(async()=>tab('codex').click());expect(node.querySelector('.saved-provider-list')?.textContent).not.toContain('工作账号');
});
const config = (id: string): Config => ({ id, agent:'claude',name:id === 'kimi' ? 'Kimi' : 'GLM',providerId:'custom',plan:'anthropic',protocol:'anthropic',baseUrl:'https://example.com',model:'test-model',models:['test-model'],hasCredential:true });
const savedCard = (id: string) => node.querySelector<HTMLButtonElement>(`[data-connection-id="${id}"] .saved-provider`)!;
it('allows adding a new supplier and editing another connection during a task', async () => {
  const original = mocks.call.getMockImplementation()!;
  const updated = vi.fn(async () => {});
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'get_provider_profile' ? config(args.id) : command === 'save_provider_profile' ? {id:args.settings.id ?? 'new',agent:'claude',hasCredential:true,current:args.activate} : original(command,args));
  await render({claude:['kimi']}, updated);
  expect(savedCard('kimi').textContent).toContain('任务使用中');
  expect(button('添加').disabled).toBe(false); expect(button('CC Switch 导入').disabled).toBe(false);
  await act(async () => button('添加').click()); await act(async () => providerCard('Kimi / Moonshot').click());
  expect(node.querySelector<HTMLInputElement>('#provider-name')?.disabled).toBe(false);
  await act(async () => button('保存').click());
  expect(mocks.call).toHaveBeenCalledWith('save_provider_profile', { settings: expect.objectContaining({id:null,agent:'claude'}),activate:false });
  await act(async () => button('所有连接').click()); await act(async () => savedCard('glm').click());
  expect(node.querySelector<HTMLInputElement>('#provider-key')?.disabled).toBe(false);
  await act(async () => button('保存为默认').click());
  expect(mocks.call).toHaveBeenCalledWith('save_provider_profile', { settings: expect.objectContaining({id:'glm'}),activate:true });
  expect(updated).toHaveBeenCalledWith('claude');
});
it('opens the used supplier read-only, and reacts when its task starts or finishes', async () => {
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'get_provider_profile' ? config(args.id) : original(command,args));
  await render(); await act(async () => savedCard('kimi').click());
  expect(button('保存').disabled).toBe(false);
  await render({claude:['kimi']});
  expect(node.querySelector('[role="status"]')?.textContent).toContain('任务完成或停止后可编辑');
  for (const id of ['provider-name','provider-url','provider-key','provider-default-model']) expect(node.querySelector<HTMLInputElement>(`#${id}`)?.disabled).toBe(true);
  for (const label of ['保存','保存为默认','删除','测试','获取模型']) expect(button(label).disabled).toBe(true);
  await act(async () => button('保存').click());
  expect(mocks.call).not.toHaveBeenCalledWith('save_provider_profile', expect.anything());
  await render({codex:['kimi']}); expect(button('保存').disabled).toBe(false);
  await render({claude:['kimi']}); expect(button('保存').disabled).toBe(true);
  await render(); expect(button('保存').disabled).toBe(false);
});
it('permits default reordering without modifying a connection used by a task', async () => {
  await render({claude:['kimi']});
  expect(node.querySelector<HTMLButtonElement>('[aria-label="下移 Kimi"]')?.disabled).toBe(false);
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="下移 Kimi"]')!.click());
  expect(mocks.call).toHaveBeenCalledWith('reorder_provider_connections', {agent:'claude',ids:['glm','kimi','@local','@official']});
  expect(mocks.call).not.toHaveBeenCalledWith('save_provider_profile', expect.anything());
});

async function inputValue(id: string, value: string) {
  await act(async () => {
    const input = node.querySelector<HTMLInputElement>(`#${id}`)!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
async function selectValue(id: string, value: string) {
  await act(async () => {
    const select = node.querySelector<HTMLSelectElement>(`#${id}`)!;
    select.value = value; select.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

it.each(['opencode', 'pi'].flatMap(agent => [false, true].map(hasCredential => ({ agent, hasCredential }))))('allows $agent saved protocol changes with saved credential=$hasCredential, preserving the connection and key', async ({ agent, hasCredential }) => {
  const own: AgentProfile = { id: 'api', agent, name: 'API', model: 'test-model', current: false, hasCredential };
  const settings: Config = { ...config('api'), agent, name: 'API', protocol: 'responses', plan: 'responses', hasCredential };
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'get_provider_profile' ? settings : command === 'save_provider_profile' ? own : original(command, args));
  await act(async () => root.render(<ProviderSettings profiles={[own]} officialAgents={[]} initialAgent={agent} updated={async () => {}}/>));
  await act(async () => savedCard('api').click());
  expect(node.querySelector<HTMLSelectElement>('#provider-protocol')!.disabled).toBe(false);
  await selectValue('provider-protocol', 'gemini');
  await act(async () => button('保存').click());
  expect(mocks.call).toHaveBeenCalledWith('save_provider_profile', { settings: expect.objectContaining({ id: 'api', protocol: 'gemini', plan: 'gemini', hasCredential, apiKey: null }), activate: false });
});

it('fetches models from an unsaved draft without a default model and retains its entered key and name', async () => {
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'fetch_provider_models_for_settings' ? ['returned-model', 'second-model'] : command === 'save_provider_profile' ? { id: 'new', agent: 'opencode', hasCredential: true, current: false } : original(command, args));
  await render(); await act(async () => tab('opencode').click()); await act(async () => button('添加').click()); await act(async () => providerCard('自定义 / 兼容 API').click());
  await inputValue('provider-name', '我的未保存连接'); await inputValue('provider-url', 'https://example.com/v1beta'); await inputValue('provider-key', 'draft-key'); await inputValue('provider-default-model', ''); await selectValue('provider-protocol', 'gemini');
  expect(button('获取模型').disabled).toBe(false);
  await act(async () => button('获取模型').click());
  expect(mocks.call).toHaveBeenCalledWith('fetch_provider_models_for_settings', { settings: expect.objectContaining({ id: null, name: '我的未保存连接', model: '', apiKey: 'draft-key', protocol: 'gemini', baseUrl: 'https://example.com/v1beta' }) });
  expect(mocks.call.mock.calls.some(([command]) => command === 'save_provider_profile')).toBe(false);
  expect(node.querySelector<HTMLInputElement>('#provider-default-model')!.value).toBe('returned-model');
  expect(node.querySelector<HTMLInputElement>('#provider-name')!.value).toBe('我的未保存连接');
  expect(node.querySelector<HTMLInputElement>('#provider-key')!.value).toBe('draft-key');
  expect(node.querySelector('.provider-editor-heading .connection-state')!.textContent).toBe('新连接');
  await act(async () => button('保存').click());
  expect(mocks.call).toHaveBeenCalledWith('save_provider_profile', { settings: expect.objectContaining({ id: null, apiKey: 'draft-key', model: 'returned-model', models: ['returned-model', 'second-model'] }), activate: false });
});

it('uses a saved key for unsaved protocol discovery without silently saving or replacing the default model', async () => {
  const original = mocks.call.getMockImplementation()!;
  const settings = { ...config('api'), agent: 'pi', name: 'API', protocol: 'responses', plan: 'responses' };
  const own = { id: 'api', agent: 'pi', name: 'API', model: 'test-model', current: false, hasCredential: true };
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'get_provider_profile' ? settings : command === 'fetch_provider_models_for_settings' ? ['another-model'] : original(command, args));
  await act(async () => root.render(<ProviderSettings profiles={[own]} officialAgents={[]} initialAgent="pi" updated={async () => {}}/>));
  await act(async () => savedCard('api').click()); await selectValue('provider-protocol', 'chat'); await act(async () => button('获取模型').click());
  expect(mocks.call).toHaveBeenCalledWith('fetch_provider_models_for_settings', { settings: expect.objectContaining({ id: 'api', protocol: 'chat', apiKey: null, hasCredential: true }) });
  expect(mocks.call.mock.calls.some(([command]) => command === 'save_provider_profile')).toBe(false);
  expect(node.querySelector<HTMLInputElement>('#provider-default-model')!.value).toBe('test-model');
  expect(node.querySelector('.provider-model-list')!.textContent).toContain('another-model');
  expect(node.querySelector<HTMLInputElement>('#provider-key')!.readOnly).toBe(true);
});

it('retains a draft on model-discovery failure without creating a connection', async () => {
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'fetch_provider_models_for_settings' ? Promise.reject(new Error('HTTP 404')) : original(command, args));
  await render(); await act(async () => button('添加').click()); await act(async () => providerCard('自定义 / 兼容 API').click()); await inputValue('provider-url', 'https://example.com'); await inputValue('provider-key', 'draft-key'); await inputValue('provider-default-model', '');
  await act(async () => button('获取模型').click());
  expect(node.querySelector('[role="alert"]')!.textContent).toContain('HTTP 404');
  expect(node.querySelector<HTMLInputElement>('#provider-key')!.value).toBe('draft-key');
  expect(mocks.call.mock.calls.some(([command]) => command === 'save_provider_profile')).toBe(false);
});

it('deletes a connection directly from its list row only after naming it in the confirmation', async () => {
  const updated = vi.fn(async () => {}); await render({}, updated);
  expect(node.querySelector('button button')).toBeNull();
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="删除 Kimi"]')!.click());
  expect(node.querySelector('[aria-label="确认删除连接"]')!.textContent).toContain('Kimi');
  expect(node.querySelector('[aria-label="确认删除连接"]')!.textContent).toContain('API Key');
  expect(mocks.call.mock.calls.some(([command]) => command === 'delete_provider_profiles')).toBe(false);
  await act(async () => button('确认删除 1 个连接').click());
  expect(mocks.call).toHaveBeenCalledWith('delete_provider_profiles', { agent: 'claude', ids: ['kimi'] });
  expect(updated).toHaveBeenCalledOnce();
});

it('selects and atomically deletes multiple connections, with select-all and partial selection accessible', async () => {
  await render(); const all = () => node.querySelector<HTMLInputElement>('[aria-label="全选当前可删除连接"]')!;
  await act(async () => node.querySelector<HTMLInputElement>('[aria-label="选择 Kimi"]')!.click());
  expect(all().indeterminate).toBe(true); expect(button('删除所选 (1)').disabled).toBe(false);
  await act(async () => all().click()); expect(all().checked).toBe(true); expect(all().indeterminate).toBe(false);
  await act(async () => button('删除所选 (2)').click());
  expect(node.querySelector('[aria-label="确认删除连接"]')!.textContent).toContain('Kimi、GLM');
  await act(async () => button('确认删除 2 个连接').click());
  expect(mocks.call).toHaveBeenCalledWith('delete_provider_profiles', { agent: 'claude', ids: ['kimi', 'glm'] });
  expect(mocks.call.mock.calls.filter(([command]) => command === 'delete_provider_profiles')).toHaveLength(1);
});

it('select-all targets only visible idle connections, and rechecks task use before deletion', async () => {
  await render({ claude: ['kimi'] });
  expect(node.querySelector<HTMLInputElement>('[aria-label="选择 Kimi"]')!.disabled).toBe(true);
  expect(node.querySelector<HTMLButtonElement>('[aria-label="删除 Kimi"]')!.disabled).toBe(true);
  await act(async () => node.querySelector<HTMLInputElement>('[aria-label="全选当前可删除连接"]')!.click()); await act(async () => button('删除所选 (1)').click());
  expect(node.querySelector('[aria-label="确认删除连接"]')!.textContent).toContain('GLM');
  await render({ claude: ['kimi', 'glm'] }); expect(button('确认删除 1 个连接').disabled).toBe(true);
  await act(async () => button('确认删除 1 个连接').click()); expect(mocks.call.mock.calls.some(([command]) => command === 'delete_provider_profiles')).toBe(false);
  await act(async () => button('取消').click()); await render();
  await act(async () => { const input = node.querySelector<HTMLInputElement>('[aria-label="搜索模型连接"]')!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'Kimi'); input.dispatchEvent(new Event('input', { bubbles: true })); });
  await act(async () => node.querySelector<HTMLInputElement>('[aria-label="全选当前可删除连接"]')!.click()); await act(async () => button('删除所选 (1)').click()); await act(async () => button('确认删除 1 个连接').click());
  expect(mocks.call).toHaveBeenCalledWith('delete_provider_profiles', { agent: 'claude', ids: ['kimi'] });
});

it('keeps native official accounts out of deletion and opens account management without logging out', async () => {
  await render(); await act(async () => tab('codex').click());
  expect(node.querySelector<HTMLInputElement>('[aria-label="全选当前可删除连接"]')!.disabled).toBe(true);
  expect(node.querySelector('.provider-row-delete')).toBeNull(); expect(node.querySelector('.provider-row-select')).toBeNull();
  await act(async () => savedCard('@official').click()); expect(mocks.call).toHaveBeenCalledWith('list_official_accounts', { agent: 'codex', force: false });
  expect(mocks.call.mock.calls.some(([command]) => ['delete_provider_profiles', 'remove_official_account', 'logout_agent_account'].includes(command))).toBe(false);
});

it('offers Gemini CLI with its native protocol and no unrelated official-account login', async () => {
  await render(); expect(tab('gemini')).toBeDefined(); await act(async () => tab('gemini').click()); await act(async () => button('添加').click());
  expect(node.querySelector('.provider-api-heading')!.textContent).toContain('Gemini API');
  expect(node.querySelector('.official-login-card')).toBeNull();
});

it.each(['opencode', 'pi'])('protects %s protocol changes only while that saved connection is in use', async agent => {
  const own: AgentProfile = { id: 'api', agent, name: 'API', model: 'test-model', current: false, hasCredential: true };
  const settings: Config = { ...config('api'), agent, name: 'API', protocol: 'responses', plan: 'responses' };
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'get_provider_profile' ? settings : original(command, args));
  const paint = async (runningConnections: Record<string, string[]>) => act(async () => root.render(<ProviderSettings profiles={[own]} officialAgents={[]} initialAgent={agent} runningConnections={runningConnections} updated={async () => {}}/>));
  await paint({}); await act(async () => savedCard('api').click()); await selectValue('provider-protocol', 'chat');
  await paint({ [agent]: ['api'] }); expect(node.querySelector<HTMLSelectElement>('#provider-protocol')!.disabled).toBe(true);
  await paint({ [agent === 'pi' ? 'opencode' : 'pi']: ['api'] }); expect(node.querySelector<HTMLSelectElement>('#provider-protocol')!.disabled).toBe(false);
  expect(node.querySelector<HTMLSelectElement>('#provider-protocol')!.value).toBe('chat');
});

it('keeps all selected rows and the confirmation when an atomic deletion fails', async () => {
  const original = mocks.call.getMockImplementation()!;
  mocks.call.mockImplementation(async (command: string, args: any) => command === 'delete_provider_profiles' ? Promise.reject(new Error('任务使用中的连接不能删除')) : original(command, args));
  const updated = vi.fn(async () => {}); await render({}, updated);
  await act(async () => node.querySelector<HTMLInputElement>('[aria-label="全选当前可删除连接"]')!.click()); await act(async () => button('删除所选 (2)').click()); await act(async () => button('确认删除 2 个连接').click());
  expect(node.querySelector('[role="alert"]')!.textContent).toContain('任务使用中');
  expect(node.querySelectorAll('.provider-sort-row')).toHaveLength(2);
  expect(node.querySelectorAll<HTMLInputElement>('.provider-row-select:checked')).toHaveLength(2);
  expect(button('确认删除 2 个连接').disabled).toBe(false); expect(updated).not.toHaveBeenCalled();
});
