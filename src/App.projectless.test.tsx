// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { call, subscribe } from './api';
import type { Bootstrap, Message, Session, RpcEvent } from './types';
import { PROVIDER_REQUIRED } from './chatConnection';
import { mergeEvent } from './events';
import type { Followup } from './Followups';
import { PlatformUsageIndicator } from './PlatformUsageView';
import { AppUpdateNotice } from './AppUpdate';
import { readWorkspaceDraft } from './workspaceDrafts';
import { loadModelSelection, saveModelSelection } from './modelSelection';

vi.mock('./api', () => ({ desktop:true, call:vi.fn(), subscribe:vi.fn(async()=>()=>{}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen:vi.fn(async()=>()=>{}) }));
vi.mock('./windowTheme', () => ({ watchWindowTheme:()=>()=>{} }));
vi.mock('./DesktopTitleBar', () => ({ DesktopTitleBar:()=>null }));
vi.mock('./PlatformUsageView', () => ({ PlatformUsageIndicator:vi.fn(()=> <span data-testid="platform-allowance">供应商额度</span>) }));
vi.mock('./AppUpdate', () => ({ default:()=>null, AppUpdateNotice:vi.fn(()=>null) }));
let root: Root, container: HTMLDivElement, data: Bootstrap, messages: Message[], providerMissing: boolean;
let pendingRequests: RpcEvent[];
let notify: (event: RpcEvent) => void;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); vi.mocked(call).mockClear(); messages = []; pendingRequests = []; providerMissing = false;
  vi.mocked(AppUpdateNotice).mockReset();
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
afterEach(async () => { await act(async()=>root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function render() { await act(async()=>{root.render(<App/>); await new Promise(resolve=>setTimeout(resolve,30));}); }
async function type(text: string) {
  const textarea = container.querySelector<HTMLTextAreaElement>('.composer textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,text);textarea.dispatchEvent(new Event('input',{bubbles:true}));});
}
async function send() {
  await act(async()=>{container.querySelector<HTMLFormElement>('.composer')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));await new Promise(resolve=>setTimeout(resolve,30));});
}

it('opens and closes the model picker without asking the browser to scroll the chat for focus', async () => {
  await render();
  const focus = vi.spyOn(HTMLElement.prototype, 'focus');
  const trigger = container.querySelector<HTMLButtonElement>('.model-chip')!;
  await act(async () => trigger.click());
  const search = container.querySelector<HTMLInputElement>('[aria-label="搜索模型菜单"]')!;
  expect(document.activeElement).toBe(search);
  const searchFocus = focus.mock.contexts.findIndex(element => element === search);
  expect(focus.mock.calls[searchFocus]).toEqual([{ preventScroll: true }]);
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="关闭模型选择"]')!.click());
  expect(document.activeElement).toBe(trigger);
  expect(focus.mock.calls.at(-1)).toEqual([{ preventScroll: true }]);
});

it('keeps the model popup inside a narrow chat pane so it cannot create a horizontal scrollbar', async () => {
  await render();
  const workspace = container.querySelector<HTMLElement>('.workspace')!;
  const trigger = container.querySelector<HTMLButtonElement>('.model-chip')!;
  const anchor = trigger.parentElement!;
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this === workspace) return new DOMRect(300,40,320,674);
    if (this === anchor) return new DOMRect(550,600,150,28);
    if (this.classList.contains('model-popover')) return new DOMRect(550+parseFloat(this.style.left || '0'),300,Math.min(400,parseFloat(this.style.maxWidth || '400')),280);
    return original.call(this);
  });
  await act(async () => trigger.click());
  const popup = container.querySelector<HTMLElement>('.model-popover')!;
  const bounds = popup.getBoundingClientRect(), host = workspace.getBoundingClientRect();
  expect(bounds.left).toBeGreaterThanOrEqual(host.left+12);
  expect(bounds.right).toBeLessThanOrEqual(host.right-12);
  expect(popup.querySelector('.model-option')).not.toBeNull();
  expect(workspace.scrollLeft).toBe(0);
});

it('renders the first settings page immediately and keeps the hidden chat geometry for returning', async () => {
  await render();
  const app = container.querySelector<HTMLElement>('.app')!;
  const columns = app.style.gridTemplateColumns;
  const textarea = container.querySelector<HTMLTextAreaElement>('.composer textarea')!;
  await act(async () => {
    flushSync(() => container.querySelector<HTMLButtonElement>('button[aria-label="设置"]')!.click());
    expect(container.querySelector('.settings-page h1')?.textContent).toBe('设置');
    expect(container.querySelector('.settings-page-heading h2')?.textContent).toBe('常规');
    expect(container.querySelector('.settings-loading')).toBeNull();
  });
  expect(app.style.gridTemplateColumns).toBe(columns);
  await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="聊天"]')!.click());
  expect(app.style.gridTemplateColumns).toBe(columns);
  expect(container.querySelector('.composer textarea')).toBe(textarea);
  expect(textarea.style.height).toBe('48px');
});

it('remembers an explicit model and reasoning choice immediately and restores it in a fresh window', async () => {
  data.profiles[0].models = ['test-model', 'last-picked'];
  const base = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={}) => command === 'list_models' ? {data:['test-model','last-picked'].map(model => ({id:model,model,displayName:model,isDefault:model==='test-model',supportedReasoningEfforts:[{reasoningEffort:'low'},{reasoningEffort:'high'}]})),source:{providerId:'custom',providerName:'Test API',mark:'T',available:true}} : base(command,args));
  await render();
  await act(async()=>container.querySelector<HTMLButtonElement>('.model-chip')!.click());
  await act(async()=>[...container.querySelectorAll<HTMLButtonElement>('.model-option')].find(button=>button.textContent?.includes('last-picked'))!.click());
  await act(async()=>container.querySelector<HTMLButtonElement>('.model-chip')!.click());
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="选择推理强度"]')!.click());
  await act(async()=>[...container.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].find(button=>button.textContent==='高')!.click());
  expect(loadModelSelection('claude',data.profiles,[])).toEqual({connectionId:'api:claude',model:'last-picked',effort:'high'});
  await act(async()=>root.unmount());
  localStorage.setItem('supercode.draft.v2:%3Aclaude',JSON.stringify({version:2,updatedAt:1,draft:{text:'保留草稿',attachments:[],selection:{connectionId:'api:claude',model:'test-model',effort:'low'}}}));
  vi.stubGlobal('location',new URL('http://localhost/?window=new'));
  root=createRoot(container); await render();
  expect(container.querySelector('.model-chip-name')?.textContent).toBe('last-picked');
  expect(container.querySelector('.model-chip-effort')?.textContent).toBe('高');
  await type('新窗口使用上次选择'); await send();
  expect(call).toHaveBeenCalledWith('send_chat_message',expect.objectContaining({model:'last-picked',effort:'high'}));
});

it('starts a new chat with the current Agent and its remembered connection instead of configured defaults', async () => {
  localStorage.setItem('supercode.session','pi-chat');
  data.profiles.push({id:'api:pi',agent:'pi',name:'Pi API',model:'pi-default',models:['pi-picked'],hasCredential:true,current:true});
  data.sessions=[{id:'pi-chat',projectId:'existing',title:'Pi 会话',agent:'pi',connectionId:'api:pi',model:'pi-default',status:'idle',nativeId:null,turnId:null,updatedAt:1}];
  saveModelSelection('pi',{connectionId:'api:pi',model:'pi-picked',effort:'high'});
  const base = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={}) => command === 'list_models' && args.agent === 'pi' ? {data:['pi-default','pi-picked'].map(model=>({id:model,model,displayName:model,isDefault:model==='pi-default',supportedReasoningEfforts:[{reasoningEffort:'high'}]}))} : base(command,args));
  await render();
  expect(container.querySelector('.model-chip-name')?.textContent).toBe('pi-default');
  await act(async()=>window.dispatchEvent(new KeyboardEvent('keydown',{key:'n',ctrlKey:true,bubbles:true,cancelable:true})));
  expect(container.querySelector('.model-chip-name')?.textContent).toBe('pi-picked');
  expect(container.querySelector('.model-chip-effort')?.textContent).toBe('高');
  await type('沿用 Pi'); await send();
  expect(call).toHaveBeenCalledWith('create_session',expect.objectContaining({agent:'pi',connectionId:'api:pi',model:'pi-picked'}));
});

it('falls back to a catalog default without retaining the removed model reasoning override', async () => {
  saveModelSelection('claude',{connectionId:'api:claude',model:'removed-model',effort:'max'});
  await render(); await type('使用可用模型'); await send();
  expect(call).toHaveBeenCalledWith('send_chat_message',expect.objectContaining({model:'test-model',effort:null}));
});

