// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import SideChat, { type SideDraft } from './SideChat';
import { call } from './api';
import { PROVIDER_REQUIRED } from './chatConnection';
import type { Session, RpcEvent } from './types';
import type { ComponentProps } from 'react';
import type { Followup } from './Followups';

vi.mock('./api', () => ({ desktop:true, call:vi.fn(), subscribe:vi.fn(async()=>()=>{}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen:vi.fn(async()=>()=>{}) }));
const defaults = {text:'',model:'test-model',readOnly:true,permissionMode:'read',attachments:[],effort:null};
let root: Root, container: HTMLDivElement, missing: boolean, sessions: Session[];
let requests: RpcEvent[];
const providerRequired = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  missing = true; sessions = []; requests = []; providerRequired.mockClear();
  vi.mocked(call).mockReset();
  vi.mocked(call).mockImplementation(async (command,args={}) => {
    if(command==='check_chat_connection') { if(missing) throw PROVIDER_REQUIRED; return null; }
    if(command==='create_session') {
      const session: Session = {id:'side',projectId:String(args.projectId),title:'Side',agent:String(args.agent),connectionId:String(args.connectionId),model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:1};
      sessions.push(session); return session;
    }
    if(command==='bootstrap') return {sessions};
    if(command==='runtime_info') return {requests};
    if(command==='respond_request') { requests = requests.filter(r => r.id !== args.id); return null; }
    if(['list_messages','list_followups'].includes(command)) return [];
    if(command==='followup_capabilities') return {steeringMode:'native'};
    return null;
  });
  container=document.createElement('div');document.body.append(container);root=createRoot(container);
});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();});
async function render(draft:SideDraft, options:Partial<ComponentProps<typeof SideChat>> = {}) {
  await act(async()=>root.render(<SideChat draft={draft} projectId="" agent="pi" connectionId={draft.connectionId??'@local'} defaults={defaults} close={()=>{}} opened={()=>{}} openFile={()=>{}} showDiff={()=>{}} providerRequired={providerRequired} {...options}/>));
}
async function send() {await act(async()=>{container.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));});}
it('blocks Enter while a side model switch is pending and uses the new model afterwards', async () => {
  missing = false;
  const base = vi.mocked(call).getMockImplementation()!; let finish!: (session: Session) => void;
  vi.mocked(call).mockImplementation((command, args={}) => command === 'switch_session_model' ? new Promise(resolve => { finish = resolve; }) : base(command,args));
  const profiles = [{ id:'api:pi',agent:'pi',name:'Pi API',model:'test-model',models:['test-model','next-model'],hasCredential:true,current:true }];
  await render({key:'draft',text:'建立侧边聊天',connectionId:'api:pi'}, { profiles, catalog:{ origin:'saved',data:[{id:'test-model',model:'test-model',displayName:'test-model',isDefault:true},{id:'next-model',model:'next-model',displayName:'next-model',isDefault:false}],source:{providerId:'custom',providerName:'Pi API',mark:'P',available:true} } });
  await send();
  const input = container.querySelector<HTMLTextAreaElement>('[aria-label="侧边聊天消息"]')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,'切换后再发送'); input.dispatchEvent(new Event('input',{bubbles:true})); });
  await act(async () => container.querySelector<HTMLButtonElement>('.model-chip')!.click());
  await act(async () => [...container.querySelectorAll<HTMLButtonElement>('.model-option')].find(button => button.textContent?.includes('next-model'))!.click());
  vi.mocked(call).mockClear();
  await act(async () => input.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true})));
  await send();
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  sessions[0] = { ...sessions[0], model:'next-model' };
  await act(async () => finish(sessions[0]));
  await send();
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({payload:expect.objectContaining({text:'切换后再发送',model:'next-model'})}));
});
it.each([['execute','ask'],['execute','full'],['execute','auto'],['revise','full']] as const)('reviews a native side-chat plan as %s preserving %s permission and its unsent draft',async (decision,remembered)=>{
  missing=false;
  const pendingConnectionChanged=vi.fn();
  localStorage.setItem('supercode.execution-permissions.v1',JSON.stringify({codex:remembered}));
  const base=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>command==='list_messages'?[{id:'p',seq:1,sessionId:'side',role:'assistant',kind:'plan',text:'# 侧边计划\n\n包含边界验收。',data:{status:'completed'}},{id:'r',seq:2,sessionId:'side',role:'activity',kind:'runMarker',text:'',data:{status:'completed'}}]:base(command,args));
  await act(async()=>{await import('./ProposedPlan');});
  await render({key:'draft',text:'先规划'},{agent:'codex',pendingConnectionChanged});await send();
  const input=container.querySelector<HTMLTextAreaElement>('[aria-label="侧边聊天消息"]')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,'保留这份侧边草稿');input.dispatchEvent(new Event('input',{bubbles:true}));});
  vi.mocked(call).mockClear();pendingConnectionChanged.mockClear();
  await act(async()=>container.querySelector<HTMLButtonElement>(decision==='execute'?'.plan-request-execute':'.plan-request-revise')!.click());
  expect(call).toHaveBeenCalledWith('enqueue_followup',{sessionId:'side',payload:expect.objectContaining({permissionMode:decision==='execute'?remembered:'read',readOnly:decision==='revise',attachments:[]})});
  expect(input.value).toBe('保留这份侧边草稿');
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'codex',id:'@local'}],[null]]);
});
it('restores full access when accepting a native Claude plan in the side chat',async()=>{
  missing=false;
  localStorage.setItem('supercode.execution-permissions.v1',JSON.stringify({claude:'full'}));
  requests=[{id:'claude:side:plan',method:'claude/tool/requestApproval',params:{threadId:null,toolName:'ExitPlanMode',input:{plan:'# 侧边计划\n\n创建文件。'}}}];
  await act(async()=>{await import('./PlanRequest');await import('./Markdown');});
  await render({key:'draft',text:'先规划'},{agent:'claude'});await send();
  await act(async()=>{await vi.dynamicImportSettled();});
  await act(async()=>container.querySelector<HTMLButtonElement>('.plan-request-execute')!.click());
  expect(call).toHaveBeenCalledWith('respond_request',{id:'claude:side:plan',result:{decision:'accept',executionPermission:'full'}});
  expect(container.querySelector('[aria-label="侧边聊天权限模式"]')?.textContent).toContain('完全访问');
});

