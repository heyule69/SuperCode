// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { call, subscribe } from './api';
import type { Bootstrap, Message, Session, RpcEvent } from './types';
import { PROVIDER_REQUIRED } from './chatConnection';
import { mergeEvent } from './events';

vi.mock('./api', () => ({ desktop:true, call:vi.fn(), subscribe:vi.fn(async()=>()=>{}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen:vi.fn(async()=>()=>{}) }));
vi.mock('./windowTheme', () => ({ watchWindowTheme:()=>()=>{} }));
vi.mock('./DesktopTitleBar', () => ({ DesktopTitleBar:()=>null }));
vi.mock('./PlatformUsageView', () => ({ PlatformUsageIndicator:()=> <span data-testid="platform-allowance">供应商额度</span> }));
vi.mock('./AppUpdate', () => ({ default:()=>null, AppUpdateNotice:()=>null }));
let root: Root, container: HTMLDivElement, data: Bootstrap, messages: Message[], providerMissing: boolean;
let pendingRequests: RpcEvent[];
let notify: (event: RpcEvent) => void;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); vi.mocked(call).mockClear(); messages = []; pendingRequests = []; providerMissing = false;
  vi.mocked(subscribe).mockImplementation(async handler => { notify = handler; return () => {}; });
  data = { projects:[{id:'existing',name:'已有项目',path:'D:/existing'}],sessions:[],agents:[],profiles:[{id:'api:claude',agent:'claude',name:'Test API',model:'test-model',hasCredential:true,current:true}],officialAgents:[],connectionOrder:{},sidebar:{ projects:{}, sessions:{}, sections:[] },codexPath:null,loadMcp:false } as Bootstrap;
  vi.mocked(call).mockImplementation(async (command, args={}) => {
    if (command==='bootstrap') return structuredClone(data);
    if (command==='check_chat_connection') { if (providerMissing) throw PROVIDER_REQUIRED; return null; }
    if (command==='list_provider_accounts') return [];
    if (command==='get_model_source') return { providerId:providerMissing?'unknown':'custom', providerName:providerMissing?'':'Test API', mark:providerMissing?'':'T', available:!providerMissing };
    if (command==='list_models') {
      const profile = data.profiles?.find(p=>p.id===args.connectionId);
      return { data:profile?.model?[{id:profile.model,model:profile.model,displayName:profile.model,isDefault:true}]:[], source:{providerId:'custom',providerName:'Test API',mark:'T',available:!providerMissing} };
    }
    if (command==='list_messages') return [...messages];
    if (command==='get_usage') return { records:[] };
    if (command==='pending_requests') return pendingRequests;
    if (command==='list_local_skills') return { skills:[] };
    if (command==='list_extensions') return { plugins:[], mcp:[], warnings:[] };
    if (command==='followup_capabilities') return {};
    if (command==='list_followups') return [];
    if (command==='workspace_changes') return { files:[],diff:'',branch:'',isGit:false };
    if (command==='browser_open') return { sessionId:String(args.sessionId),url:'https://example.com/',title:'Example',status:'idle',action:'' };
    if (command==='create_session') {
      const chat: Session = { id:'standalone',projectId:String(args.projectId),workspacePath:'C:/Documents/SuperCode/chat',title:'新会话',agent:String(args.agent),model:null,connectionId:String(args.connectionId),nativeId:null,status:'idle',turnId:null,updatedAt:1 };
      data.sessions.push(chat); return chat;
    }
    if (command==='archive_session') { data.sessions = []; return null; }
    if (command==='send_chat_message') {
      messages = [{ id:'saved',seq:1,sessionId:'standalone',role:'user',text:String(args.text),kind:'userMessage',data:{clientMessageId:args.clientMessageId} }];
      return data.sessions[0];
    }
    return null;
  });
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async()=>root.unmount()); container.remove(); vi.restoreAllMocks(); });
async function render() { await act(async()=>{root.render(<App/>); await new Promise(resolve=>setTimeout(resolve,30));}); }
async function type(text: string) {
  const textarea = container.querySelector<HTMLTextAreaElement>('.composer textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,text);textarea.dispatchEvent(new Event('input',{bubbles:true}));});
}
async function send() {
  await act(async()=>{container.querySelector<HTMLFormElement>('.composer')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));await new Promise(resolve=>setTimeout(resolve,30));});
}

it.each(['claude','codex','opencode','pi'])('shows the submitted %s message before connection checks, session creation and native startup finish', async agent => {
  localStorage.setItem('supercode.agent', agent);
  data.profiles = [{...data.profiles[0], id:`api:${agent}`, agent}];
  let finishCheck!: () => void, finishCreate!: () => void, finishSend!: () => void;
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args) => {
    if (command === 'check_chat_connection') return new Promise<void>(resolve => { finishCheck = resolve; });
    if (command === 'create_session') { await new Promise<void>(resolve => { finishCreate = resolve; }); return fallback(command, args); }
    if (command === 'send_chat_message') { await new Promise<void>(resolve => { finishSend = resolve; }); return fallback(command, args); }
    return fallback(command, args);
  });
  await render(); await type('第一时间显示我'); await send();
  const bubble = container.querySelector('[aria-label="你的消息"]')!;
  expect(bubble.textContent).toContain('第一时间显示我');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('');
  const status = container.querySelector('.working-indicator')!;
  expect(status.textContent).toBe('正在发送…');
  expect(bubble.compareDocumentPosition(status) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(call).not.toHaveBeenCalledWith('create_session', expect.anything());
  await act(async () => finishCheck());
  expect(container.querySelectorAll('[aria-label="你的消息"]')).toHaveLength(1);
  expect(call).not.toHaveBeenCalledWith('send_chat_message', expect.anything());
  await act(async () => finishCreate());
  expect(container.querySelector('[aria-label="你的消息"]')?.textContent).toContain('第一时间显示我');
  expect(call).toHaveBeenCalledWith('send_chat_message', expect.objectContaining({clientMessageId:expect.any(String)}));
  // Preserve the bubble while native startup is pending; don't clear a new draft
  // or submit it twice if another submit event arrives before a render.
  await type('下一条草稿'); await send();
  expect(vi.mocked(call).mock.calls.filter(([command]) => command === 'send_chat_message')).toHaveLength(1);
  await act(async () => finishSend());
  expect(container.querySelectorAll('[aria-label="你的消息"]')).toHaveLength(1);
  expect(container.querySelector('[aria-label="你的消息"]')?.textContent).toContain('第一时间显示我');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('下一条草稿');
});