it('restores the reasoning level of an existing chat without replacing it with the new-chat preference', async () => {
  localStorage.setItem('supercode.session','saved-effort');
  data.sessions=[{id:'saved-effort',projectId:'existing',title:'已有聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:1}];
  localStorage.setItem('supercode.draft.v2:saved-effort',JSON.stringify({version:2,updatedAt:1,draft:{text:'',attachments:[],selection:{connectionId:'api:claude',model:'test-model',effort:'max'}}}));
  saveModelSelection('claude',{connectionId:'api:claude',model:'another-model',effort:'low'});
  await render();
  expect(container.querySelector('.model-chip-name')?.textContent).toBe('test-model');
  expect(container.querySelector('.model-chip-effort')?.textContent).toBe('最高');
  await type('继续当前聊天'); await send();
  expect(call).toHaveBeenCalledWith('send_chat_message',expect.objectContaining({model:'test-model',effort:'max'}));
});

it('does not replace the remembered model when switching an existing chat fails', async () => {
  localStorage.setItem('supercode.session','model-chat');
  data.sessions=[{id:'model-chat',projectId:'existing',title:'模型会话',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:1}];
  data.profiles[0].models=['test-model','failed-model'];
  saveModelSelection('claude',{connectionId:'api:claude',model:'test-model',effort:'high'});
  const base = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(command==='switch_session_model')throw new Error('switch failed');
    if(command==='list_models')return {data:['test-model','failed-model'].map(model=>({id:model,model,displayName:model,isDefault:model==='test-model'}))};
    return base(command,args);
  });
  await render();
  await act(async()=>container.querySelector<HTMLButtonElement>('.model-chip')!.click());
  await act(async()=>[...container.querySelectorAll<HTMLButtonElement>('.model-option')].find(button=>button.textContent?.includes('failed-model'))!.click());
  expect(loadModelSelection('claude',data.profiles,[])).toEqual({connectionId:'api:claude',model:'test-model',effort:'high'});
});

it('does not resurrect a resolved approval when initial recovery returns late', async () => {
  const original = vi.mocked(call).getMockImplementation()!;
  let finish!: (requests: RpcEvent[]) => void;
  vi.mocked(call).mockImplementation((command, args) => command === 'pending_requests' ? new Promise(resolve => { finish = resolve; }) : original(command, args));
  await render();
  const request: RpcEvent = { id: 'resolved-during-load', method: 'claude/tool/requestApproval', params: { toolName: 'Shell', input: { command: 'example' } } };
  await act(async () => { notify(request); notify({ method: 'serverRequest/resolved', params: { requestId: request.id } }); finish([request]); await vi.dynamicImportSettled(); });
  expect(container.querySelector('.request-card')).toBeNull();
});

it.each(['pi','opencode','gemini'])('rebinds %s provisional native startup and keeps its boot approval visible by exact owner', async agent => {
  localStorage.setItem('supercode.session','native-chat');
  data.sessions = [{id:'native-chat',projectId:'existing',title:'原生启动',agent,connectionId:`api:${agent}`,model:'test-model',status:'starting',nativeId:'boot-native',turnId:null,updatedAt:1}];
  data.profiles.push({id:`api:${agent}`,agent,name:'Native API',model:'test-model',hasCredential:true,current:true});
  pendingRequests = [{id:'boot-approval',method:'claude/tool/requestApproval',params:{appSessionId:'native-chat',threadId:'different-boot-alias',toolName:'BOOT_APPROVAL',input:{}}}];
  await render(); await act(async () => {await vi.dynamicImportSettled();});
  expect(container.querySelector('.request-card')?.textContent).toContain('BOOT_APPROVAL');
  data.sessions[0] = {...data.sessions[0],nativeId:'actual-native',status:'running',turnId:'native-turn'};
  await act(async () => {
    notify({method:'thread/started',params:{appSessionId:'native-chat',thread:{id:'actual-native'}}});
    notify({method:'turn/started',params:{appSessionId:'native-chat',threadId:'actual-native',turn:{id:'native-turn'}}});
    notify({method:'item/agentMessage/delta',params:{appSessionId:'native-chat',threadId:'actual-native',turnId:'native-turn',itemId:'native-answer',delta:'正式线程实时输出'}});
    notify({id:'unrelated',method:'claude/tool/requestApproval',params:{appSessionId:'other-chat',threadId:'actual-native',toolName:'OTHER_OWNER',input:{}}});
    await new Promise(resolve=>setTimeout(resolve,80)); await vi.dynamicImportSettled();
  });
  expect(container.textContent).toContain('正式线程实时输出');
  expect(container.querySelector('.request-card')?.textContent).toContain('BOOT_APPROVAL');
  expect(container.textContent).not.toContain('OTHER_OWNER');
});

it('keeps a new approval when reconnect recovery returns an older empty snapshot', async () => {
  await render();
  const original = vi.mocked(call).getMockImplementation()!;
  let finish!: (requests: RpcEvent[]) => void;
  vi.mocked(call).mockImplementation((command, args) => command === 'pending_requests' ? new Promise(resolve => { finish = resolve; }) : original(command, args));
  const stopped = vi.mocked(subscribe).mock.calls.at(-1)![2];
  await act(async () => { stopped(); notify({ id: 'new-during-reconnect', method: 'claude/tool/requestApproval', params: { toolName: 'NEW_APPROVAL', input: {} } }); finish([]); await vi.dynamicImportSettled(); });
  await act(async () => { await vi.dynamicImportSettled(); });
  expect(container.querySelector('.request-card')?.textContent).toContain('NEW_APPROVAL');
});

it('blocks Enter and submit while a model handoff is pending, then sends the selected model', async () => {
  localStorage.setItem('supercode.session', 'model-chat');
  data.sessions = [{ id:'model-chat',projectId:'existing',title:'模型聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:'native',turnId:null,updatedAt:1 }];
  data.profiles[0].models = ['test-model', 'next-model'];
  const original = vi.mocked(call).getMockImplementation()!; let finish!: (session: Session) => void;
  vi.mocked(call).mockImplementation((command, args) => command === 'switch_session_model' ? new Promise(resolve => { finish = resolve; }) : command === 'list_models' ? Promise.resolve({data:[{id:'test-model',model:'test-model',displayName:'test-model',isDefault:true},{id:'next-model',model:'next-model',displayName:'next-model',isDefault:false}],source:{providerId:'custom',providerName:'Test API',mark:'T',available:true}}) : original(command, args));
  await render(); await type('切换后发送这条消息');
  await act(async () => container.querySelector<HTMLButtonElement>('.model-chip')!.click());
  await act(async () => [...container.querySelectorAll<HTMLButtonElement>('.model-option')].find(button => button.textContent?.includes('next-model'))!.click());
  expect(container.querySelector<HTMLButtonElement>('.send-button')!.disabled).toBe(true);
  await act(async () => {
    container.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key:'Enter', bubbles:true, cancelable:true }));
    container.querySelector('form.composer')!.dispatchEvent(new Event('submit', { bubbles:true, cancelable:true }));
  });
  expect(call).not.toHaveBeenCalledWith('send_chat_message', expect.anything());
  expect(call).not.toHaveBeenCalledWith('enqueue_followup', expect.anything());
  data.sessions[0] = { ...data.sessions[0], model:'next-model' };
  await act(async () => { finish(data.sessions[0]); await vi.dynamicImportSettled(); });
  await send();
  expect(call).toHaveBeenCalledWith('send_chat_message', expect.objectContaining({ model:'next-model' }));
});