it('edits its own queued message in the side composer and keeps its position instead of enqueueing a duplicate',async()=>{
  missing=false;
  const rows:Followup[]=['first','second'].map(id=>({id,sessionId:'side',status:'queued',error:null,payload:{...defaults,text:id==='first'?'第一条侧边补充':'第二条侧边补充',attachments:id==='first'?[{kind:'skill',name:'Recall',path:'D:/skills/recall/SKILL.md'}]:[]}}));
  const base=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(command==='list_followups')return structuredClone(rows);
    if(command==='begin_followup_edit'){const row=rows.find(row=>row.id===args.id)!;const original=structuredClone(row);row.status='editing';return {row:original,token:'side-token'};}
    if(command==='finish_followup_edit'){const row=rows.find(row=>row.id===args.id)!;if(args.payload)row.payload=structuredClone(args.payload) as Followup['payload'];row.status='queued';return null;}
    return base(command,args);
  });
  await render({key:'draft',text:'开启侧边聊天'});await send();
  const area=container.querySelector<HTMLTextAreaElement>('[aria-label="侧边聊天消息"]')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(area,'保留侧边草稿');area.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>container.querySelector<HTMLButtonElement>('[data-followup-id="first"] [aria-label="排队消息更多操作"]')!.click());
  await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('.followup-menu button')].find(button=>button.textContent==='编辑消息')!.click());
  expect(area.value).toBe('第一条侧边补充');expect(document.activeElement).toBe(area);
  expect(container.querySelector('[data-followup-id="first"]')).toBeNull();
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(area,'编辑后的侧边补充');area.dispatchEvent(new Event('input',{bubbles:true}));});
  vi.mocked(call).mockClear();await send();
  expect(call).toHaveBeenCalledWith('finish_followup_edit',{id:'first',token:'side-token',payload:expect.objectContaining({text:'编辑后的侧边补充',attachments:rows[0].payload.attachments})});
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  expect(call).not.toHaveBeenCalledWith('change_followup',expect.anything());
  expect([...container.querySelectorAll('.followup-row')].map(row=>row.getAttribute('data-followup-id'))).toEqual(['first','second']);
  expect(area.value).toBe('保留侧边草稿');
});
it('keeps the side-chat question beside its composer and cancels it without submitting a chat message', async () => {
  missing = false;
  requests = [{ id: 'pi:question', method: 'item/tool/requestUserInput', params: { threadId: null, nativeRequest: { method: 'select' }, questions: [{ id: '0', question: '怎么继续？', options: [{ label: '继续' }] }] } }];
  await render({ key: 'draft', text: '测试' }); await send();
  for (let i = 0; i < 10 && !container.querySelector('.question-request'); i++) {
    await act(async () => { await vi.dynamicImportSettled(); await new Promise(resolve => setTimeout(resolve, 20)); });
  }
  expect(container.querySelector('.side-chat-scroll .question-request')).toBeNull();
  expect(container.querySelector('footer .question-dock .question-request')).not.toBeNull();
  expect(container.querySelector('[aria-label="侧边聊天消息"]')?.getAttribute('rows')).toBe('1');
  vi.mocked(call).mockClear();
  await act(async () => container.querySelector<HTMLButtonElement>('.question-request-skip')!.click());
  expect(call).toHaveBeenCalledWith('respond_request', { id: 'pi:question', result: { answers: { '0': { answers: [] } } } });
  expect(call).not.toHaveBeenCalledWith('enqueue_followup', expect.anything());
  expect(container.querySelector('.question-dock')).toBeNull();
});