it('keeps the new user before streamed work when a startup storage read has not saved the user yet', async () => {
  localStorage.setItem('supercode.session', 'chat');
  data.sessions = [{id:'chat',projectId:'existing',title:'慢启动',agent:'claude',connectionId:'api:claude',model:'test-model',nativeId:'native',status:'idle',turnId:null,updatedAt:1}];
  let finish!: () => void;
  let clientMessageId = '';
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args={}) => {
    if (command === 'send_chat_message') {
      clientMessageId = String(args.clientMessageId);
      await new Promise<void>(resolve => { finish = resolve; }); return data.sessions[0];
    }
    return fallback(command, args);
  });
  await render(); await type('继续'); await send();
  await act(async () => {
    notify({method:'turn/started',params:{threadId:'native',turn:{id:'t'}}});
    notify({method:'item/agentMessage/delta',params:{threadId:'native',turnId:'t',itemId:'a',delta:'正在处理'}});
    await new Promise(resolve => setTimeout(resolve,80));
  });
  const bubble = container.querySelector('[aria-label="你的消息"]')!;
  expect(bubble.textContent).toContain('继续');
  expect(container.querySelectorAll('.conversation-turn')).toHaveLength(1);
  messages = [{id:'saved-user',seq:1,sessionId:'chat',role:'user',text:'继续',kind:'userMessage',data:{clientMessageId}}];
  await act(async () => finish());
  expect(container.querySelectorAll('[aria-label="你的消息"]')).toHaveLength(1);
  expect(container.querySelectorAll('.conversation-turn')).toHaveLength(1);
  expect(container.querySelector('[aria-label="你的消息"]')).toBe(bubble);
});

it('restores a rejected send and removes its temporary bubble without overwriting a newer draft', async () => {
  let reject!: (reason: string) => void;
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args) => command === 'check_chat_connection'
    ? new Promise((_, fail) => { reject = fail; }) : fallback(command, args));
  await render(); await type('失败后保留的消息'); await send();
  expect(container.querySelector('[aria-label="你的消息"]')).not.toBeNull();
  await act(async () => reject('连接失败'));
  expect(container.querySelector('[aria-label="你的消息"]')).toBeNull();
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('失败后保留的消息');
  await send(); await type('我新输入的草稿');
  await act(async () => reject('连接失败'));
  expect(container.querySelector('[aria-label="你的消息"]')).toBeNull();
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('我新输入的草稿');
});

it('ignores an older startup storage response that arrives after the acknowledgement', async () => {
  localStorage.setItem('supercode.session','chat');
  data.sessions = [{id:'chat',projectId:'existing',title:'乱序读取',agent:'claude',connectionId:'api:claude',model:'test-model',nativeId:'native',status:'idle',turnId:null,updatedAt:1}];
  let finishSend!: () => void, finishOlderRead!: (result: Message[]) => void;
  let holdNextRead = false, clientMessageId = '';
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args={}) => {
    if (command === 'send_chat_message') { clientMessageId = String(args.clientMessageId); await new Promise<void>(resolve => { finishSend = resolve; }); return data.sessions[0]; }
    if (command === 'list_messages' && holdNextRead) { holdNextRead = false; return new Promise<Message[]>(resolve => { finishOlderRead = resolve; }); }
    return fallback(command,args);
  });
  await render(); await type('用户应当一直可见'); await send();
  holdNextRead = true;
  await act(async () => { notify({method:'turn/started',params:{threadId:'native',turn:{id:'t'}}}); });
  messages = [{id:'saved-user',seq:1,sessionId:'chat',role:'user',text:'用户应当一直可见',kind:'userMessage',data:{clientMessageId}}];
  await act(async () => { finishSend(); await new Promise(resolve => setTimeout(resolve,30)); });
  const bubble = container.querySelector('[aria-label="你的消息"]')!;
  await act(async () => finishOlderRead([]));
  expect(container.querySelectorAll('[aria-label="你的消息"]')).toHaveLength(1);
  expect(container.querySelector('[aria-label="你的消息"]')).toBe(bubble);
  expect(bubble.textContent).toContain('用户应当一直可见');
});