it('restores a selected chat draft outside the twenty-row memory cache', async () => {
  localStorage.setItem('supercode.session', 'older-chat');
  data.sessions = [{ id:'older-chat',projectId:'existing',title:'旧聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:1 }];
  localStorage.setItem('supercode.draft.v2:older-chat', JSON.stringify({version:2,updatedAt:1,draft:{text:'仍保留的旧草稿',attachments:[]}}));
  for (let index=0;index<25;index++) localStorage.setItem(`supercode.draft.v2:newer-${index}`, JSON.stringify({version:2,updatedAt:index+2,draft:{text:'其他草稿',attachments:[]}}));
  await render();
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('仍保留的旧草稿');
});

it('shows a draft persistence warning on quota failure, retains the draft and clears it after retry', async () => {
  await render();
  const original = localStorage;
  vi.stubGlobal('localStorage', { get length(){return original.length;},key:(index:number)=>original.key(index),getItem:(key:string)=>original.getItem(key),removeItem:(key:string)=>original.removeItem(key),clear:()=>original.clear(),setItem:(key:string,value:string)=>{if(key.startsWith('supercode.draft.v2:'))throw new Error('quota');original.setItem(key,value);} } satisfies Storage);
  await type('磁盘未保存时仍要保留的草稿');
  await act(async () => { await new Promise(resolve=>setTimeout(resolve,280)); });
  expect(container.querySelector('.draft-persistence-error')?.textContent).toContain('草稿暂未保存到磁盘');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('磁盘未保存时仍要保留的草稿');
  vi.unstubAllGlobals();
  await act(async () => container.querySelector<HTMLButtonElement>('.draft-persistence-error button')!.click());
  expect(container.querySelector('.draft-persistence-error')).toBeNull();
  expect(readWorkspaceDraft(':claude')?.text).toBe('磁盘未保存时仍要保留的草稿');
});

it('flushes the current draft before unload without overwriting a draft saved by another window', async () => {
  await render(); await type('刚输入的新草稿');
  localStorage.setItem('supercode.draft.v2:other-chat', JSON.stringify({ version:2,updatedAt:Date.now(),draft:{text:'另一窗口的草稿',attachments:[]} }));
  window.dispatchEvent(new Event('beforeunload'));
  expect(readWorkspaceDraft(':claude')?.text).toBe('刚输入的新草稿');
  expect(readWorkspaceDraft('other-chat')?.text).toBe('另一窗口的草稿');
});

it('isolates selected-content details, traps focus, blocks shortcuts and closes with Escape', async () => {
  localStorage.setItem('supercode.session', 'selection-chat');
  data.sessions = [{ id:'selection-chat',projectId:'existing',title:'选中文字',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:'native',turnId:null,updatedAt:1 }];
  messages = [{ id:'assistant',seq:1,sessionId:'selection-chat',role:'assistant',kind:'assistantMessage',text:'需要进一步查看的文字',data:null }];
  await render();
  const range = document.createRange(); range.selectNodeContents(container.querySelector('.message.assistant .message-content')!);
  Object.defineProperty(range, 'getBoundingClientRect', { value:() => new DOMRect(100,100,100,20) });
  window.getSelection()!.addRange(range);
  await act(async () => container.querySelector('.chat-scroll')!.dispatchEvent(new MouseEvent('mouseup', { bubbles:true })));
  await act(async () => [...container.querySelectorAll<HTMLButtonElement>('.selection-actions button')].find(button => button.textContent === '更多详情')!.click());
  const dialog = container.querySelector<HTMLElement>('[aria-label="选中内容"]')!;
  expect(dialog.contains(document.activeElement)).toBe(true);
  expect(container.querySelector<HTMLElement>('main.workspace')?.inert).toBe(true);
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key:'n',ctrlKey:true,bubbles:true,cancelable:true })));
  expect(container.querySelector('[aria-label="选中内容"]')).toBe(dialog);
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key:'Escape',bubbles:true,cancelable:true })));
  expect(container.querySelector('[aria-label="选中内容"]')).toBeNull();
  expect(container.querySelector<HTMLElement>('main.workspace')?.inert).toBe(false);
});

it('opens only a side-chat question for selected user text without adding or sending it', async () => {
  localStorage.setItem('supercode.session', 'selection-chat');
  data.sessions = [{ id:'selection-chat',projectId:'existing',title:'选中文字',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:'native',turnId:null,updatedAt:1 }];
  messages = [{ id:'user',seq:1,sessionId:'selection-chat',role:'user',kind:'userMessage',text:'需要侧边提问的文字',data:null }];
  await render(); await type('保留主输入草稿');
  const range = document.createRange(); range.selectNodeContents(container.querySelector('.message.user .message-text')!);
  Object.defineProperty(range, 'getBoundingClientRect', { value:() => new DOMRect(100,100,100,20) });
  window.getSelection()!.addRange(range);
  await act(async () => container.querySelector('.chat-scroll')!.dispatchEvent(new MouseEvent('mouseup', { bubbles:true })));
  expect([...container.querySelectorAll('.selection-actions button')].map(button => button.textContent)).toEqual(['在侧边聊天中提问']);
  await act(async () => container.querySelector<HTMLButtonElement>('.selection-actions button')!.click());
  await act(async () => { await vi.dynamicImportSettled(); });
  expect(container.querySelector<HTMLTextAreaElement>('[aria-label="侧边聊天消息"]')!.value).toContain('需要侧边提问的文字');
  expect(container.querySelector<HTMLTextAreaElement>('.workspace .composer textarea')!.value).toBe('保留主输入草稿');
  expect(vi.mocked(call).mock.calls.some(([command]) => command === 'send_chat_message')).toBe(false);
});

it('blocks workspace and sidebar shortcuts while the update dialog is open', async () => {
  localStorage.setItem('supercode.session', 'update-chat');
  data.sessions = [{ id:'update-chat',projectId:'existing',title:'更新时的聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:1 }];
  await render();
  const changed = vi.mocked(AppUpdateNotice).mock.calls.at(-1)![0].dialogChanged!;
  await act(async () => changed(true));
  expect(container.querySelector<HTMLElement>('.desktop-shell')?.inert).toBe(true);
  await act(async () => {
    for (const options of [{key:'r',altKey:true},{key:'p',altKey:true},{key:'u',shiftKey:true},{key:'a',shiftKey:true},{key:'k'}]) {
      window.dispatchEvent(new KeyboardEvent('keydown', { ...options, ctrlKey:true, bubbles:true, cancelable:true }));
    }
  });
  expect(call).not.toHaveBeenCalledWith('archive_session', expect.anything());
  expect(call).not.toHaveBeenCalledWith('update_sidebar_item', expect.anything());
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  await act(async () => changed(false));
  expect(container.querySelector<HTMLElement>('.desktop-shell')?.inert).toBe(false);
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', {key:'a',ctrlKey:true,shiftKey:true,bubbles:true,cancelable:true})));
  expect(call).toHaveBeenCalledWith('archive_session', { sessionId:'update-chat' });
});

it.each(['waiting','live-child'])('only locks the supplier used by a background %s and unlocks it when that task finishes', async activity => {
  localStorage.setItem('supercode.session', 'idle-chat');
  data.profiles.push({id:'running-profile',agent:'claude',name:'运行中的连接',model:'test-model',hasCredential:true,current:false});
  data.sessions = [
    { id:'idle-chat',projectId:'existing',title:'当前聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:1 },
    { id:'background',projectId:'existing',title:'后台任务',agent:'claude',connectionId:'running-profile',model:'test-model',status:activity === 'waiting' ? 'waiting' : 'idle',nativeId:'background-native',turnId:activity === 'waiting' ? 'background-turn' : null,updatedAt:1 },
  ];
  data.runningConnections = activity === 'live-child' ? {claude:['running-profile']} : {};
  const original = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args={}) => command === 'get_provider_profile' ? {id:args.id,agent:'claude',name:'运行中的连接',providerId:'custom',plan:'anthropic',protocol:'anthropic',baseUrl:'https://example.com',model:'test-model',models:['test-model'],hasCredential:true} : original(command,args));
  await render();
  await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', {key:',',ctrlKey:true,bubbles:true,cancelable:true})));
  await act(async () => { await vi.dynamicImportSettled(); });
  const suppliers = [...container.querySelectorAll<HTMLButtonElement>('.settings-nav button')].find(button => button.textContent === '模型供应商')!;
  await act(async () => suppliers.click()); await act(async () => { await vi.dynamicImportSettled(); });
  expect(container.querySelector<HTMLButtonElement>('.settings-page-heading .primary-button')?.disabled).toBe(false);
  const used = container.querySelector<HTMLButtonElement>('[data-connection-id="running-profile"] .saved-provider')!;
  expect(used.textContent).toContain('任务使用中');
  expect(container.querySelector('[data-connection-id="api:claude"]')?.textContent).not.toContain('任务使用中');
  await act(async () => used.click());
  expect(container.querySelector<HTMLInputElement>('#provider-key')?.disabled).toBe(true);
  data.sessions[1].status = 'idle'; data.sessions[1].turnId = null;
  data.runningConnections = {};
  await act(async () => [...container.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === '刷新连接')!.click());
  expect(container.querySelector<HTMLInputElement>('#provider-key')?.disabled).toBe(false);
  expect(data.sessions[0].connectionId).toBe('api:claude');
  expect(call).not.toHaveBeenCalledWith('switch_session_model', expect.anything());
});

it.each([['execute','ask'],['execute','full'],['execute','auto'],['revise','full']] as const)('submits a reviewed native plan as %s preserving %s permission and the composer draft',async (decision,remembered)=>{
  localStorage.setItem('supercode.session','planned');
  localStorage.setItem('supercode.permissions.v2',JSON.stringify({codex:'read'}));
  localStorage.setItem('supercode.execution-permissions.v1',JSON.stringify({codex:remembered}));
  data.profiles=[{...data.profiles[0],id:'api:codex',agent:'codex'}];
  data.sessions=[{id:'planned',projectId:'existing',title:'原生计划',agent:'codex',connectionId:'api:codex',model:'test-model',status:'idle',nativeId:'native',turnId:null,updatedAt:1}];
  messages=[{id:'u',seq:1,sessionId:'planned',role:'user',text:'先规划',kind:'userMessage',data:{}},{id:'p',seq:2,sessionId:'planned',role:'assistant',text:'# 完整计划\n\n先校验再合并，包含边界验收。',kind:'plan',data:{status:'completed',turnId:'t'}},{id:'r',seq:3,sessionId:'planned',role:'activity',text:'',kind:'runMarker',data:{status:'completed',turnId:'t'}}];
  await act(async()=>{await import('./ProposedPlan');});
  await render();await type('尚未发送的自己的草稿');
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  await act(async()=>{container.querySelector<HTMLButtonElement>(decision==='execute'?'.plan-request-execute':'.plan-request-revise')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  expect(call).toHaveBeenCalledWith('enqueue_followup',{sessionId:'planned',payload:expect.objectContaining({text:decision==='execute'?'请按上一条已确认的计划开始执行。':'请继续修改上一条计划，保持计划模式，暂不执行。',permissionMode:decision==='execute'?remembered:'read',readOnly:decision==='revise',attachments:[]})});
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('尚未发送的自己的草稿');
});

it.each([['full','full'],['auto','auto'],['ask','edit']] as const)('continues a confirmed Claude plan with %s execution permission as %s',async (remembered,expected)=>{
  localStorage.setItem('supercode.session','planned-claude');
  localStorage.setItem('supercode.permissions.v2',JSON.stringify({claude:'read'}));
  localStorage.setItem('supercode.execution-permissions.v1',JSON.stringify({claude:remembered}));
  data.sessions=[{id:'planned-claude',projectId:'existing',title:'确认计划',agent:'claude',connectionId:'api:claude',model:'test-model',status:'waiting',nativeId:'native',turnId:'turn',updatedAt:1}];
  pendingRequests=[{id:'claude:planned-claude:plan',method:'claude/tool/requestApproval',params:{threadId:'native',toolName:'ExitPlanMode',input:{plan:'# 页面计划\n\n创建 HTML 并校验。'}}}];
  await act(async()=>{await import('./PlanRequest');await import('./Markdown');});
  await render();
  await act(async()=>{await vi.dynamicImportSettled();});
  expect(call).not.toHaveBeenCalledWith('respond_request',expect.anything());
  await act(async()=>container.querySelector<HTMLButtonElement>('.plan-request-execute')!.click());
  expect(call).toHaveBeenCalledWith('respond_request',{id:'claude:planned-claude:plan',result:{decision:'accept',executionPermission:expected}});
  expect(JSON.parse(localStorage.getItem('supercode.permissions.v2')!).claude).toBe(expected);
});

it('restores full access when leaving the temporary planning menu',async()=>{
  localStorage.setItem('supercode.permissions.v2',JSON.stringify({claude:'full'}));
  await render();
  for(const mode of ['read','full']){
    await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="添加附件"]')!.click());
    await act(async()=>{await vi.dynamicImportSettled();});
    const button=Array.from(container.querySelectorAll<HTMLButtonElement>('[role="menuitemcheckbox"]')).find(b=>b.textContent?.includes('计划模式'))!;
    await act(async()=>button.click());
    expect(JSON.parse(localStorage.getItem('supercode.permissions.v2')!).claude).toBe(mode);
  }
});

function queuedMessages() {
  localStorage.setItem('supercode.session','running');
  data.sessions=[{id:'running',projectId:'existing',title:'正在工作',agent:'claude',connectionId:'api:claude',model:'test-model',status:'running',nativeId:'native',turnId:'turn',updatedAt:1}];
  const rows:Followup[]=['first','second'].map((id,index)=>({id,sessionId:'running',status:'queued',error:null,payload:{text:index?'第二条':'第一条原文',model:'test-model',readOnly:false,permissionMode:'ask',effort:'high',attachments:index?[]:[{kind:'skill',name:'Recall',path:'D:/skills/recall/SKILL.md'}]}}));
  const base=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(command==='list_followups')return structuredClone(rows.filter(row=>row.sessionId===args.sessionId));
    if(command==='begin_followup_edit'){const row=rows.find(row=>row.id===args.id)!;const original=structuredClone(row);row.status='editing';return {row:original,token:'edit-token'};}
    if(command==='finish_followup_edit'){const row=rows.find(row=>row.id===args.id)!;if(args.payload)row.payload=structuredClone(args.payload) as Followup['payload'];row.status='queued';return null;}
    return base(command,args);
  });
  return rows;
}