it('preserves edited side-chat text, attachments and the original queue item when its supplier is missing',async()=>{
  const draft: SideDraft={key:'draft',text:'选中内容',sourceId:'queued',payload:{...defaults,attachments:[{kind:'skill',name:'recall',path:'D:/skills/recall/SKILL.md'}]}};
  await render(draft);
  const textarea=container.querySelector('textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,'修改后的提问');textarea.dispatchEvent(new Event('input',{bubbles:true}));});
  await send();
  expect(call).toHaveBeenCalledWith('check_chat_connection',{agent:'pi',connectionId:'@local',sessionId:null});
  expect(call).not.toHaveBeenCalledWith('create_session',expect.anything());
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  expect(call).not.toHaveBeenCalledWith('change_followup',expect.anything());
  expect(providerRequired).toHaveBeenCalledWith('pi',expect.objectContaining({text:'修改后的提问',sourceId:'queued',payload:draft.payload}));
  expect(textarea.value).toBe('修改后的提问');
});

it('queues a configured side-chat message only after validation, then clears its input',async()=>{
  missing=false;
  await render({key:'draft',text:'侧边提问'}); await send();
  expect(call).toHaveBeenCalledWith('enqueue_followup',{sessionId:'side',payload:{...defaults,text:'侧边提问'}});
  const names=vi.mocked(call).mock.calls.map(([name])=>name);
  expect(names.indexOf('check_chat_connection')).toBeLessThan(names.indexOf('create_session'));
  expect(container.querySelector('textarea')!.value).toBe('');
  expect(providerRequired).not.toHaveBeenCalled();
});

it('shows a disabled send arrow in an empty side chat and while it starts without a turn',async()=>{
  missing=false;
  const original=vi.mocked(call).getMockImplementation()!;
  let finish!:()=>void;
  const waiting=new Promise<void>(resolve=>{finish=resolve;});
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(command==='enqueue_followup'){sessions[0]={...sessions[0],status:'starting'};return null;}
    if(command==='bootstrap'){await waiting;return {sessions};}
    return original(command,args);
  });
  await render({key:'draft',text:''});
  const action=container.querySelector<HTMLButtonElement>('footer form .send-button')!;
  expect(action.classList.contains('stop-button')).toBe(false);
  expect(action.disabled).toBe(true);
  const textarea=container.querySelector<HTMLTextAreaElement>('footer form textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,'开始侧边任务');textarea.dispatchEvent(new Event('input',{bubbles:true}));});
  await send();
  expect(textarea.value).toBe('');
  expect(action.classList.contains('stop-button')).toBe(false);
  expect(action.querySelector('.lucide-arrow-up')).not.toBeNull();
  expect(action.disabled).toBe(true);
  await act(async()=>{finish();});
  expect(action.classList.contains('stop-button')).toBe(false);
  expect(action.disabled).toBe(true);
});