it('does not turn an accepted send into a retry draft when the subsequent history refresh fails', async () => {
  localStorage.setItem('supercode.session','chat');
  data.sessions = [{id:'chat',projectId:'existing',title:'刷新失败',agent:'claude',connectionId:'api:claude',model:'test-model',nativeId:'native',status:'idle',turnId:null,updatedAt:1}];
  let failNextRead = false;
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command,args={}) => {
    if (command === 'send_chat_message') { messages = [{id:'saved-user',seq:1,sessionId:'chat',role:'user',text:String(args.text),kind:'userMessage',data:{clientMessageId:args.clientMessageId}}]; failNextRead = true; return data.sessions[0]; }
    if (command === 'list_messages' && failNextRead) { failNextRead = false; throw '读取失败'; }
    return fallback(command,args);
  });
  await render(); await type('已经发送的消息'); await send();
  expect(container.querySelectorAll('[aria-label="你的消息"]')).toHaveLength(1);
  expect(container.querySelector('[aria-label="你的消息"]')?.textContent).toContain('已经发送的消息');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('');
});
it('shows persisted context tokens and keeps live and completed usage visible without a model capacity', async () => {
  localStorage.setItem('supercode.session', 'context-chat');
  data.sessions = [{ id: 'context-chat', projectId: 'existing', title: '上下文测试', agent: 'claude', connectionId: 'api:claude', model: 'test-model', nativeId: 'native-context', status: 'running', turnId: 'turn-context', updatedAt: 1 }];
  let tokens = 12345;
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args) => command === 'get_usage' ? { records: [{ data: { total: {}, last: {}, contextTokens: tokens, cumulative: false } }] } : fallback(command, args));
  await render();
  expect(container.querySelector('.usage-trigger')?.textContent).toBe('上下文 12.3K');
  tokens = 23456;
  await act(async () => notify({ method: 'thread/tokenUsage/updated', params: { threadId: 'native-context', turnId: 'turn-context', tokenUsage: { total: {}, last: {}, contextTokens: tokens, cumulative: false } } }));
  expect(container.querySelector('.usage-trigger')?.textContent).toBe('上下文 23.5K');
  await act(async () => { notify({ method: 'turn/completed', params: { threadId: 'native-context', turn: { id: 'turn-context', status: 'completed' } } }); await new Promise(resolve => setTimeout(resolve, 30)); });
  expect(container.querySelector('.usage-trigger')?.textContent).toBe('上下文 23.5K');
});
it('does not let a delayed usage load overwrite newer live context on a slow device', async () => {
  localStorage.setItem('supercode.session', 'slow-context');
  data.sessions = [{ id: 'slow-context', projectId: 'existing', title: '上下文测试', agent: 'claude', connectionId: 'api:claude', model: 'test-model', nativeId: 'native-context', status: 'running', turnId: 'turn-context', updatedAt: 1 }];
  let resolve!: (value: unknown) => void;
  const pending = new Promise(done => { resolve = done; });
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args) => command === 'get_usage' ? pending : fallback(command, args));
  await render();
  await act(async () => notify({ method: 'thread/tokenUsage/updated', params: { threadId: 'native-context', tokenUsage: { total: {}, last: {}, contextTokens: 23456 } } }));
  await act(async () => { resolve({ records: [{ data: { total: {}, last: {}, contextTokens: 12345 } }] }); await pending; });
  expect(container.querySelector('.usage-trigger')?.textContent).toBe('上下文 23.5K');
});
it('loads official models locally on startup, reuses the result, and explicitly refreshes without a CLI', async () => {
  localStorage.setItem('supercode.session','official-chat');
  const official = { id:'official', agent:'codex', name:'ChatGPT', officialAccount:true, model:'old', models:['old','older','oldest'], current:true, hasCredential:false };
  data.profiles = [official]; data.officialAgents = ['codex'];
  data.sessions = [{id:'official-chat',projectId:'existing',title:'官方模型',agent:'codex',connectionId:'official',model:'old',status:'idle',nativeId:null,turnId:null,updatedAt:1}];
  const source = {providerId:'openai',providerName:'OpenAI',mark:'O',connectionName:'ChatGPT 官方账号',available:true};
  const live = ['latest','frontier','balanced','fast','old','older','oldest'].map(model => ({id:model,model,displayName:model,isDefault:model==='latest'}));
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command,args) => command==='get_model_source' ? source : command==='list_models' ? {data:live,source,warning:args?.forceRefresh?'官方模型接口返回 HTTP 403，当前显示本地模型目录':undefined} : command==='list_provider_accounts' ? [{agent:'codex',loggedIn:true}] : fallback(command,args));
  await render();
  expect(vi.mocked(call).mock.calls.filter(([command])=>command==='list_models')).toHaveLength(1);
  expect(call).not.toHaveBeenCalledWith('list_models',expect.objectContaining({forceRefresh:true}));
  await act(async()=>{container.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click();});
  expect(container.querySelectorAll('.model-option')).toHaveLength(7);
  expect(container.querySelector('.model-menu-scroll')?.textContent).toContain('latest');
  expect(call).toHaveBeenCalledWith('list_models',{agent:'codex',sessionId:'official-chat',connectionId:'official'});
  await act(async()=>{container.querySelector<HTMLButtonElement>('[aria-label="关闭模型选择"]')!.click();});
  await act(async()=>{container.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click();});
  expect(vi.mocked(call).mock.calls.filter(([command])=>command==='list_models')).toHaveLength(1);
  await act(async()=>{container.querySelector<HTMLButtonElement>('[aria-label="刷新模型"]')!.click();});
  expect(vi.mocked(call).mock.calls.filter(([command])=>command==='list_models')).toHaveLength(2);
  expect(container.querySelectorAll('.model-option')).toHaveLength(7);
  expect(call).toHaveBeenCalledWith('list_models',{agent:'codex',sessionId:'official-chat',connectionId:'official',forceRefresh:true});
  expect(container.querySelector('.model-catalog-warning')?.textContent).toContain('HTTP 403');
});
it('discards a delayed official catalog after changing the conversation account', async () => {
  localStorage.setItem('supercode.session','account-a-chat');
  data.profiles = ['a','b'].map(accountId=>({id:accountId,accountId,agent:'codex',name:accountId,officialAccount:true,model:`saved-${accountId}`,models:[`saved-${accountId}`],current:accountId==='a',hasCredential:false}));
  data.officialAgents=['codex'];
  data.sessions = ['a','b'].map(id=>({id:`account-${id}-chat`,projectId:'existing',title:`账号 ${id}`,agent:'codex',connectionId:id,model:`saved-${id}`,status:'idle',nativeId:null,turnId:null,updatedAt:1}));
  const source = (id: unknown) => ({providerId:'openai',providerName:'OpenAI',mark:'O',connectionName:String(id),available:true});
  let finishA!: (catalog: unknown) => void;
  const fallback=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command,args={}) => {
    if(command==='get_model_source')return source(args.connectionId);
    if(command==='list_provider_accounts')return ['a','b'].map(connectionId=>({agent:'codex',connectionId,loggedIn:true}));
    if(command==='list_models' && args.connectionId==='a')return new Promise(resolve=>{finishA=resolve;});
    if(command==='list_models')return {data:[{id:'live-b',model:'live-b',displayName:'Live B',isDefault:true}],source:source('b')};
    return fallback(command,args);
  });
  await render();
  await act(async()=>{container.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click();});
  await act(async()=>{container.querySelector<HTMLButtonElement>('.session-row[title="账号 b"]')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  // The route changed while A's independent catalog read was still in flight.
  await act(async()=>{finishA({data:[{id:'live-a',model:'live-a',displayName:'Live A',isDefault:true}],source:source('a')});});
  expect(container.querySelector('.model-chip')?.textContent).not.toContain('Live A');
  const close=container.querySelector<HTMLButtonElement>('[aria-label="关闭模型选择"]');
  if(close)await act(async()=>close.click());
  await act(async()=>{container.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click();});
  expect(container.querySelector('.model-menu-scroll')?.textContent).toContain('Live B');
  expect(container.querySelector('.model-menu-scroll')?.textContent).not.toContain('Live A');
  expect(call).toHaveBeenCalledWith('list_models',{agent:'codex',sessionId:'account-b-chat',connectionId:'b'});
});
it('shows turn changes above the composer while running and preserves the separate workspace diff', async () => {
  vi.spyOn(window,'matchMedia').mockImplementation(()=>({matches:false,addEventListener:()=>{},removeEventListener:()=>{}} as unknown as MediaQueryList));
  localStorage.setItem('supercode.context','true'); localStorage.setItem('supercode.workspace-view','changes'); localStorage.setItem('supercode.session','chat');
  data.sessions=[{id:'chat',projectId:'existing',title:'正在修改',agent:'codex',connectionId:'api:claude',model:null,status:'running',nativeId:'native',turnId:'t',updatedAt:1}];
  messages=[{id:'u',seq:1,sessionId:'chat',role:'user',kind:'userMessage',text:'编辑文件',data:{}},{id:'run-t',seq:2,sessionId:'chat',role:'activity',kind:'runMarker',text:'',data:{turnId:'t',status:'inProgress'}}];
  const fallback=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args)=>command==='workspace_changes'?{isGit:true,branch:'main',files:[{path:'old.ts',status:'M'}],diff:'diff --git a/old.ts b/old.ts\n@@ -1 +1 @@\n-old\n+existing'}:fallback(command,args));
  await render(); await act(async()=>{await vi.dynamicImportSettled();});
  const diff='diff --git a/live.ts b/live.ts\n@@ -1 +1 @@\n-before\n+current';
  await act(async()=>{notify({method:'turn/diff/updated',params:{threadId:'native',turnId:'t',diff}});await new Promise(resolve=>setTimeout(resolve,80));await vi.dynamicImportSettled();});
  await act(async()=>{await vi.dynamicImportSettled();});
  const dock=container.querySelector('.composer-area .turn-change-dock')!;
  expect(dock.textContent).toContain('1 个文件已更改');
  expect(dock.compareDocumentPosition(container.querySelector('.composer')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(container.querySelector('.change-file-header')?.textContent).toContain('old.ts');
  await act(async()=>dock.querySelector<HTMLButtonElement>('.turn-change-chip')!.click());
  expect(dock.textContent).toContain('live.ts'); expect(dock.textContent).not.toContain('old.ts');
  await act(async()=>{dock.querySelector<HTMLButtonElement>('[role="menuitem"]')!.click();await vi.dynamicImportSettled();});
  expect(container.querySelector('[aria-label="代码差异"] .change-code-line.added code')?.textContent).toBe('current');
});
it('hides the live changes dock and collapses the process as soon as completion arrives, even with stale history', async () => {
  localStorage.setItem('supercode.session','chat');
  data.sessions=[{id:'chat',projectId:'existing',title:'完成折叠',agent:'claude',connectionId:'api:claude',model:null,status:'running',nativeId:'native',turnId:'t',updatedAt:1}];
  const diff='diff --git a/live.ts b/live.ts\n@@ -0,0 +1 @@\n+new';
  messages=mergeEvent([{id:'u',seq:1,sessionId:'chat',role:'user',kind:'userMessage',text:'编辑文件',data:{}}], {method:'turn/started',params:{turn:{id:'t'}}}, 'chat');
  messages.push(
    {id:'progress',seq:3,sessionId:'chat',role:'assistant',kind:'agentMessage',text:'先检查文件',data:{turnId:'t',status:'completed'}},
    {id:'final',seq:4,sessionId:'chat',role:'assistant',kind:'agentMessage',text:'修改完成',data:{turnId:'t',status:'completed'}},
    {id:'diff-t',seq:5,sessionId:'chat',role:'activity',kind:'turnDiff',text:diff,data:{turnId:'t',status:'completed'}});
  messages=messages.map((message,index)=>({...message,seq:index + 1}));
  await render(); await act(async()=>{await vi.dynamicImportSettled();});
  expect(container.querySelector('.turn-change-dock')).not.toBeNull();
  expect(container.querySelector('.turn-live-content')?.textContent).toContain('先检查文件');
  // Both IPC reads deliberately remain at their pre-completion snapshots.
  await act(async()=>{notify({method:'turn/completed',params:{threadId:'native',turn:{id:'t',status:'completed'}}});await new Promise(resolve=>setTimeout(resolve,30));await vi.dynamicImportSettled();});
  expect(container.querySelector('.composer-area .turn-change-dock')).toBeNull();
  expect(container.querySelector('.turn-process-toggle')?.getAttribute('aria-expanded')).toBe('false');
  expect(container.querySelector('.turn-process-content')).toBeNull();
  expect(container.querySelector('.turn-final-result')?.textContent).toContain('修改完成');
  expect(container.querySelector('[aria-label="本轮文件变更"]')?.textContent).toContain('已编辑 1 个文件');
  await act(async()=>container.querySelector<HTMLButtonElement>('.turn-process-toggle')!.click());
  expect(container.querySelector('.turn-process-content')?.textContent).toContain('先检查文件');
  expect(container.querySelector('.composer-area .turn-change-dock')).toBeNull();
});