it('opens a complete plan in the right pane while keeping confirmation and the unsent draft in the composer',async()=>{
  localStorage.setItem('supercode.session','planned');
  data.profiles=[{...data.profiles[0],id:'api:codex',agent:'codex'}];
  data.sessions=[{id:'planned',projectId:'existing',title:'原生计划',agent:'codex',connectionId:'api:codex',model:'test-model',status:'idle',nativeId:'native',turnId:null,updatedAt:1}];
  const fullPlan='# 长计划\n\n'+Array.from({length:30},(_,i)=>`## 阶段 ${i+1}\n\n完整实施与验收内容。`).join('\n\n')+'\n\n最后的边界验收。';
  messages=[{id:'u',seq:1,sessionId:'planned',role:'user',text:'先规划',kind:'userMessage',data:{}},{id:'p',seq:2,sessionId:'planned',role:'assistant',text:fullPlan,kind:'plan',data:{status:'completed',turnId:'t'}},{id:'r',seq:3,sessionId:'planned',role:'activity',text:'',kind:'runMarker',data:{status:'completed',turnId:'t'}}];
  await render(); await type('保留这段草稿');
  expect(container.querySelector('.messages .plan-decision')).toBeNull();
  expect(container.querySelector('.composer-area .plan-decision')).not.toBeNull();
  await act(async()=>{container.querySelector<HTMLButtonElement>('[aria-label="在侧边栏查看完整计划"]')!.click(); await vi.dynamicImportSettled();});
  const document=container.querySelector('.workspace-plan-preview [aria-label="完整计划文档"]');
  expect(document?.querySelectorAll('h2')).toHaveLength(30);
  expect(document?.textContent).toContain('最后的边界验收');
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('保留这段草稿');
  await act(async()=>container.querySelector<HTMLButtonElement>('[title="关闭计划"]')!.click());
  expect(container.querySelector('.workspace-plan-preview')).toBeNull();
  expect(container.querySelector('.composer-area .plan-decision')).not.toBeNull();
});
async function editFirstQueuedMessage(){
  await act(async()=>container.querySelector<HTMLButtonElement>('[data-followup-id="first"] [aria-label="排队消息更多操作"]')!.click());
  await act(async()=>{[...document.querySelectorAll<HTMLButtonElement>('.followup-menu button')].find(button=>button.textContent==='编辑消息')!.click();await new Promise(resolve=>setTimeout(resolve,30));});
}
it('keeps child startup and completion separate from the primary thread even with a shared process owner',async()=>{
  queuedMessages(); data.sessions[0].agent='codex';
  messages=[{id:'user',seq:1,sessionId:'running',role:'user',text:'检查两个子代理',kind:'userMessage',data:{}}];
  await render();
  await act(async()=>notify({method:'thread/started',params:{appSessionId:'running',thread:{id:'child-a',source:{subAgent:{threadSpawn:{parentThreadId:'native'}}}}}}));
  await act(async()=>notify({method:'turn/started',params:{appSessionId:'running',threadId:'child-a',turn:{id:'child-turn'}}}));
  await act(async()=>{
    notify({method:'item/completed',params:{appSessionId:'running',threadId:'child-a',item:{id:'child-answer',type:'agentMessage',text:'子代理内容不能串进主对话'}}});
    notify({method:'item/completed',params:{appSessionId:'running',threadId:'native',item:{id:'parent-answer',type:'agentMessage',text:'主代理继续等待结果'}}});
    await new Promise(resolve=>setTimeout(resolve,70));
  });
  expect(container.textContent).toContain('主代理继续等待结果');
  expect(container.textContent).not.toContain('子代理内容不能串进主对话');
  expect(container.querySelector('.composer [aria-label="停止生成"]')).not.toBeNull();
  await act(async()=>notify({method:'turn/completed',params:{appSessionId:'running',threadId:'child-a',turn:{id:'child-turn',status:'completed'}}}));
  expect(container.querySelector('.composer [aria-label="停止生成"]')).not.toBeNull();
});
it('ignores an initial empty goal notification and clears command feedback when switching conversations',async()=>{
  queuedMessages(); data.sessions[0].agent='codex'; await render();
  await act(async()=>notify({method:'thread/goal/cleared',params:{threadId:'native',goal:null}}));
  expect(container.querySelector('.command-feedback')).toBeNull();
  await act(async()=>notify({method:'thread/goal/updated',params:{threadId:'native',goal:{objective:'完成测试目标',status:'active',tokensUsed:15}}}));
  expect(container.querySelector('.command-feedback')?.textContent).toContain('完成测试目标');
  await act(async()=>notify({method:'thread/goal/cleared',params:{threadId:'native',goal:null}}));
  expect(container.querySelector('.command-feedback')?.textContent).toContain('目标已清除');
  await act(async()=>container.querySelector<HTMLButtonElement>('[title="新建聊天 (Ctrl+N)"]')!.click());
  expect(container.querySelector('.command-feedback')).toBeNull();
});
it('opens child details without starting a chat and preserves the main composer and workspace when closed',async()=>{
  queuedMessages();
  localStorage.setItem('supercode.context','true');
  messages=[{id:'task',seq:1,sessionId:'running',role:'tool',text:'只读检查完成',kind:'claudeToolCall',data:{tool:'Task',status:'completed',arguments:{description:'demo'}}}];
  await act(async()=>{await import('./SubagentPanel');});
  await render();await type('尚未发送的主会话草稿');
  await act(async()=>container.querySelector<HTMLButtonElement>('.subagent-row')!.click());
  expect(container.querySelector('.subagent-panel')?.textContent).toContain('Agent demo');
  expect(container.querySelector('.subagent-panel')?.textContent).toContain('只读检查完成');
  expect(container.querySelector<HTMLElement>('.workspace-panel')?.hidden).toBe(true);
  expect(container.querySelector('.conversation-surface [aria-label="任务运行状态"]')).not.toBeNull();
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('尚未发送的主会话草稿');
  expect(call).not.toHaveBeenCalledWith('create_session',expect.anything());
  expect(call).not.toHaveBeenCalledWith('send_chat_message',expect.anything());
  const opener=container.querySelector<HTMLButtonElement>('.subagent-row')!;
  const focus=vi.spyOn(opener,'focus');
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="关闭子智能体"]')!.click());
  await act(async()=>{await new Promise(resolve=>requestAnimationFrame(resolve));});
  expect(focus).toHaveBeenCalledWith({preventScroll:true});
  expect(container.querySelector('.subagent-panel')).toBeNull();
  expect(container.querySelector<HTMLElement>('.workspace-panel')?.hidden).toBe(false);
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('尚未发送的主会话草稿');
});
it.each(['none','subagent','side-chat'] as const)('opens an operation code change with %s in the right pane and preserves drafts',async pane=>{
  localStorage.setItem('supercode.session','review');
  localStorage.setItem('supercode.context','false');
  data.sessions=[{id:'review',projectId:'existing',title:'本次代码改动',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:'native',turnId:null,updatedAt:1}];
  const diff='diff --git a/中文.ts b/中文.ts\n--- a/中文.ts\n+++ b/中文.ts\n@@ -1 +1 @@\n-old_value\n+new_value\n';
  messages=[
    {id:'u',seq:1,sessionId:'review',role:'user',kind:'userMessage',text:'修改文件',data:{}},
    {id:'edit',seq:2,sessionId:'review',role:'tool',kind:'claudeToolCall',text:'saved',data:{tool:'Edit',status:'completed',changes:[{path:'中文.ts',diff,added:1,removed:1}]}},
    {id:'answer',seq:3,sessionId:'review',role:'assistant',kind:'agentMessage',text:'修改已完成',data:{}},
    {id:'run',seq:4,sessionId:'review',role:'activity',kind:'runMarker',text:'',data:{status:'completed',durationMs:1000}},
  ];
  if(pane==='subagent') messages.splice(2,0,{id:'task',seq:0,sessionId:'review',role:'tool',kind:'claudeToolCall',text:'子代理只读结果',data:{tool:'Task',status:'completed',arguments:{description:'demo'}}});
  if(pane==='side-chat') {
    const base=vi.mocked(call).getMockImplementation()!;
    vi.mocked(call).mockImplementation(async(command,args={})=>command==='list_followups'&&args.sessionId==='review'?[{id:'side-source',sessionId:'review',status:'paused',error:null,payload:{text:'侧边聊天草稿',permissionMode:'ask',model:'test-model',attachments:[]}}]:base(command,args));
  }
  await render(); await type('保留我的未发送草稿');
  if(pane==='subagent') await act(async()=>{container.querySelector<HTMLButtonElement>('.subagent-row')!.click();await vi.dynamicImportSettled();});
  if(pane==='side-chat') {
    await act(async()=>{await import('./SideChat');});
    await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="排队消息更多操作"]')!.click());
    await act(async()=>{[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(item=>item.textContent?.includes('在侧边聊天中打开'))!.click();await new Promise(resolve=>setTimeout(resolve,30));await vi.dynamicImportSettled();});
    const input=container.querySelector<HTMLTextAreaElement>('[aria-label="侧边聊天消息"]')!;
    expect(input).not.toBeNull();
    await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,'保留侧边聊天草稿');input.dispatchEvent(new Event('input',{bubbles:true}));});
  }
  const previousPane=pane==='none'?null:container.querySelector<HTMLElement>(`.${pane==='subagent'?'subagent-panel':'side-chat'}`);
  if(previousPane) expect(previousPane.hidden).toBe(false);
  await act(async()=>container.querySelector<HTMLButtonElement>('.conversation-surface .turn-process-toggle')!.click());
  await act(async()=>container.querySelector<HTMLButtonElement>('.conversation-surface .activity-group-summary')!.click());
  expect(container.querySelector('.file-preview')).toBeNull();
  await act(async()=>{container.querySelector<HTMLButtonElement>('.conversation-surface .activity-row')!.click();await vi.dynamicImportSettled();});
  await act(async()=>{await vi.dynamicImportSettled();});
  const workspace=container.querySelector<HTMLElement>('.workspace-panel')!;
  expect(workspace.hidden).toBe(false);
  expect(workspace.getAttribute('aria-hidden')).toBe('false');
  expect(workspace.querySelector('.workspace-file-preview')?.textContent).toContain('本次变更 · 中文.ts');
  expect(workspace.querySelector('.change-preview')?.textContent).toContain('new_value');
  expect(container.querySelector('.conversation-surface .activity-body')).toBeNull();
  expect(container.querySelector('.conversation-surface')?.textContent).not.toContain('new_value');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('保留我的未发送草稿');
  if(previousPane) expect(previousPane.hidden).toBe(true);
  await act(async()=>container.querySelector<HTMLButtonElement>('.right-pane [title="关闭文件"]')!.click());
  expect(container.querySelector('.workspace-file-preview')).toBeNull();
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('保留我的未发送草稿');
  if(previousPane) {
    expect(previousPane.isConnected).toBe(true);
    expect(previousPane.hidden).toBe(false);
  }
  if(pane==='side-chat') expect(container.querySelector<HTMLTextAreaElement>('[aria-label="侧边聊天消息"]')?.value).toBe('保留侧边聊天草稿');
});
it('edits a queued message in the main composer, saves in place, and restores its unsent draft',async()=>{
  const rows=queuedMessages();await render();await type('原来没发的草稿');await editFirstQueuedMessage();
  const input=container.querySelector<HTMLTextAreaElement>('.composer textarea')!;
  expect(input.value).toBe('第一条原文');expect(document.activeElement).toBe(input);
  expect(container.querySelector('.composer-skills')?.textContent).toContain('Recall');
  expect(container.querySelector('.followup-queue textarea')).toBeNull();
  expect([...container.querySelectorAll('.followup-row')].map(row=>row.getAttribute('data-followup-id'))).toEqual(['second']);
  expect(container.querySelector('.composer .send-button')?.getAttribute('aria-label')).toBe('发送修改');
  await type('/stop');vi.mocked(call).mockClear();await send();
  expect(call).toHaveBeenCalledWith('finish_followup_edit',{id:'first',token:'edit-token',payload:expect.objectContaining({text:'/stop',effort:'high',attachments:rows[0].payload.attachments})});
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
  expect(input.value).toBe('原来没发的草稿');
  expect([...container.querySelectorAll('.followup-row')].map(row=>row.getAttribute('data-followup-id'))).toEqual(['first','second']);
  expect(container.querySelector('.composer-editing')).toBeNull();
});
it('retains failed edits and their attachments, then cancels back to the original queued message and draft',async()=>{
  const rows=queuedMessages();await render();await type('自己的草稿');await editFirstQueuedMessage();await type('修改后暂时无法发送');
  const base=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>{if(command==='finish_followup_edit'&&args.payload)throw new Error('连接暂不可用');return base(command,args);});
  await send();
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('修改后暂时无法发送');
  expect(container.querySelector('.composer-editing')).not.toBeNull();
  expect(rows[0].payload.text).toBe('第一条原文');expect(rows[0].status).toBe('editing');
  expect(container.querySelector('.error-banner')?.textContent).toContain('连接暂不可用');
  await act(async()=>[...container.querySelectorAll<HTMLButtonElement>('.composer button')].find(button=>button.textContent==='取消编辑')!.click());
  expect(call).toHaveBeenCalledWith('finish_followup_edit',{id:'first',token:'edit-token',payload:null});
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('自己的草稿');
  expect(rows[0].payload.text).toBe('第一条原文');expect(rows[0].status).toBe('queued');
});
it('releases only its edit reservation when the editor closes so the queue is not left blocked',async()=>{
  const rows=queuedMessages();await render();await editFirstQueuedMessage();
  expect(rows[0].status).toBe('editing');vi.mocked(call).mockClear();
  await act(async()=>root.unmount());
  expect(call).toHaveBeenCalledWith('finish_followup_edit',{id:'first',token:'edit-token',payload:null});
  expect(rows[0].status).toBe('queued');expect(rows[1].payload.text).toBe('第二条');
});
it('releases a delayed edit reservation that arrives after the window has already closed',async()=>{
  const rows=queuedMessages();const base=vi.mocked(call).getMockImplementation()!;
  let release!:()=>void;const delayed=new Promise<void>(resolve=>{release=resolve;});
  vi.mocked(call).mockImplementation(async(command,args={})=>{const result=await base(command,args);if(command==='begin_followup_edit')await delayed;return result;});
  await render();await editFirstQueuedMessage();expect(rows[0].status).toBe('editing');
  await act(async()=>root.unmount());await act(async()=>{release();await new Promise(resolve=>setTimeout(resolve,10));});
  expect(call).toHaveBeenCalledWith('finish_followup_edit',{id:'first',token:'edit-token',payload:null});
  expect(rows[0].status).toBe('queued');
});