it('uses one side-chat action for stopping and sending, and sends a new draft without interrupting its active task', async () => {
  missing = false;
  const original = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command,args={}) => {
    if(command==='enqueue_followup') {
      sessions[0] = {...sessions[0],status:'running',turnId:'turn',nativeId:'native'};
      return null;
    }
    return original(command,args);
  });
  await render({key:'draft',text:'开始任务'}); await send();
  const action = container.querySelector<HTMLButtonElement>('footer form .send-button')!;
  expect(container.querySelectorAll('footer form .send-button')).toHaveLength(1);
  expect(action.getAttribute('aria-label')).toBe('停止侧边聊天任务');
  const textarea = container.querySelector<HTMLTextAreaElement>('footer form textarea')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,'补充说明'); textarea.dispatchEvent(new Event('input',{bubbles:true})); });
  expect(container.querySelector('footer form .send-button')).toBe(action);
  expect(action.getAttribute('aria-label')).toBe('发送侧边聊天消息');
  expect(action.type).toBe('submit');
  vi.mocked(call).mockClear();
  await act(async () => action.click());
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({sessionId:'side',payload:expect.objectContaining({text:'补充说明'})}));
  expect(call).not.toHaveBeenCalledWith('interrupt_turn',expect.anything());
  expect(action.getAttribute('aria-label')).toBe('停止侧边聊天任务');
  expect(action.type).toBe('button');
  vi.mocked(call).mockClear();
  await act(async () => action.click());
  expect(call).toHaveBeenCalledWith('interrupt_turn',{sessionId:'side'});
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
});

it('can retry the preserved draft with the newly configured connection',async()=>{
  await render({key:'draft',text:'保留这段侧边提问'}); await send();
  const draft: SideDraft=providerRequired.mock.calls[0][1];
  missing=false;
  await render({...draft,connectionId:'api:pi'}); await send();
  expect(call).toHaveBeenCalledWith('check_chat_connection',{agent:'pi',connectionId:'api:pi',sessionId:null});
  expect(call).toHaveBeenCalledWith('create_session',expect.objectContaining({connectionId:'api:pi'}));
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({payload:expect.objectContaining({text:'保留这段侧边提问'})}));
});

