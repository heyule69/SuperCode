// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import SideChat, { type SideDraft } from './SideChat';
import { call } from './api';
import { PROVIDER_REQUIRED } from './chatConnection';
import type { Session } from './types';

vi.mock('./api', () => ({ desktop:true, call:vi.fn(), subscribe:vi.fn(async()=>()=>{}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen:vi.fn(async()=>()=>{}) }));
const defaults = {text:'',model:'test-model',readOnly:true,permissionMode:'read',attachments:[],effort:null};
let root: Root, container: HTMLDivElement, missing: boolean, sessions: Session[];
const providerRequired = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  missing = true; sessions = []; providerRequired.mockClear();
  vi.mocked(call).mockReset();
  vi.mocked(call).mockImplementation(async (command,args={}) => {
    if(command==='check_chat_connection') { if(missing) throw PROVIDER_REQUIRED; return null; }
    if(command==='create_session') {
      const session: Session = {id:'side',projectId:String(args.projectId),title:'Side',agent:String(args.agent),connectionId:String(args.connectionId),model:'test-model',status:'idle',nativeId:null,turnId:null,updatedAt:1};
      sessions.push(session); return session;
    }
    if(command==='bootstrap') return {sessions};
    if(command==='runtime_info') return {requests:[]};
    if(['list_messages','list_followups'].includes(command)) return [];
    if(command==='followup_capabilities') return {steeringMode:'native'};
    return null;
  });
  container=document.createElement('div');document.body.append(container);root=createRoot(container);
});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();});
async function render(draft:SideDraft) {
  await act(async()=>root.render(<SideChat draft={draft} projectId="" agent="pi" connectionId={draft.connectionId??'@local'} defaults={defaults} close={()=>{}} opened={()=>{}} openFile={()=>{}} showDiff={()=>{}} providerRequired={providerRequired}/>));
}
async function send() {await act(async()=>{container.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));});}

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

it('can retry the preserved draft with the newly configured connection',async()=>{
  await render({key:'draft',text:'保留这段侧边提问'}); await send();
  const draft: SideDraft=providerRequired.mock.calls[0][1];
  missing=false;
  await render({...draft,connectionId:'api:pi'}); await send();
  expect(call).toHaveBeenCalledWith('check_chat_connection',{agent:'pi',connectionId:'api:pi',sessionId:null});
  expect(call).toHaveBeenCalledWith('create_session',expect.objectContaining({connectionId:'api:pi'}));
  expect(call).toHaveBeenCalledWith('enqueue_followup',expect.objectContaining({payload:expect.objectContaining({text:'保留这段侧边提问'})}));
});
