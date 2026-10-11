import type { ConversationTurn } from './events';
import { messageMedia } from './mediaSources';
import { isActive, type Message } from './types';

const visibleReply = (message: Message) => message.role === 'assistant' && (!!message.text.trim() || messageMedia(message).length > 0);

// Prefer an agent's explicit final phase. Older/native protocols without phases
// use the last visible assistant message; grouped content keeps text and media together.
export function finalReply(items: Message[]): Message[] {
  const replies = items.filter(visibleReply);
  const explicit = replies.filter(message => message.data?.phase === 'final_answer');
  if (explicit.length) return explicit;
  const last = replies.filter(message => message.data?.phase !== 'commentary').at(-1);
  if (!last) return [];
  const group = last.data?.messageId;
  return typeof group === 'string' && group ? replies.filter(message => message.data?.messageId === group && message.data?.phase !== 'commentary') : [last];
}

export interface TurnLifecycle { sessionStatus?: string; activeTurnId?: string | null; latest?: boolean }

export function presentTurn(turn: ConversationTurn, lifecycle: TurnLifecycle = {}) {
  const status = turn.run?.data?.status;
  const turnId = turn.run?.data?.turnId ?? turn.items.find(message => message.data?.turnId)?.data?.turnId;
  // Legacy history may have no marker, or a marker evicted before completion
  // was persisted. An idle session is authoritative for its finished history.
  // During a run, only a different, identified older turn can use this fallback.
  const inactive = lifecycle.sessionStatus === 'idle' || lifecycle.latest === false
    && isActive(lifecycle.sessionStatus) && !!turnId && !!lifecycle.activeTurnId && turnId !== lifecycle.activeTurnId;
  const completed = status === 'completed' || !['failed', 'interrupted'].includes(String(status))
    && inactive && finalReply(turn.items).length > 0;
  const final = completed ? finalReply(turn.items) : [];
  const resultIds = new Set(final.map(message => message.id));
  const notices = turn.items.filter(message => message.role === 'system' && message.kind !== 'turnDiff');
  const process = turn.items.filter(message => message.kind !== 'turnDiff' && message.role !== 'system' && !resultIds.has(message.id) && (message.role !== 'assistant' || visibleReply(message)));
  return { completed, final, notices, process };
}