it('selects a supplier and model for the side draft without changing or starting the main conversation', async () => {
  missing = false;
  await render({key:'draft',text:'我的侧边提问',connectionId:'api:pi'}, {catalog:{origin:'saved',data:[{id:'test-model',model:'test-model',displayName:'test-model',isDefault:true}],source:{providerId:'custom',providerName:'Pi API',mark:'P',available:true}},profiles:[
    {id:'api:pi',agent:'pi',name:'Pi API',model:'test-model',models:['test-model','other-model'],hasCredential:true,current:true},
    {id:'other:pi',agent:'pi',name:'Other API',model:'second-model',hasCredential:true,current:false},
    {id:'api:claude',agent:'claude',name:'Claude API',model:'claude-only',hasCredential:true,current:true},
  ]});
  expect(call).not.toHaveBeenCalledWith('create_session',expect.anything());
  await act(async()=>container.querySelector<HTMLButtonElement>('.model-chip')!.click());
  expect(container.querySelector('.model-popover')?.textContent).not.toContain('claude-only');
  await act(async()=>Array.from(container.querySelectorAll<HTMLButtonElement>('.model-option')).find(button=>button.textContent?.includes('second-model'))!.click());
  expect(container.querySelector<HTMLTextAreaElement>('textarea')!.value).toBe('我的侧边提问');
  expect(call).not.toHaveBeenCalledWith('switch_session_model',expect.anything());
  await send();
  expect(call).toHaveBeenCalledWith('create_session',expect.objectContaining({connectionId:'other:pi',model:'second-model',agent:'pi'}));
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({sessionId:'side',payload:expect.objectContaining({text:'我的侧边提问',model:'second-model'})}));
});

it('preserves the current side draft and its attachments when opening supplier settings', async () => {
  const manageProviders = vi.fn();
  const attachment = {kind:'skill' as const,name:'recall',path:'D:/skills/recall/SKILL.md'};
  await render({key:'draft',text:'进一步说明',payload:{...defaults,attachments:[attachment]}}, {manageProviders});
  await act(async()=>container.querySelector<HTMLButtonElement>('.model-chip')!.click());
  expect(manageProviders).toHaveBeenCalledWith('pi',expect.objectContaining({text:'进一步说明',payload:expect.objectContaining({attachments:[attachment]})}));
  expect(call).not.toHaveBeenCalledWith('create_session',expect.anything());
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="移除技能 recall"]')!.click());
  expect(container.querySelector('.composer-skill')).toBeNull();
  expect(container.querySelector<HTMLTextAreaElement>('textarea')!.value).toBe('进一步说明');
});

it('uses the configured send shortcut and retains the side permission choice in the next message', async () => {
  missing = false;
  await render({key:'draft',text:'侧边问题'}, {enterSend:false});
  const textarea = container.querySelector<HTMLTextAreaElement>('textarea')!;
  await act(async()=>textarea.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true})));
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="侧边聊天权限模式"]')!.click());
  await act(async()=>container.querySelector<HTMLButtonElement>('[role="menuitemradio"][aria-checked="false"]')!.click());
  await act(async()=>textarea.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',ctrlKey:true,bubbles:true,cancelable:true})));
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({payload:expect.objectContaining({text:'侧边问题',readOnly:false})}));
});

it('keeps a newer side draft while an earlier message is being submitted', async () => {
  missing = false;
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const original = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async (command,args={}) => {
    if(command==='check_chat_connection') await gate;
    return original(command,args);
  });
  await render({key:'draft',text:'先发送这条'});
  await act(async()=>container.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  const textarea = container.querySelector<HTMLTextAreaElement>('textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(textarea,'下一条还在编辑');textarea.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>{release();await gate;});
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({payload:expect.objectContaining({text:'先发送这条'})}));
  expect(textarea.value).toBe('下一条还在编辑');
});