it('reloads persisted adapter edit snapshots during a turn without waiting for completion', async () => {
  localStorage.setItem('supercode.session','chat');
  data.sessions=[{id:'chat',projectId:'existing',title:'实时编辑',agent:'claude',connectionId:'api:claude',model:null,status:'running',nativeId:'native',turnId:'t',updatedAt:1}];
  messages=[{id:'u',seq:1,sessionId:'chat',role:'user',kind:'userMessage',text:'编辑文件',data:{}},{id:'run-t',seq:2,sessionId:'chat',role:'activity',kind:'runMarker',text:'',data:{turnId:'t',status:'inProgress'}}];
  await render();
  messages.push({id:'change-t:live.ts',seq:3,sessionId:'chat',role:'activity',kind:'turnDiff',text:'diff --git a/live.ts b/live.ts\n@@ -0,0 +1 @@\n+new',data:{turnId:'t',path:'live.ts'}});
  const before=vi.mocked(call).mock.calls.filter(([command])=>command==='list_messages').length;
  await act(async()=>{
    notify({method:'item/completed',params:{threadId:'native',turnId:'t',item:{id:'edit1',type:'claudeToolCall',tool:'Edit',status:'completed'}}});
    notify({method:'item/completed',params:{threadId:'native',turnId:'t',item:{id:'edit2',type:'claudeToolCall',tool:'Write',status:'completed'}}});
    await new Promise(resolve=>setTimeout(resolve,260));await vi.dynamicImportSettled();
  });
  expect(container.querySelector('.turn-change-dock')?.textContent).toContain('1 个文件已更改');
  expect(vi.mocked(call).mock.calls.filter(([command])=>command==='list_messages')).toHaveLength(before+1);
});
it('keeps browser and file changes independent, including fullscreen and closing either panel', async () => {
  const media = vi.spyOn(window, 'matchMedia').mockImplementation(() => ({matches:false,addEventListener:()=>{},removeEventListener:()=>{}} as unknown as MediaQueryList));
  localStorage.setItem('supercode.context', 'true');
  localStorage.setItem('supercode.session', 'chat');
  data.sessions = [{id:'chat',projectId:'existing',title:'面板测试',agent:'claude',connectionId:'api:claude',model:null,status:'idle',nativeId:null,turnId:null,updatedAt:1}];
  await render();
  expect(container.querySelector('[aria-label="工作空间"]')).not.toBeNull();
  await act(async () => { container.querySelector<HTMLButtonElement>('[aria-label="打开内置浏览器"]')!.click(); await vi.dynamicImportSettled(); await new Promise(resolve=>setTimeout(resolve,30)); });
  const browser = container.querySelector('[aria-label="内置浏览器"]')!;
  expect(browser).not.toBeNull();
  expect(browser.querySelector('.context-tabs')).toBeNull();
  expect(browser.textContent).not.toContain('文件变更');
  expect(browser.parentElement?.classList.contains('conversation-surface')).toBe(true);
  expect(container.querySelector('[aria-label="工作空间"]')?.parentElement).toBe(container.querySelector('.app'));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="全屏查看网页"]')!.click());
  expect(browser.classList.contains('browser-expanded')).toBe(true);
  expect(container.querySelector('[aria-label="工作空间"]')).not.toBeNull();
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="收起工作空间"]')!.click());
  expect(container.querySelector('[aria-label="工作空间"]')).toBeNull();
  expect(container.querySelector('[aria-label="内置浏览器"]')).toBe(browser);
  expect(call).toHaveBeenCalledWith('browser_open', expect.objectContaining({sessionId:'chat'}));
  expect(vi.mocked(call).mock.calls.filter(([command])=>command==='browser_open')).toHaveLength(1);
  expect(call).not.toHaveBeenCalledWith('browser_close', expect.anything());
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="打开工作空间"]')!.click());
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="关闭浏览器"]')!.click());
  expect(container.querySelector('[aria-label="内置浏览器"]')).toBeNull();
  expect(container.querySelector('[aria-label="工作空间"]')).not.toBeNull();
  media.mockRestore();
});
it('docks questions above the composer while keeping approvals in the conversation and restores the composer after answering', async () => {
  pendingRequests = [
    { id: 'question', method: 'item/tool/requestUserInput', params: { questions: [{ id: '0', question: '界面显示正常吗？', options: [{ label: '正常' }] }] } },
    { id: 'approval', method: 'claude/tool/requestApproval', params: { toolName: 'Shell', input: { command: 'example' } } },
  ];
  await render(); await act(async () => { await vi.dynamicImportSettled(); });
  const dock = container.querySelector('.composer-area .question-dock')!;
  expect(dock.querySelector('.question-request')).not.toBeNull();
  expect(container.querySelector('.chat-scroll .question-request')).toBeNull();
  expect(container.querySelector('.chat-scroll .request-card')).not.toBeNull();
  expect(container.querySelector('.composer textarea')?.getAttribute('rows')).toBe('1');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.style.height).toBe('28px');
  expect(dock.compareDocumentPosition(container.querySelector('.composer')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  await act(async () => dock.querySelector<HTMLButtonElement>('.question-request-option')!.click());
  expect(call).toHaveBeenCalledWith('respond_request', { id: 'question', result: { answers: { '0': { answers: ['正常'] } } } });
  expect(container.querySelector('.question-dock')).toBeNull();
  expect(container.querySelector('.composer textarea')?.getAttribute('rows')).toBe('2');
  expect(container.querySelector('.chat-scroll .request-card')).not.toBeNull();
});
it('sends a projectless draft without opening the project picker and keeps it in the direct chat area', async () => {
  await render();
  expect(container.querySelector('[aria-label="选择项目"]')?.textContent).toBe('SuperCode');
  expect(call).not.toHaveBeenCalledWith('create_session', expect.anything());
  const textarea = container.querySelector<HTMLTextAreaElement>('.composer textarea')!;
  await act(async()=>{
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,'直接聊天');
    textarea.dispatchEvent(new Event('input',{bubbles:true}));
  });
  await act(async()=>{container.querySelector<HTMLFormElement>('.composer')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));await new Promise(resolve=>setTimeout(resolve,30));});
  expect(call).toHaveBeenCalledWith('create_session', expect.objectContaining({projectId:''}));
  expect(call).toHaveBeenCalledWith('send_chat_message', expect.objectContaining({sessionId:'standalone',text:'直接聊天'}));
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(container.querySelector('.sidebar-direct [aria-label="最近对话"] .session-row')).not.toBeNull();
  expect(container.querySelector('.sidebar-projects .session-row')).toBeNull();
  expect(localStorage.getItem('supercode.project')).toBe('');
  await act(async()=>container.querySelector<HTMLButtonElement>('[title="会话设置"]')!.click());
  await act(async()=>{ [...container.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent==='归档会话')!.click(); await new Promise(resolve=>setTimeout(resolve,30)); });
  expect(call).toHaveBeenCalledWith('archive_session', {sessionId:'standalone'});
  expect(container.querySelector('.new-chat-welcome')).not.toBeNull();
  await act(async()=>container.querySelector<HTMLButtonElement>('.new-chat')!.click());
  expect(container.querySelector('[aria-label="选择项目"]')?.textContent).toBe('SuperCode');
});