it.each(['claude', 'codex', 'opencode', 'pi', 'gemini'])('starts two %s sessions before either startup finishes without queuing or crossing events', async agent => {
  localStorage.setItem('supercode.session', 'a');
  data.profiles = [{ ...data.profiles[0], id:`api:${agent}`, agent }];
  data.sessions = ['a', 'b'].map(id => ({ id, projectId:'existing', title:`会话 ${id}`, agent, connectionId:`api:${agent}`, model:'test-model', nativeId:null, status:'idle', turnId:null, updatedAt:1 }));
  const finish = new Map<string, () => void>();
  const saved = new Map<string, Message[]>();
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args={}) => {
    const id = String(args.sessionId);
    if (command === 'list_messages') return saved.get(id) ?? [];
    if (command === 'send_chat_message') {
      await new Promise<void>(resolve => finish.set(id, resolve));
      saved.set(id, [{ id:`user-${id}`, seq:1, sessionId:id, role:'user', text:String(args.text), kind:'userMessage', data:{clientMessageId:args.clientMessageId} }]);
      return data.sessions.find(s => s.id === id)!;
    }
    return fallback(command, args);
  });
  await render(); await type('A 同时工作'); await send();
  await act(async () => container.querySelector<HTMLButtonElement>('[title="会话 b"]')!.click());
  await type('B 同时工作'); await send();
  expect(finish.size).toBe(2);
  expect(call).not.toHaveBeenCalledWith('enqueue_followup', expect.anything());
  expect(container.querySelector('[aria-label="你的消息"]')?.textContent).toBe('B 同时工作');
  await act(async () => {
    data.sessions = data.sessions.map(s => ({ ...s, nativeId:`native-${s.id}`, status:'running', turnId:`turn-${s.id}` }));
    notify({method:'turn/started', params:{appSessionId:'a', threadId:'native-a', turn:{id:'turn-a'}}});
    notify({method:'turn/started', params:{appSessionId:'b', threadId:'native-b', turn:{id:'turn-b'}}});
    notify({method:'item/agentMessage/delta', params:{appSessionId:'a', threadId:'native-a', turnId:'turn-a', itemId:'item-a', delta:'A 私有输出'}});
    notify({method:'item/agentMessage/delta', params:{appSessionId:'b', threadId:'native-b', turnId:'turn-b', itemId:'item-b', delta:'B 私有输出'}});
    await new Promise(resolve => setTimeout(resolve,80));
  });
  expect(container.textContent).not.toContain('A 私有输出');
  expect(container.textContent).toContain('B 私有输出');
  await act(async () => finish.get('b')!());
  expect(container.querySelector('[aria-label="停止生成"]')).not.toBeNull();
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="停止生成"]')!.click());
  expect(call).toHaveBeenCalledWith('interrupt_turn', {sessionId:'b'});
  expect(call).not.toHaveBeenCalledWith('interrupt_turn', {sessionId:'a'});
  await act(async () => finish.get('a')!());
});

