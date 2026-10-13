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