it('notifies the selected connection before validation and keeps it protected when the side chat is hidden', async () => {
  missing=false;
  const pendingConnectionChanged=vi.fn();
  let validate!:()=>void,enqueue!:()=>void;
  const validation=new Promise<void>(resolve=>{validate=resolve;});
  const enqueuing=new Promise<void>(resolve=>{enqueue=resolve;});
  const original=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(['check_chat_connection','create_session','enqueue_followup'].includes(command))expect(pendingConnectionChanged).toHaveBeenLastCalledWith({agent:'pi',id:'selected:pi'});
    if(command==='check_chat_connection')await validation;
    if(command==='enqueue_followup')await enqueuing;
    return original(command,args);
  });
  const draft={key:'draft',text:'侧边发送',connectionId:'selected:pi'};
  await render(draft,{pendingConnectionChanged});await send();
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'pi',id:'selected:pi'}]]);
  await render(draft,{pendingConnectionChanged,connectionId:'unrelated:pi',hidden:true});
  expect(container.querySelector('aside')!.hidden).toBe(true);
  expect(pendingConnectionChanged).toHaveBeenCalledTimes(1);
  await act(async()=>{validate();await validation;});
  expect(call).toHaveBeenCalledWith('create_session',expect.objectContaining({connectionId:'selected:pi'}));
  expect(pendingConnectionChanged).toHaveBeenCalledTimes(1);
  await act(async()=>{enqueue();await enqueuing;});
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'pi',id:'selected:pi'}],[null]]);
});

it('keeps an unbound side draft on its initial route when the main default changes', async () => {
  missing=false;
  const draft={key:'draft',text:'保留侧边选择'};
  const pendingConnectionChanged=vi.fn();
  await render(draft,{connectionId:'original:pi',pendingConnectionChanged});
  await render(draft,{connectionId:'new-default:pi',pendingConnectionChanged});
  await send();
  expect(call).toHaveBeenCalledWith('check_chat_connection',{agent:'pi',connectionId:'original:pi',sessionId:null});
  expect(call).toHaveBeenCalledWith('create_session',expect.objectContaining({connectionId:'original:pi'}));
  expect(pendingConnectionChanged).toHaveBeenNthCalledWith(1,{agent:'pi',id:'original:pi'});
});

it.each(['check_chat_connection','create_session','enqueue_followup'])('clears the pending route when %s fails',async failedCommand=>{
  missing=false;
  const pendingConnectionChanged=vi.fn(),original=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>{if(command===failedCommand)throw new Error('提交失败');return original(command,args);});
  await render({key:'draft',text:'侧边发送',connectionId:'chosen:pi'},{pendingConnectionChanged});await send();
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'pi',id:'chosen:pi'}],[null]]);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain('提交失败');
});

it('clears a missing-provider pending route while preserving the side draft',async()=>{
  const pendingConnectionChanged=vi.fn();
  await render({key:'draft',text:'连接尚未配置',connectionId:'missing:pi'},{pendingConnectionChanged});await send();
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'pi',id:'missing:pi'}],[null]]);
  expect(providerRequired).toHaveBeenCalledWith('pi',expect.objectContaining({text:'连接尚未配置',connectionId:'missing:pi'}));
});

it.each(['check_chat_connection','create_session'])('clears an unmounted %s operation without clearing a newly opened side chat or enqueueing the old draft',async waitingCommand=>{
  missing=false;
  const pendingConnectionChanged=vi.fn();
  let oldRelease!:()=>void,newRelease!:()=>void;
  const oldGate=new Promise<void>(resolve=>{oldRelease=resolve;});
  const newGate=new Promise<void>(resolve=>{newRelease=resolve;});
  const original=vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(command===waitingCommand&&args.connectionId==='old:pi')await oldGate;
    if(command==='check_chat_connection'&&args.connectionId==='new:pi')await newGate;
    return original(command,args);
  });
  await render({key:'old',text:'旧侧边草稿',connectionId:'old:pi'},{pendingConnectionChanged});await send();
  await act(async()=>root.render(null));
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'pi',id:'old:pi'}],[null]]);
  await render({key:'new',text:'新侧边草稿',connectionId:'new:pi'},{pendingConnectionChanged});await send();
  expect(pendingConnectionChanged).toHaveBeenLastCalledWith({agent:'pi',id:'new:pi'});
  await act(async()=>{oldRelease();await oldGate;});
  expect(pendingConnectionChanged).toHaveBeenCalledTimes(3);
  expect(call).not.toHaveBeenCalledWith('enqueue_followup',expect.anything());
  if(waitingCommand==='check_chat_connection')expect(call).not.toHaveBeenCalledWith('create_session',expect.objectContaining({connectionId:'old:pi'}));
  await act(async()=>{newRelease();await newGate;});
  expect(pendingConnectionChanged).toHaveBeenLastCalledWith(null);
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({payload:expect.objectContaining({text:'新侧边草稿'})}));
});