it('starts an unbound draft from the direct chat section even when a project is selected', async () => {
  localStorage.setItem('supercode.project', 'existing');
  await render();
  expect(container.querySelector('[aria-label="选择项目"]')?.textContent).toBe('已有项目');
  expect(container.querySelector('.sidebar-projects')?.textContent).toContain('已有项目');
  expect(container.querySelector('.sidebar-direct')?.textContent).toContain('开始一个新任务');
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="新建任务"]')!.click());
  expect(container.querySelector('[aria-label="选择项目"]')?.textContent).toBe('SuperCode');
  expect(call).not.toHaveBeenCalledWith('create_session', expect.anything());
});

it.each(['claude','codex','opencode','pi'])('prompts before creating or sending an unconfigured %s conversation, with an actionable supplier tab', async agent => {
  localStorage.setItem('supercode.agent',agent); data.profiles = []; providerMissing = true;
  await render();
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  await type('保留我的消息'); await send();
  expect(call).toHaveBeenCalledWith('check_chat_connection',expect.objectContaining({agent,sessionId:null}));
  expect(call).not.toHaveBeenCalledWith('create_session',expect.anything());
  expect(call).not.toHaveBeenCalledWith('send_chat_message',expect.anything());
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  expect(container.querySelector('[role="dialog"][aria-label="添加模型供应商"]')?.textContent).toContain('请先添加模型供应商');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('保留我的消息');
  await act(async()=>{[...container.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent==='去添加')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  // Provider settings loads lazily on its first opening.
  for(let i=0;i<30&&!container.querySelector(`#providers-${agent}-tab`);i++) await act(async()=>{await new Promise(resolve=>setTimeout(resolve,20));});
  expect(container.querySelector(`#providers-${agent}-tab`)?.getAttribute('aria-selected')).toBe('true');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('保留我的消息');
});

it('checks a pinned running chat before queueing and leaves its conversation and draft intact', async () => {
  localStorage.setItem('supercode.session','running'); providerMissing = true; data.profiles = [];
  data.sessions = [{id:'running',projectId:'existing',title:'正在工作',agent:'claude',connectionId:'@local',model:'old-model',status:'running',nativeId:'native',turnId:'turn',updatedAt:1}];
  await render(); await type('下一轮补充'); await send();
  expect(call).toHaveBeenCalledWith('check_chat_connection',{agent:'claude',connectionId:'@local',sessionId:'running'});
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  expect(data.sessions[0].status).toBe('running');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('下一轮补充');
  expect(container.querySelector('[aria-label="添加模型供应商"]')).not.toBeNull();
});

it('switches one composer button between stopping an empty running chat and queueing a draft without interrupting', async () => {
  localStorage.setItem('supercode.session','running');
  data.sessions = [{id:'running',projectId:'existing',title:'正在工作',agent:'claude',connectionId:'api:claude',model:'test-model',status:'running',nativeId:'native',turnId:'turn',updatedAt:1}];
  await render();
  const action = container.querySelector<HTMLButtonElement>('.composer .send-button')!;
  expect(container.querySelectorAll('.composer .send-button')).toHaveLength(1);
  expect(action.type).toBe('button');
  expect(action.getAttribute('aria-label')).toBe('停止生成');
  await type('  ');
  expect(action.getAttribute('aria-label')).toBe('停止生成');
  await type('下一轮补充');
  expect(container.querySelector('.composer .send-button')).toBe(action);
  expect(action.type).toBe('submit');
  expect(action.getAttribute('aria-label')).toBe('加入队列');
  vi.mocked(call).mockClear();
  await act(async () => { action.click(); await new Promise(resolve=>setTimeout(resolve,30)); });
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({sessionId:'running',payload:expect.objectContaining({text:'下一轮补充'})}));
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('');
  expect(action.getAttribute('aria-label')).toBe('停止生成');
  vi.mocked(call).mockClear();
  await act(async () => action.click());
  expect(call).toHaveBeenCalledWith('interrupt_turn',{sessionId:'running'});
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  expect(call).not.toHaveBeenCalledWith('send_chat_message',expect.anything());
});

it('keeps an empty idle chat in send mode when another chat is running, and disables stop until a turn exists', async () => {
  localStorage.setItem('supercode.session','idle');
  data.sessions = [
    {id:'idle',projectId:'existing',title:'空闲聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:2},
    {id:'starting',projectId:'existing',title:'正在启动',agent:'claude',connectionId:'api:claude',model:'test-model',status:'starting',nativeId:null,turnId:null,updatedAt:1},
  ];
  await render();
  expect(container.querySelector('.composer .stop-button')).toBeNull();
  expect(container.querySelector<HTMLButtonElement>('.composer .send-button')?.disabled).toBe(true);
  await act(async () => { container.querySelector<HTMLButtonElement>('.session-row[title="正在启动"]')!.click(); await new Promise(resolve=>setTimeout(resolve,30)); });
  const stop = container.querySelector<HTMLButtonElement>('.composer .send-button')!;
  expect(stop.getAttribute('aria-label')).toBe('停止生成');
  expect(stop.disabled).toBe(true);
  await act(async () => stop.click());
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
});

it('keeps local help usable without a supplier and permits retrying the draft after configuration', async () => {
  data.profiles = []; providerMissing = true;
  await render(); await type('/help'); await send();
  expect(call).not.toHaveBeenCalledWith('check_chat_connection',expect.anything());
  expect(container.querySelector('[role="dialog"][aria-label="添加模型供应商"]')).toBeNull();
  await type('待配置后发送'); await send();
  await act(async()=>{[...container.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent==='去添加')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  for(let i=0;i<30&&!container.querySelector('#providers-claude-tab');i++) await act(async()=>{await new Promise(resolve=>setTimeout(resolve,20));});
  data.profiles = [{id:'api:claude',agent:'claude',name:'Test API',model:'test-model',hasCredential:true,current:true}]; providerMissing = false;
  await act(async()=>{[...container.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent==='刷新连接')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="聊天"]')!.click());
  await send();
  expect(call).toHaveBeenCalledWith('create_session',expect.objectContaining({connectionId:'api:claude'}));
  expect(call).toHaveBeenCalledWith('send_chat_message',expect.objectContaining({text:'待配置后发送'}));
});

it('hides the stale model, supplier icon and quota before sending an unconfigured historical chat', async () => {
  localStorage.setItem('supercode.session','old-chat'); data.profiles=[]; providerMissing=true;
  data.sessions=[{id:'old-chat',projectId:'existing',title:'旧对话',agent:'claude',connectionId:'@local',model:'k3[1M]',status:'idle',nativeId:'native',turnId:null,updatedAt:1}];
  messages=[{id:'history',seq:1,sessionId:'old-chat',role:'assistant',text:'这是之前的回复',kind:'assistantMessage',data:{}}];
  await render();
  expect(container.querySelector('.model-chip')?.textContent).toContain('添加模型供应商');
  expect(container.querySelector('.model-chip')?.outerHTML).not.toMatch(/Kimi|k3|model-provider-mark/);
  expect(container.querySelector('[data-testid="platform-allowance"]')).toBeNull();
  expect(call).not.toHaveBeenCalledWith('list_models',expect.anything());
  expect(container.textContent).toContain('这是之前的回复');
  await type('你好'); await send();
  expect(container.querySelector('[role="dialog"][aria-label="添加模型供应商"]')).not.toBeNull();
  expect(container.querySelector('.model-chip')?.textContent).toContain('添加模型供应商');
  expect(container.querySelector('[data-testid="platform-allowance"]')).toBeNull();
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('你好');
});

it('opens the matching supplier settings from the empty model chip', async () => {
  data.profiles=[];providerMissing=true;localStorage.setItem('supercode.agent','pi');
  await render();
  await act(async()=>{container.querySelector<HTMLButtonElement>('.model-chip')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  for(let i=0;i<30&&!container.querySelector('#providers-pi-tab');i++) await act(async()=>{await new Promise(resolve=>setTimeout(resolve,20));});
  expect(container.querySelector('#providers-pi-tab')?.getAttribute('aria-selected')).toBe('true');
  expect(call).not.toHaveBeenCalledWith('create_session',expect.anything());
});

it('displays models and platform allowance only for the current configured supplier', async () => {
  await render();
  expect(container.querySelector('.model-chip')?.textContent).toContain('test-model');
  expect(container.querySelector('[data-testid="platform-allowance"]')).not.toBeNull();
  expect(call).toHaveBeenCalledWith('get_model_source',expect.objectContaining({agent:'claude',connectionId:'api:claude',sessionId:null}));
});

it('selects an inline skill and toggles planning without losing the message draft or opening settings', async () => {
  localStorage.setItem('supercode.agent', 'claude'); localStorage.setItem('supercode.project', 'inline-skills');
  data.projects = [{ id: 'inline-skills', name: '技能项目', path: 'D:/skills-project' }];
  const original = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args) => command === 'list_local_skills'
    ? { skills: [{ name: 'design', description: '界面设计', path: 'C:/skills/design/SKILL.md' }] } : original(command, args));
  await render(); await type('保留这段消息');
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="添加附件"]')!.click());
  const skill = [...container.querySelectorAll<HTMLButtonElement>('.add-skill-row')].find(button => button.textContent?.includes('design'))!;
  await act(async () => skill.click());
  expect(container.querySelector('.composer textarea')?.getAttribute('aria-label')).toBe('消息');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('保留这段消息');
  expect(container.querySelector('[aria-label="已选择技能"]')?.textContent).toContain('Design');
  expect(container.querySelector('.settings-page')).toBeNull();
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="添加附件"]')!.click());
  await act(async () => container.querySelector<HTMLButtonElement>('.add-actions [role="menuitemcheckbox"]')!.click());
  expect(container.querySelector('[aria-label="权限模式"]')?.textContent).toContain('计划模式');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('保留这段消息');
  expect(call).not.toHaveBeenCalledWith('create_session', expect.anything());
  expect(call).not.toHaveBeenCalledWith('send_chat_message', expect.anything());
});

it('starts an explicitly submitted native Codex goal with current permissions while preserving the normal draft', async () => {
  localStorage.setItem('supercode.agent', 'codex');
  data.profiles = [{ ...data.profiles[0], agent: 'codex', id: 'api:codex' }];
  const original = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args = {}) => {
    if (command === 'execute_agent_command') return { text: '目标已设置', session: data.sessions[0] };
    return original(command, args);
  });
  await render(); await type('尚未发送的草稿');
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="添加附件"]')!.click());
  const goal = container.querySelector<HTMLButtonElement>('[data-goal-entry]')!;
  await act(async () => goal.click());
  expect(call).not.toHaveBeenCalledWith('execute_agent_command', expect.anything());
  const input = container.querySelector<HTMLTextAreaElement>('.add-goal-panel textarea')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, 'clear'); input.dispatchEvent(new Event('input', { bubbles: true })); });
  await act(async () => { container.querySelector<HTMLButtonElement>('.add-goal-actions button')!.click(); await new Promise(resolve => setTimeout(resolve, 30)); });
  expect(call).toHaveBeenCalledWith('execute_agent_command', { sessionId: 'standalone', command: 'goal-create', argument: 'clear', permissionMode: 'ask' });
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('尚未发送的草稿');
  expect(container.querySelector('.command-feedback')?.textContent).toContain('目标已设置');
  expect(call).not.toHaveBeenCalledWith('send_chat_message', expect.anything());
});

