import { bench, describe } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { Conversation } from './Conversation';
import { conversationTurns, mergeEvent } from './events';
import type { Message } from './types';

// Fixed synthetic workloads. This measures the pipeline, not a live agent/WebView.
const history: Message[] = Array.from({ length: 300 }, (_, turn) => [
  { seq: turn * 3, id: `user-${turn}`, sessionId: 'bench', role: 'user', kind: 'userMessage', text: '检查这个修改并解释结果。', data: null },
  { seq: turn * 3 + 1, id: `run-${turn}`, sessionId: 'bench', role: 'activity', kind: 'runMarker', text: '', data: { status: 'completed', turnId: `turn-${turn}` } },
  { seq: turn * 3 + 2, id: `reply-${turn}`, sessionId: 'bench', role: 'assistant', kind: 'agentMessage', text: '已完成检查。'.repeat(40), data: { status: 'completed' } },
]).flat();
const streaming: Message[] = [...history, { seq: 901, id: 'live', sessionId: 'bench', role: 'assistant', kind: 'agentMessage', text: '输出'.repeat(16_000), data: { status: 'inProgress' } }];
const noop = () => {};
describe('bounded conversation workloads', () => {
  bench('merge one delta into 900 historical messages and 32K live text', () => {
    mergeEvent(streaming, { method: 'item/agentMessage/delta', params: { threadId: 'native', itemId: 'live', delta: '新的片段' } }, 'bench');
  }, { time: 250, warmupTime: 100 });
  bench('group 900 historical messages', () => { conversationTurns(history); }, { time: 250, warmupTime: 100 });
  bench('render 30 completed turns to static markup', () => { renderToStaticMarkup(<Conversation messages={history.slice(0, 90)} agentName="Benchmark fixture" autoExpand={false} showDiff={noop} openFile={noop}/>); }, { time: 1000, warmupTime: 250 });
});