it('allows a new conversation and Agent selection while other conversations are running or waiting', async () => {
  localStorage.setItem('supercode.session', 'a');
  data.agents = ['claude','codex','opencode','pi','gemini'].map(id => ({id, name:id, installed:true, path:'agent.exe', connected:true}));
  data.profiles = data.agents.map(a => ({ ...data.profiles[0], id:`api:${a.id}`, agent:a.id }));
  data.sessions = ['a','b'].map((id, n) => ({id,projectId:'existing',title:`运行中的 ${id}`,agent:'claude',connectionId:'api:claude',model:'test-model',nativeId:`native-${id}`,status:n?'waiting':'running',turnId:`t-${id}`,updatedAt:1}));
  await render();
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="新建任务"]')!.click());
  expect(container.querySelector('[aria-label="停止生成"]')).toBeNull();
  const agent = container.querySelector<HTMLButtonElement>('[aria-label="Agent"]')!;
  expect(agent.disabled).toBe(false);
  await act(async () => agent.click());
  await act(async () => container.querySelectorAll<HTMLButtonElement>('[aria-label="选择 Agent"] [role="menuitemradio"]')[1].click());
  expect(container.querySelector('[aria-label="Agent"]')?.textContent).toContain('codex');
  await type('第三个会话立即发送'); await send();
  expect(call).toHaveBeenCalledWith('create_session', expect.objectContaining({agent:'codex'}));
  expect(call).toHaveBeenCalledWith('send_chat_message', expect.objectContaining({sessionId:'standalone'}));
  expect(call).not.toHaveBeenCalledWith('enqueue_followup', expect.anything());
  expect(call).not.toHaveBeenCalledWith('interrupt_turn', expect.anything());
});

it('starts fresh chats concurrently and restores a pending bubble when returning to its chat', async () => {
  let count = 0;
  const finish = new Map<string, () => void>();
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args={}) => {
    if (command === 'create_session') {
      const s: Session = {id:`new-${++count}`,projectId:'',title:`新聊天 ${count}`,agent:String(args.agent),connectionId:String(args.connectionId),model:'test-model',nativeId:null,status:'idle',turnId:null,updatedAt:count};
      data.sessions.push(s); return s;
    }
    if (command === 'send_chat_message') { await new Promise<void>(resolve => finish.set(String(args.sessionId),resolve)); return data.sessions.find(s => s.id === args.sessionId)!; }
    if (command === 'list_messages') return [];
    return fallback(command,args);
  });
  await render(); await type('新的第一个聊天'); await send();
  expect(finish.has('new-1')).toBe(true);
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="新建任务"]')!.click());
  await type('新的第二个聊天'); await send();
  expect(finish.has('new-2')).toBe(true);
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  await act(async () => container.querySelector<HTMLButtonElement>('[title="新聊天 1"]')!.click());
  expect(container.querySelector('[aria-label="你的消息"]')?.textContent).toBe('新的第一个聊天');
  expect(container.textContent).not.toContain('新的第二个聊天');
  await act(async () => { finish.get('new-1')!(); finish.get('new-2')!(); });
});

