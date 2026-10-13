// @vitest-environment happy-dom
import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useChatHistory } from './useChatHistory';
import { call } from './api';
import type { Message } from './types';

vi.mock('./api', () => ({ call: vi.fn() }));
let root: Root, node: HTMLDivElement, history: ReturnType<typeof useChatHistory>;
const message = (text: string): Message => ({ id:'answer',seq:2,sessionId:'chat',role:'assistant',kind:'agentMessage',text,data:{status:'inProgress',turnId:'turn'} });
function Harness() {
  const current = useRef({sessionId:'chat'}), users = useRef(new Set<string>()), messages = useRef(new Map<string,Message>()), area = useRef<HTMLElement>(null), scroll = useRef<{height:number;top:number}|null>(null);
  history = useChatHistory(current,users,messages,area,scroll);
  return <div>{history.messages.map(message=>message.text).join('|')}</div>;
}
beforeEach(async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; vi.useFakeTimers(); vi.mocked(call).mockReset();
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  await act(async () => root.render(<Harness/>));
  await act(async () => history.setMessages([message('A')]));
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); vi.useRealTimers(); });
it.each(['A','AB'])('consumes queued delta before a delayed %s snapshot and never appends it twice', async snapshot => {
  let finish!: (messages: Message[]) => void;
  vi.mocked(call).mockImplementation(() => new Promise(resolve=>{finish=resolve;}));
  let read!: Promise<void>; act(() => {read=history.loadMessages('chat');});
  act(() => history.enqueue({method:'item/agentMessage/delta',params:{itemId:'answer',turnId:'turn',delta:'B'}}));
  await act(async () => {finish([message(snapshot)]);await read;});
  expect(history.messages[0].text).toBe('AB');
  await act(async () => vi.advanceTimersByTime(60));
  expect(history.messages[0].text).toBe('AB');
});
it('keeps queued live output when an older page is prepended', async () => {
  let finish!: (messages: Message[]) => void;
  vi.mocked(call).mockImplementation(() => new Promise(resolve=>{finish=resolve;}));
  let read!: Promise<void>; act(() => {read=history.loadMessages('chat',2);});
  act(() => history.enqueue({method:'item/agentMessage/delta',params:{itemId:'answer',turnId:'turn',delta:'B'}}));
  await act(async () => {finish([{...message('old page'),id:'old',seq:1}]);await read;vi.advanceTimersByTime(60);});
  expect(history.messages.map(message=>message.text)).toEqual(['old page','AB']);
});
