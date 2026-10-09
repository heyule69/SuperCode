// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { call } from './api';
import type { Bootstrap, Message, Session } from './types';

vi.mock('./api', () => ({ desktop:true, call:vi.fn(), subscribe:vi.fn(async()=>()=>{}) }));
vi.mock('@tauri-apps/api/event', () => ({ listen:vi.fn(async()=>()=>{}) }));
vi.mock('./windowTheme', () => ({ watchWindowTheme:()=>()=>{} }));
vi.mock('./DesktopTitleBar', () => ({ DesktopTitleBar:()=>null }));
vi.mock('./PlatformUsageView', () => ({ PlatformUsageIndicator:()=>null }));
vi.mock('./AppUpdate', () => ({ default:()=>null, AppUpdateNotice:()=>null }));
let root: Root, container: HTMLDivElement, data: Bootstrap, messages: Message[];
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); vi.mocked(call).mockClear(); messages = [];
  data = { projects:[{id:'existing',name:'已有项目',path:'D:/existing'}],sessions:[],agents:[],profiles:[],officialAgents:[],connectionOrder:{},sidebar:{ projects:{}, sessions:{}, sections:[] },codexPath:null,loadMcp:false } as Bootstrap;
  vi.mocked(call).mockImplementation(async (command, args={}) => {
    if (command==='bootstrap') return structuredClone(data);
    if (command==='list_models') return { data:[], source:undefined };
    if (command==='list_messages') return [...messages];
    if (command==='get_usage') return { records:[] };
    if (command==='pending_requests') return [];
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
it('sends a projectless draft without opening the project picker and keeps it in recents', async () => {
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
  expect(container.querySelector('[aria-label="最近对话"]')).not.toBeNull();
  expect(localStorage.getItem('supercode.project')).toBe('');
  await act(async()=>container.querySelector<HTMLButtonElement>('[title="会话设置"]')!.click());
  await act(async()=>{ [...container.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent==='归档会话')!.click(); await new Promise(resolve=>setTimeout(resolve,30)); });
  expect(call).toHaveBeenCalledWith('archive_session', {sessionId:'standalone'});
  expect(container.querySelector('.new-chat-welcome')).not.toBeNull();
  await act(async()=>container.querySelector<HTMLButtonElement>('.new-chat')!.click());
  expect(container.querySelector('[aria-label="选择项目"]')?.textContent).toBe('SuperCode');
});