it.each(['claude','codex','opencode','pi','gemini'])('shows the submitted %s message before connection checks, session creation and native startup finish', async agent => {
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
it.each(['claude', 'codex', 'opencode', 'pi', 'gemini'] as const)('shows persisted, live and completed context without a guessed model capacity for %s', async agent => {
  localStorage.setItem('supercode.session', 'context-chat');
  data.sessions = [{ id: 'context-chat', projectId: 'existing', title: '上下文测试', agent, connectionId: `api:${agent}`, model: 'provider-specific-model', nativeId: 'native-context', status: 'running', turnId: 'turn-context', updatedAt: 1 }];
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
it('waits for hosted-route capacity and never substitutes the publisher window while its catalog is pending', async () => {
  localStorage.setItem('supercode.session','hosted-context');
  data.profiles[0].model = 'MiniMax-M3';
  data.sessions = [{id:'hosted-context',projectId:'existing',title:'托管模型',agent:'claude',connectionId:'api:claude',model:'MiniMax-M3',nativeId:'native-context',status:'running',turnId:'turn-context',updatedAt:1}];
  let resolve!: (value: unknown) => void;
  const pending = new Promise(done => { resolve = done; });
  const fallback = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command,args) => command==='list_models' ? pending : command==='get_usage' ? {records:[{data:{total:{},last:{},contextTokens:96000,cumulative:false}}]} : fallback(command,args));
  await render();
  expect(container.querySelector('.usage-trigger')?.textContent).toBe('上下文 96.0K');
  expect(container.querySelector('.usage-trigger')?.getAttribute('aria-label')).not.toContain('1,000,000');
  await act(async () => { resolve({data:[{id:'MiniMax-M3',model:'MiniMax-M3',displayName:'MiniMax-M3',isDefault:true,contextWindow:192000}]}); await pending; });
  expect(container.querySelector('.usage-trigger')?.textContent).toBe('上下文 50%');
  await act(async () => notify({method:'thread/tokenUsage/updated',params:{threadId:'native-context',turnId:'turn-context',tokenUsage:{total:{},last:{},contextTokens:96000,modelContextWindow:160000,cumulative:false}}}));
  expect(container.querySelector('.usage-trigger')?.textContent).toBe('上下文 60%');
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
    {id:'progress',seq:3,sessionId:'chat',role:'assistant',kind:'agentMessage',text:'先检查文件',data:{turnId:'t',status:'completed',phase:'commentary'}},
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

it('folds restored legacy progress and tools while preserving the final answer through pagination', async () => {
  localStorage.setItem('supercode.session', 'chat');
  data.sessions = [{id:'chat',projectId:'existing',title:'旧历史记录',agent:'claude',connectionId:'api:claude',model:null,status:'idle',nativeId:'native',turnId:null,updatedAt:1}];
  const marker: Message = {id:'run-old',seq:2,sessionId:'chat',role:'activity',kind:'runMarker',text:'',data:{turnId:'old',status:'inProgress'}};
  const history: Message[] = [{id:'u',seq:1,sessionId:'chat',role:'user',kind:'userMessage',text:'制作插件',data:{}}, marker,
    ...Array.from({length:125}, (_, index): Message => ({id:`old-${index}`,seq:index + 3,sessionId:'chat',role:index === 124 ? 'assistant' : index % 2 ? 'tool' : 'assistant',kind:index % 2 ? 'commandExecution' : 'agentMessage',text:index === 124 ? '插件开发完成。' : `旧执行过程 ${index}`,data:{turnId:'old',status:'completed'}}))];
  const original = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args = {}) => {
    if (command !== 'list_messages') return original(command, args);
    const page = history.filter(message => message.seq < Number(args.before ?? Infinity)).slice(-100).map(message => ({...message}));
    if (page[0]?.seq > marker.seq) page[0].historyRun = {id:marker.id,turnKey:'u',data:marker.data};
    return page;
  });
  await render();
  expect(container.querySelector('.turn-process-toggle')?.getAttribute('aria-expanded')).toBe('false');
  expect(container.querySelector('.turn-process-content')).toBeNull();
  expect(container.querySelector('.turn-final-result')?.textContent).not.toContain('旧执行过程 26');
  expect(container.querySelector('.turn-final-result')?.textContent).not.toContain('旧执行过程 27');
  expect(container.querySelector('.turn-final-result')?.textContent).toContain('插件开发完成');
  const final = container.querySelector('.turn-final-result .message:last-child');
  await act(async () => container.querySelector<HTMLButtonElement>('.turn-process-toggle')!.click());
  expect(container.querySelectorAll('.turn-process-content .activity-group-summary')).toHaveLength(50);
  expect(container.querySelector('.turn-process-content')?.textContent).toContain('旧执行过程 26');
  await act(async () => { container.querySelector<HTMLButtonElement>('.load-earlier')!.click(); await vi.dynamicImportSettled(); });
  expect(vi.mocked(call).mock.calls.some(([command,args]) => command === 'list_messages' && args?.before === 28)).toBe(true);
  expect(container.querySelector('.load-earlier')).toBeNull();
  expect(container.querySelectorAll('.turn-process-content .activity-group-summary')).toHaveLength(62);
  expect(container.querySelector('.turn-process-content')?.textContent).toContain('旧执行过程 0');
  expect(container.querySelector('.turn-final-result')?.textContent).not.toContain('旧执行过程 0');
  expect(container.querySelector('.turn-final-result .message:last-child')).toBe(final);
  expect(container.querySelectorAll('[aria-label="你的消息"]')).toHaveLength(1);
});

it('keeps restored active, failed and interrupted legacy conversations expanded', async () => {
  localStorage.setItem('supercode.session', 'chat');
  messages = [{id:'progress',seq:1,sessionId:'chat',role:'assistant',kind:'agentMessage',text:'部分过程',data:null}, {id:'partial',seq:2,sessionId:'chat',role:'assistant',kind:'agentMessage',text:'还未完成的输出',data:null}];
  for (const status of ['starting','running','waiting','failed','interrupted']) {
    data.sessions = [{id:'chat',projectId:'existing',title:'恢复会话',agent:'claude',connectionId:'api:claude',model:null,status,nativeId:'native',turnId:status === 'running' ? 'active' : null,updatedAt:1}];
    await render();
    expect(container.querySelector('.turn-process-toggle')).toBeNull();
    expect(container.textContent).toContain('部分过程');
    await act(async () => root.unmount());
    root = createRoot(container);
  }
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
it('shares right-side tabs without remounting the browser and collapses the last closed tab', async () => {
  const media = vi.spyOn(window, 'matchMedia').mockImplementation(() => ({matches:false,addEventListener:()=>{},removeEventListener:()=>{}} as unknown as MediaQueryList));
  localStorage.setItem('supercode.context', 'true');
  localStorage.setItem('supercode.session', 'chat');
  data.sessions = [{id:'chat',projectId:'existing',title:'面板测试',agent:'claude',connectionId:'api:claude',model:null,status:'idle',nativeId:null,turnId:null,updatedAt:1}];
  await render();
  await type('保留主聊天草稿');
  expect(container.querySelector('[aria-label="工作空间"]')).not.toBeNull();
  await act(async () => { container.querySelector<HTMLButtonElement>('[aria-label="打开内置浏览器"]')!.click(); await vi.dynamicImportSettled(); await new Promise(resolve=>setTimeout(resolve,30)); });
  const browser = container.querySelector('[aria-label="内置浏览器"]')!;
  expect(browser).not.toBeNull();
  expect(browser.querySelector('.context-tabs')).toBeNull();
  expect(browser.textContent).not.toContain('文件变更');
  expect(browser.parentElement?.classList.contains('right-pane-content')).toBe(true);
  expect(container.querySelector('[aria-label="工作空间"]')?.parentElement).toBe(browser.parentElement);
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="展开右侧栏"]')!.click());
  expect(container.querySelector('[aria-label="还原右侧栏宽度"]')).not.toBeNull();
  expect(container.querySelector('[aria-label="工作空间"]')).not.toBeNull();
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="关闭工作空间"]')!.click());
  expect(container.querySelector('[aria-label="工作空间"]')).toBeNull();
  expect(container.querySelector('[aria-label="内置浏览器"]')).toBe(browser);
  expect(call).toHaveBeenCalledWith('browser_open', expect.objectContaining({sessionId:'chat'}));
  expect(vi.mocked(call).mock.calls.filter(([command])=>command==='browser_open')).toHaveLength(1);
  expect(call).not.toHaveBeenCalledWith('browser_close', expect.anything());
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="打开工作空间"]')!.click());
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="关闭浏览器"]')!.click());
  expect(container.querySelector('[aria-label="内置浏览器"]')).toBeNull();
  expect(container.querySelector('[aria-label="工作空间"]')).not.toBeNull();
  expect(call).toHaveBeenCalledWith('browser_close',expect.objectContaining({sessionId:'chat',mountId:expect.any(String)}));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="关闭工作空间"]')!.click());
  expect(container.querySelector<HTMLElement>('.right-pane')?.hidden).toBe(true);
  expect(container.querySelector('.app')?.classList.contains('context-hidden')).toBe(true);
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('保留主聊天草稿');
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
  media.mockRestore();
});
it('previews an explicitly referenced external file with its line without changing the current chat', async () => {
  localStorage.setItem('supercode.session', 'preview-chat');
  data.sessions = [{ id: 'preview-chat', projectId: 'existing', title: '文件预览', agent: 'claude', connectionId: 'api:claude', model: 'test-model', status: 'idle', nativeId: null, turnId: null, updatedAt: 1 }];
  messages = [{ id: 'reference', seq: 1, sessionId: 'preview-chat', role: 'assistant', kind: 'agentMessage', text: '[查看报告](<D:/外部目录 with spaces/报告.md:7>)', data: { status: 'completed' } }];
  const base = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command, args = {}) => command === 'read_project_file' ? '1\n2\n3\n4\n5\n6\n目录外文件内容' : base(command, args));
  await render(); await act(async () => { await vi.dynamicImportSettled(); });
  await act(async () => container.querySelector<HTMLAnchorElement>('a.markdown-link')!.click());
  await act(async () => { await vi.dynamicImportSettled(); });
  expect(call).toHaveBeenCalledWith('read_project_file', { projectId: 'existing', sessionId: 'preview-chat', path: 'D:/外部目录 with spaces/报告.md' });
  expect(container.querySelector('.document-panel .source-selected')?.textContent).toContain('目录外文件内容');
  expect(container.querySelector('.document-panel .error-banner')).toBeNull();
  expect(localStorage.getItem('supercode.session')).toBe('preview-chat');
  expect(call).not.toHaveBeenCalledWith('interrupt_turn', expect.anything());
});

it.each(['close','switch'] as const)('does not reopen or select a delayed file after the user chooses to %s its view',async action=>{
  localStorage.setItem('supercode.context','true');localStorage.setItem('supercode.workspace-view','files');localStorage.setItem('supercode.session','slow-file');
  data.sessions=[{id:'slow-file',projectId:'existing',title:'慢文件',agent:'claude',connectionId:'api:claude',model:'test-model',status:'running',nativeId:'native',turnId:'t',updatedAt:1}];
  let finish!:(text:string)=>void;const pending=new Promise<string>(resolve=>{finish=resolve;});
  const base=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>command==='list_workspace_directory'?{entries:[{name:'slow.ts',path:'slow.ts',isDirectory:false}],truncated:false}:command==='read_project_file'?pending:base(command,args));
  await render();await act(async()=>{await vi.dynamicImportSettled();});await type('保留草稿');
  await act(async()=>container.querySelector<HTMLButtonElement>('.workspace-tree-row[title="slow.ts"]')!.click());
  expect(container.querySelector('.right-pane-tabs')?.textContent).toContain('slow.ts');
  if(action==='close')await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="关闭右侧栏"]')!.click());
  else await act(async()=>Array.from(container.querySelectorAll<HTMLButtonElement>('.right-pane [role="tab"]')).find(tab=>tab.textContent==='工作空间')!.click());
  await act(async()=>finish('export const loaded = true;'));await act(async()=>{await vi.dynamicImportSettled();});
  if(action==='close') {expect(container.querySelector('.document-panel')).toBeNull();expect(container.querySelector<HTMLElement>('.right-pane')?.hidden).toBe(true);}
  else {expect(container.querySelector<HTMLElement>('.document-panel')?.hidden).toBe(true);expect(container.querySelector<HTMLElement>('[aria-label="工作空间"]')?.hidden).toBe(false);}
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('保留草稿');
  expect(container.querySelector('.composer .send-button')?.getAttribute('aria-label')).toBe('加入队列');
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
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