it('does not clear a concurrent queue-resume pending operation when a send finishes',async()=>{
  missing=false;
  const pendingConnectionChanged=vi.fn(),original=vi.mocked(call).getMockImplementation()!;
  let sending=false,sendRelease!:()=>void,resumeRelease!:()=>void;
  const sendGate=new Promise<void>(resolve=>{sendRelease=resolve;});
  const resumeGate=new Promise<void>(resolve=>{resumeRelease=resolve;});
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(command==='list_followups')return [{id:'paused',sessionId:'side',status:'paused',error:null,payload:{...defaults,text:'暂缓的侧边补充'}}];
    if(command==='check_chat_connection'&&sending)await sendGate;
    if(command==='change_followup'&&args.action==='resume')await resumeGate;
    return original(command,args);
  });
  await render({key:'draft',text:'开始侧边聊天',connectionId:'bound:pi'},{pendingConnectionChanged});await send();
  pendingConnectionChanged.mockClear();sending=true;
  const input=container.querySelector<HTMLTextAreaElement>('[aria-label="侧边聊天消息"]')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,'新侧边任务');input.dispatchEvent(new Event('input',{bubbles:true}));});
  await send();
  await act(async()=>[...container.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent==='继续排队')!.click());
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'pi',id:'bound:pi'}],[{agent:'pi',id:'bound:pi'}]]);
  await act(async()=>{sendRelease();await sendGate;});
  expect(pendingConnectionChanged.mock.calls.some(([connection])=>connection===null)).toBe(false);
  await act(async()=>{resumeRelease();await resumeGate;});
  expect(pendingConnectionChanged).toHaveBeenLastCalledWith(null);
});

it('protects a goal start before validating and clears it when the start completes',async()=>{
  missing=false;
  const pendingConnectionChanged=vi.fn(),original=vi.mocked(call).getMockImplementation()!;
  let release!:()=>void;
  const gate=new Promise<void>(resolve=>{release=resolve;});
  vi.mocked(call).mockImplementation(async(command,args={})=>{
    if(command==='list_local_skills')return {skills:[],warnings:[]};
    if(command==='list_extensions')return {plugins:[],mcp:[],warnings:[]};
    if(command==='check_chat_connection'){expect(pendingConnectionChanged).toHaveBeenLastCalledWith({agent:'codex',id:'goal:codex'});await gate;}
    return original(command,args);
  });
  await render({key:'draft',text:'',connectionId:'goal:codex'},{agent:'codex',pendingConnectionChanged});
  await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="添加附件"]')!.click());
  await act(async()=>container.querySelector<HTMLButtonElement>('[data-goal-entry]')!.click());
  await act(async()=>{
    const input=container.querySelector<HTMLTextAreaElement>('.add-goal-panel textarea')!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,'完成侧边任务');input.dispatchEvent(new Event('input',{bubbles:true}));
  });
  await act(async()=>container.querySelector<HTMLButtonElement>('.add-goal-actions button')!.click());
  expect(pendingConnectionChanged.mock.calls).toEqual([[{agent:'codex',id:'goal:codex'}]]);
  await act(async()=>{release();await gate;});
  expect(call).toHaveBeenCalledWith('execute_agent_command',{sessionId:'side',command:'goal-create',argument:'完成侧边任务',permissionMode:'read'});
  expect(pendingConnectionChanged).toHaveBeenLastCalledWith(null);
});
