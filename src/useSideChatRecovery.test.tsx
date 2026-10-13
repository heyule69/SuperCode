// @vitest-environment happy-dom
import { act, useRef, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call, subscribe } from './api';
import { useSideChatRecovery } from './useSideChatRecovery';
import type { Message, RpcEvent, Session } from './types';

vi.mock('./api', () => ({ call: vi.fn(), subscribe: vi.fn(), desktop: true }));
const initial: Session = { id: 'side', projectId: '', agent: 'codex', title: 'side', model: 'test', status: 'running', nativeId: 'native', turnId: 'turn', updatedAt: 1 };
const message = (text: string, status = 'inProgress'): Message => ({ id: 'answer', seq: 1, sessionId: 'side', role: 'assistant', kind: 'agentMessage', text, data: { turnId: 'turn', status } });
let root: Root, node: HTMLDivElement, api: ReturnType<typeof useSideChatRecovery>, notify: (event: RpcEvent) => void;
let saved: Message[], requests: RpcEvent[], session: Session;
const report = vi.fn(), changed = vi.fn();
function Harness({ hidden = false, boot = initial }: { hidden?: boolean; boot?: Session }) {
  const [value, setValue] = useState(boot), current = useRef<Session | undefined>(value); current.current = value;
  api = useSideChatRecovery(current, next => { setValue(next); changed(next); }, report);
  return <aside hidden={hidden}>{api.messages.map(message => message.text).join('|')}<span>{value.status}</span><span>{api.requests.map(request => request.id).join(',')}</span></aside>;
}
beforeEach(async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; vi.useFakeTimers(); saved = []; requests = []; session = { ...initial }; report.mockClear(); changed.mockClear();
  vi.mocked(subscribe).mockReset(); vi.mocked(call).mockReset();
  vi.mocked(subscribe).mockImplementation(async callback => { notify = callback; return () => {}; });
  vi.mocked(call).mockImplementation(async command => command === 'bootstrap' ? { sessions: [session] } : command === 'list_messages' ? saved : { requests });
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  await act(async () => root.render(<Harness/>));
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); vi.useRealTimers(); });
const delta = (text: string): RpcEvent => ({ method: 'item/agentMessage/delta', params: { threadId: 'native', turnId: 'turn', itemId: 'answer', delta: text } });
it('preserves streamed text received during a delayed snapshot without duplicating persisted deltas', async () => {
  await act(async () => { notify(delta('A')); vi.advanceTimersByTime(50); });
  const base = vi.mocked(call).getMockImplementation()!; let finish!: (rows: Message[]) => void;
  vi.mocked(call).mockImplementation((command, args) => command === 'list_messages' ? new Promise(resolve => { finish = resolve; }) : base(command, args));
  let read!: Promise<void>; act(() => { read = api.refresh(); });
  await act(async () => { notify(delta('B')); vi.advanceTimersByTime(50); });
  await act(async () => { finish([message('A')]); await read; });
  expect(api.messages[0].text).toBe('AB');
  let next!: Promise<void>; act(() => { next = api.refresh(); });
  await act(async () => { notify(delta('C')); finish([message('ABC')]); await next; });
  expect(api.messages[0].text).toBe('ABC');
});
it('keeps a completed item, turn and resolved approval when older snapshots return', async () => {
  const approval: RpcEvent = { id: 'approval', method: 'item/commandExecution/requestApproval', params: { threadId: 'native', turnId: 'turn' } };
  await act(async () => { notify({ method: 'turn/started', params: { threadId: 'native', turn: { id: 'turn' } } }); notify(delta('A')); notify(approval); });
  const base = vi.mocked(call).getMockImplementation()!; let finish!: (rows: Message[]) => void;
  requests = [approval]; vi.mocked(call).mockImplementation((command, args) => command === 'list_messages' ? new Promise(resolve => { finish = resolve; }) : base(command, args));
  let read!: Promise<void>; act(() => { read = api.refresh(); });
  await act(async () => {
    notify({ method: 'item/completed', params: { threadId: 'native', turnId: 'turn', item: { id: 'answer', type: 'agentMessage', text: '最终结果' } } });
    notify({ method: 'serverRequest/resolved', params: { threadId: 'native', requestId: 'approval' } });
    notify({ method: 'turn/completed', params: { threadId: 'native', turn: { id: 'turn', status: 'completed' } } });
    finish([message('A'), { id: 'run-turn', seq: 2, sessionId: 'side', role: 'activity', kind: 'runMarker', text: '', data: { turnId: 'turn', status: 'inProgress' } }]); await read;
  });
  expect(api.messages.find(message => message.id === 'answer')?.text).toBe('最终结果');
  expect(api.messages.find(message => message.id === 'run-turn')?.data?.status).toBe('completed');
  expect(api.requests).toEqual([]); expect(node.textContent).toContain('idle');
});
it('ignores another chat turn and continues its own subscription while hidden', async () => {
  await act(async () => root.render(<Harness hidden/>)); vi.mocked(call).mockClear();
  await act(async () => { notify({ method: 'turn/completed', params: { threadId: 'other', turn: { id: 'other-turn', status: 'completed' } } }); vi.advanceTimersByTime(200); });
  expect(call).not.toHaveBeenCalled();
  await act(async () => { notify(delta('hidden output')); vi.advanceTimersByTime(50); });
  expect(api.messages[0].text).toBe('hidden output');
  expect(node.querySelector('aside')?.hidden).toBe(true);
});
it('ignores out of order snapshots and callbacks after unmount', async () => {
  const base = vi.mocked(call).getMockImplementation()!; const completions: ((rows: Message[]) => void)[] = [];
  vi.mocked(call).mockImplementation((command, args) => command === 'list_messages' ? new Promise(resolve => completions.push(resolve)) : base(command, args));
  let older!: Promise<void>, newer!: Promise<void>; act(() => { older = api.refresh(); newer = api.refresh(); });
  await act(async () => { completions[1]([message('new')]); await newer; completions[0]([message('old')]); await older; });
  expect(api.messages[0].text).toBe('new');
  let late!: Promise<void>; act(() => { late = api.refresh(); });
  await act(async () => root.render(null)); changed.mockClear();
  await act(async () => { completions[2]([message('closed')]); await late; });
  expect(changed).not.toHaveBeenCalled();
});
it('keeps a new approval received while the runtime snapshot is pending', async () => {
  let finish!: (info: { requests: RpcEvent[] }) => void; const base = vi.mocked(call).getMockImplementation()!;
  vi.mocked(call).mockImplementation((command, args) => command === 'runtime_info' ? new Promise(resolve => { finish = resolve; }) : base(command, args));
  let read!: Promise<void>; act(() => { read = api.refresh(); });
  const approval: RpcEvent = { id: 'new', method: 'item/commandExecution/requestApproval', params: { threadId: 'native', turnId: 'turn' } };
  await act(async () => { notify(approval); finish({ requests: [] }); await read; });
  expect(api.requests).toEqual([approval]);
});
it.each(['pi','opencode','gemini'])('rebinds %s boot native ID and keeps owner-scoped startup requests', async agent => {
  const boot = {...initial,agent,nativeId:'boot-native',status:'starting'};
  await act(async () => {root.render(null);});
  await act(async () => root.render(<Harness boot={boot}/>));
  session = {...boot,nativeId:'actual-native',status:'running'};
  const approval: RpcEvent = {id:'boot-approval',method:'claude/tool/requestApproval',params:{appSessionId:'side',threadId:'older-boot-native',toolName:'BOOT_APPROVAL'}};
  await act(async () => {
    notify(approval);
    notify({method:'thread/started',params:{appSessionId:'side',thread:{id:'actual-native'}}});
    notify({method:'turn/started',params:{appSessionId:'side',threadId:'actual-native',turn:{id:'turn'}}});
    notify({method:'item/agentMessage/delta',params:{appSessionId:'side',threadId:'actual-native',turnId:'turn',itemId:'answer',delta:'actual output'}});
    notify({id:'other',method:'approval',params:{appSessionId:'other-chat',threadId:'actual-native'}});
    vi.advanceTimersByTime(50);
  });
  expect(api.messages.find(message=>message.id==='answer')?.text).toBe('actual output'); expect(api.requests).toEqual([approval]);
  expect(changed.mock.calls.at(-1)![0].nativeId).toBe('actual-native');
  requests = [approval,{id:'other',method:'approval',params:{appSessionId:'other-chat',threadId:'actual-native'}}]; saved = [...api.messages];
  await act(async () => api.refresh());
  expect(api.requests).toEqual([approval]);
});
it('keeps a Codex child startup separate from its already bound primary native', async () => {
  await act(async () => {
    notify({method:'thread/started',params:{appSessionId:'side',thread:{id:'child',source:{subAgent:{}}}}});
    notify({method:'turn/started',params:{appSessionId:'side',threadId:'child',turn:{id:'child-turn'}}});
    notify({method:'item/agentMessage/delta',params:{appSessionId:'side',threadId:'child',turnId:'child-turn',itemId:'child-answer',delta:'child output'}});
    notify(delta('parent output')); vi.advanceTimersByTime(50);
  });
  expect(api.messages.map(message=>message.text)).toEqual(['parent output']);
  expect(changed).not.toHaveBeenCalled();
});