it.each(['claude','codex','opencode','pi','gemini'])('prompts before creating or sending an unconfigured %s conversation, with an actionable supplier tab', async agent => {
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

it('keeps empty idle and starting chats in disabled send mode until a turn exists', async () => {
  localStorage.setItem('supercode.session','idle');
  data.sessions = [
    {id:'idle',projectId:'existing',title:'空闲聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:2},
    {id:'starting',projectId:'existing',title:'正在启动',agent:'claude',connectionId:'api:claude',model:'test-model',status:'starting',nativeId:null,turnId:null,updatedAt:1},
  ];
  await render();
  expect(container.querySelector('.composer .stop-button')).toBeNull();
  expect(container.querySelector<HTMLButtonElement>('.composer .send-button')?.disabled).toBe(true);
  expect(container.querySelector('.active-elsewhere')).toBeNull();
  expect(container.textContent).not.toContain('另一会话正在运行');
  await act(async () => { container.querySelector<HTMLButtonElement>('.session-row[title="正在启动"]')!.click(); await new Promise(resolve=>setTimeout(resolve,30)); });
  const action = container.querySelector<HTMLButtonElement>('.composer .send-button')!;
  expect(container.querySelector('.composer .stop-button')).toBeNull();
  expect(action.type).toBe('submit');
  expect(action.getAttribute('aria-label')).toBe('发送消息');
  expect(action.disabled).toBe(true);
  await act(async () => action.click());
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
});

it.each(['codex','claude','opencode','pi','gemini'])('does not show a stop icon for an empty %s chat without an active turn',async agent=>{
  localStorage.setItem('supercode.session','starting');
  data.sessions=[{id:'starting',projectId:'existing',title:'正在启动',agent,connectionId:'@local',model:'test-model',status:'starting',nativeId:null,turnId:null,updatedAt:1}];
  await render();
  const action=container.querySelector<HTMLButtonElement>('.composer .send-button')!;
  expect(action.classList.contains('stop-button')).toBe(false);
  expect(action.querySelector('.lucide-square')).toBeNull();
  expect(action.querySelector('.lucide-arrow-up')).not.toBeNull();
  expect(action.disabled).toBe(true);
});

it('keeps the disabled send arrow while a cleared draft is being submitted and switches to stop only when its turn starts',async()=>{
  localStorage.setItem('supercode.session','idle');
  data.sessions=[{id:'idle',projectId:'existing',title:'空闲聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:'native',turnId:null,updatedAt:1}];
  let finish!:(session:Session)=>void;
  const base=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>command==='send_chat_message'?new Promise<Session>(resolve=>{finish=resolve;}):base(command,args));
  await render();await type('开始检查');await send();
  const action=container.querySelector<HTMLButtonElement>('.composer .send-button')!;
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')?.value).toBe('');
  expect(action.classList.contains('stop-button')).toBe(false);
  expect(action.querySelector('.lucide-arrow-up')).not.toBeNull();
  expect(action.disabled).toBe(true);
  await act(async()=>{data.sessions[0]={...data.sessions[0],status:'starting'};finish(data.sessions[0]);await new Promise(resolve=>setTimeout(resolve,30));});
  expect(action.classList.contains('stop-button')).toBe(false);
  await act(async()=>{data.sessions[0]={...data.sessions[0],status:'running',turnId:'turn'};notify({method:'turn/started',params:{threadId:'native',turn:{id:'turn'}}});await new Promise(resolve=>setTimeout(resolve,30));});
  expect(action.classList.contains('stop-button')).toBe(true);
  expect(action.disabled).toBe(false);
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

it('selects the connected Gemini Agent and opens its model supplier tab without borrowing another account', async () => {
  data.agents = [{ id: 'claude', name: 'Claude Code', installed: true, path: 'claude.exe', connected: true }, { id: 'gemini', name: 'Gemini CLI', installed: true, path: 'gemini.exe', connected: true }];
  data.profiles.push({ id: 'api:gemini', agent: 'gemini', name: 'Gemini API', model: 'gemini-test', hasCredential: true, current: true });
  await render(); await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Agent"]')!.click());
  const option = [...container.querySelectorAll<HTMLButtonElement>('[aria-label="选择 Agent"] [role="menuitemradio"]')].find(button => button.textContent?.includes('Gemini CLI'))!;
  expect(option.textContent).toContain('Gemini API');
  await act(async () => option.click());
  expect(container.querySelector('[aria-label="Agent"]')!.textContent).toContain('Gemini CLI');
  await type('使用 Gemini 执行'); await send();
  expect(call).toHaveBeenCalledWith('create_session', expect.objectContaining({ agent: 'gemini', connectionId: 'api:gemini' }));
  expect(call).toHaveBeenCalledWith('send_chat_message', expect.objectContaining({ text: '使用 Gemini 执行', model: 'gemini-test', permissionMode: 'ask' }));
});

it('sends reported Gemini native commands and shows its correct local status', async () => {
  localStorage.setItem('supercode.session', 'gemini-chat');
  data.agents = [{ id: 'gemini', name: 'Gemini CLI', installed: true, path: 'gemini.exe', connected: true }];
  data.profiles = [{ id: 'api:gemini', agent: 'gemini', name: 'Gemini API', model: 'gemini-test', hasCredential: true, current: true }];
  data.sessions = [{ id: 'gemini-chat', projectId: 'existing', title: 'Gemini 对话', agent: 'gemini', connectionId: 'api:gemini', model: 'gemini-test', nativeId: 'gemini-native', status: 'idle', turnId: null, updatedAt: 1 }];
  await render(); await act(async () => notify({ method: 'agent/commands', params: { appSessionId: 'gemini-chat', threadId: 'gemini-native', commands: ['review', 'compact'] } }));
  await type('/review 当前文件'); await send();
  expect(call).toHaveBeenCalledWith('send_message', expect.objectContaining({ sessionId: 'gemini-chat', text: '/review 当前文件' }));
  await type('/status'); await send(); expect(container.textContent).toContain('Gemini CLI ·');
  expect(call).not.toHaveBeenCalledWith('execute_agent_command', expect.anything());
});

it('reports unsupported Gemini manual compaction without calling the runtime or clearing the draft', async () => {
  localStorage.setItem('supercode.agent', 'gemini');
  await render(); await type('/compact'); await send();
  expect(container.querySelector('[role="alert"]')!.textContent).toContain('Gemini CLI 当前不支持手动压缩');
  expect(container.querySelector<HTMLTextAreaElement>('.composer textarea')!.value).toBe('/compact');
  expect(call).not.toHaveBeenCalledWith('compact_session_context', expect.anything());
  expect(call).not.toHaveBeenCalledWith('create_session', expect.anything());
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
it('keeps one connection quota revision across chats rather than treating each chat as a new plan',async()=>{
  localStorage.setItem('supercode.session','quota-first');
  data.sessions=['quota-first','quota-second'].map((id,index)=>({id,projectId:'existing',title:index?'第二个额度聊天':'第一个额度聊天',agent:'claude',connectionId:'api:claude',model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:index+1}));
  vi.mocked(PlatformUsageIndicator).mockClear();await render();
  const before=vi.mocked(PlatformUsageIndicator).mock.calls.at(-1)?.[0];
  expect(before?.source?.available).toBe(true);
  await act(async()=>{Array.from(container.querySelectorAll<HTMLButtonElement>('.session-row')).find(button=>button.textContent?.includes('第二个额度聊天'))!.click();await new Promise(resolve=>setTimeout(resolve,30));});
  const after=vi.mocked(PlatformUsageIndicator).mock.calls.at(-1)?.[0];
  expect(after?.connectionId).toBe('api:claude');expect(after?.revision).toBe(before?.revision);
});
it('keeps a configured plan entry when model catalog metadata is temporarily absent',async()=>{
  data.profiles=[{...data.profiles[0],name:'GLM Coding',providerId:'glm',plan:'coding'}];
  const base=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>command==='list_models'?{data:[{id:'test-model',model:'test-model',displayName:'test-model',isDefault:true}]}:base(command,args));
  await render();
  expect(container.querySelector('[data-testid="platform-allowance"]')).not.toBeNull();
  const props=vi.mocked(PlatformUsageIndicator).mock.calls.at(-1)?.[0];
  expect(props?.source?.providerId).toBe('glm');expect(props?.source?.connectionName).toBe('GLM Coding');
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
