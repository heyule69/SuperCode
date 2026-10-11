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
