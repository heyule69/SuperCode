// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { call } from './api';
import type { Bootstrap, Message, Session, RpcEvent } from './types';
import { PROVIDER_REQUIRED } from './chatConnection';

vi.mock('./api', () => ({ desktop:true, call:vi.fn(), subscribe:vi.fn(async()=>()=>{}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen:vi.fn(async()=>()=>{}) }));
vi.mock('./windowTheme', () => ({ watchWindowTheme:()=>()=>{} }));
vi.mock('./DesktopTitleBar', () => ({ DesktopTitleBar:()=>null }));
vi.mock('./PlatformUsageView', () => ({ PlatformUsageIndicator:()=> <span data-testid="platform-allowance">供应商额度</span> }));
vi.mock('./AppUpdate', () => ({ default:()=>null, AppUpdateNotice:()=>null }));
let root: Root, container: HTMLDivElement, data: Bootstrap, messages: Message[], providerMissing: boolean;
let pendingRequests: RpcEvent[];
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); vi.mocked(call).mockClear(); messages = []; pendingRequests = []; providerMissing = false;
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
    if (command==='followup_capabilities') return {};
    if (command==='list_followups') return [];
    if (command==='workspace_changes') return { files:[],diff:'',branch:'',isGit:false };
    if (command==='create_session') {
      const chat: Session = { id:'standalone',projectId:String(args.projectId),workspacePath:'C:/Documents/SuperCode/chat',title:'新会话',agent:String(args.agent),model:null,connectionId:String(args.connectionId),nativeId:null,status:'idle',turnId:null,updatedAt:1 };
      data.sessions.push(chat); return chat;
    }
    if (command==='archive_session') { data.sessions = []; return null; }
    if (command==='send_chat_message') {
      messages = [{ id:'saved',seq:1,sessionId:'standalone',role:'user',text:String(args.text),kind:'userMessage',data:{} }];
      return data.sessions[0];
    }
    return null;
  });
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async()=>root.unmount()); container.remove(); });
async function render() { await act(async()=>{root.render(<App/>); await new Promise(resolve=>setTimeout(resolve,30));}); }
async function type(text: string) {
  const textarea = container.querySelector<HTMLTextAreaElement>('.composer textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,text);textarea.dispatchEvent(new Event('input',{bubbles:true}));});
}
async function send() {
  await act(async()=>{container.querySelector<HTMLFormElement>('.composer')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));await new Promise(resolve=>setTimeout(resolve,30));});
}
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
  await act(async () => dock.querySelector<HTMLButtonElement>('.question-request-send')!.click());
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
  expect(container.querySelector('.sidebar-direct')?.textContent).toContain('暂无对话');
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="新建直接对话"]')!.click());
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
