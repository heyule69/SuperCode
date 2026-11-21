import { expect, it } from 'vitest';
import { conversationTurns, mergeEvent } from './events';
import { shareConversationTurns } from './conversationCache';
import { indexedTurns } from './ConversationIndex';
import type { Message } from './types';

const message = (id: string, role: string, text: string): Message => ({ id, seq: 1, sessionId: 's', role, kind: 'agentMessage', text, data: { status: 'inProgress' } });
it('shares unchanged turns and cached navigation entries across a real stream reducer update', () => {
  const messages = [message('u1','user','first'), message('a1','assistant','first answer'), message('u2','user','second'), message('a2','assistant','new')];
  const first = shareConversationTurns([], conversationTurns(messages));
  const cache = new WeakMap(); const indexed = indexedTurns(first, cache);
  const updated = mergeEvent(messages, { method: 'item/agentMessage/delta', params: { itemId: 'a2', turnId: 't2', delta: ' content' } }, 's');
  const second = shareConversationTurns(first, conversationTurns(updated)), nextIndex = indexedTurns(second, cache);
  expect(second[0]).toBe(first[0]); expect(second[1]).not.toBe(first[1]);
  expect(nextIndex[0]).toBe(indexed[0]); expect(nextIndex[1].reply).toBe('new content');
});
it('invalidates a previously shared turn when its historical output changes', () => {
  const messages = [message('u1','user','first'), message('a1','assistant','partial'), message('u2','user','second')];
  const first = shareConversationTurns([], conversationTurns(messages));
  const second = shareConversationTurns(first, conversationTurns(messages.map(message => message.id === 'a1' ? { ...message, text: 'corrected history' } : message)));
  expect(second[0]).not.toBe(first[0]); expect(second[1]).toBe(first[1]);
});
it('refreshes the elapsed heading when an older native start is loaded without changing visible items',()=>{
  const user=message('u','user','完成整个任务');
  const last:Message={...message('last','activity',''),kind:'runMarker',data:{startedAt:10000,durationMs:2000,status:'completed'}};
  const reply=message('reply','assistant','完成');
  const first=shareConversationTurns([],conversationTurns([user,last,reply]));
  const original:Message={...last,id:'original',data:{startedAt:1000,durationMs:1000,status:'completed'}};
  const next=shareConversationTurns(first,conversationTurns([user,original,last,reply]));
  expect(next[0]).not.toBe(first[0]);
  expect(next[0].startedAt).toBe(1000);
  expect(next[0].run).toBe(last);
});