it('ignores an earlier supplier response after switching to an unconfigured chat', async () => {
  localStorage.setItem('supercode.session','configured');
  data.sessions=[
    {id:'configured',projectId:'existing',title:'有连接',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:2},
    {id:'missing',projectId:'existing',title:'无连接',agent:'claude',connectionId:'@local',model:'k3[1M]',status:'idle',nativeId:null,turnId:null,updatedAt:1},
  ];
  const original=vi.mocked(call).getMockImplementation()!;
  let finish!: (value:unknown)=>void;
  vi.mocked(call).mockImplementation(async (command,args={})=>{
    if(command==='get_model_source'&&args.sessionId==='configured') return new Promise(resolve=>{finish=resolve;});
    if(command==='get_model_source') return {providerId:'unknown',providerName:'',mark:'',available:false};
    return original(command,args);
  });
  await render();
  await act(async()=>{container.querySelector<HTMLButtonElement>('.session-row[title="无连接"]')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  await act(async()=>{finish({providerId:'kimi',providerName:'Kimi Code',mark:'K',available:true});await new Promise(resolve=>setTimeout(resolve,30));});
  expect(container.querySelector('.model-chip')?.outerHTML).not.toMatch(/Kimi|k3/);
  expect(container.querySelector('[data-testid="platform-allowance"]')).toBeNull();
  expect(call).not.toHaveBeenCalledWith('list_models',expect.objectContaining({sessionId:'configured'}));
  expect(container.querySelector('.session-row[title="无连接"]')?.getAttribute('aria-current')).toBe('page');
});
