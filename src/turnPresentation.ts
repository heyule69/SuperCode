import type { ConversationTurn } from './events';
import { messageMedia } from './mediaSources';
import { isActive, type Message } from './types';

const visibleReply = (message: Message) => message.role === 'assistant' && (!!message.text.trim() || messageMedia(message).length > 0);

// A native response can contain several text/media blocks. Keep that complete
// response, rather than every intermediate response from earlier tool rounds.
function replyGroup(message: Message): string | undefined {
  const native = message.data?.messageId;
  if (typeof native === 'string' && native) return native;
  // Older Pi history predates messageId, but its block IDs still identify the
  // native message: <turn>-msg-<message>-<contentIndex>.
  return /^(.*-msg-\d+)-\d+$/.exec(message.id)?.[1];
}
export function finalReply(items: Message[]): Message[] {
  const replies = items.filter(visibleReply);
  const explicit = replies.filter(message => message.data?.phase === 'final_answer');
  if (explicit.length) return explicit;
  const candidates = replies.filter(message => message.data?.phase !== 'commentary');
  const last = candidates.at(-1);
  if (!last) return [];
  const group = replyGroup(last);
  if (group) return candidates.filter(message => replyGroup(message) === group);
  // Ungrouped legacy/ACP fragments remain together until an earlier tool,
  // thought or explicit commentary marks a different response. A trailing
  // cleanup operation must not hide the preceding answer.
  const final: Message[] = [];
  for (let index = items.lastIndexOf(last); index >= 0; index--) {
    const message = items[index];
    if (message.role === 'system') continue;
    if (message.role !== 'assistant' || message.data?.phase === 'commentary' || replyGroup(message)) break;
    if (visibleReply(message)) final.unshift(message);
  }
  return final;
}

export interface TurnLifecycle { sessionStatus?: string; activeTurnId?: string | null; latest?: boolean }

export function presentTurn(turn: ConversationTurn, lifecycle: TurnLifecycle = {}) {
  const status = turn.run?.data?.status;
  const turnId = turn.run?.data?.turnId ?? turn.items.find(message => message.data?.turnId)?.data?.turnId;
  const terminal = ['completed', 'failed', 'interrupted'].includes(String(status));
  const continuing = turn.items.some(message => ['inProgress', 'running', 'preparing'].includes(String(message.data?.status))
    && (!lifecycle.activeTurnId || !message.data?.turnId || message.data.turnId === lifecycle.activeTurnId));
  // Resumed planning can share a UI turn with a previous native result. A
  // live approval, newer native turn or continuing output outweighs it.
  // A matching completion still wins over a briefly stale running session.
  const active = lifecycle.latest !== false && isActive(lifecycle.sessionStatus)
    && (lifecycle.sessionStatus !== 'running' || !terminal || continuing
      || !!lifecycle.activeTurnId && turnId !== lifecycle.activeTurnId);
  // Legacy history may have no marker, or a marker evicted before completion
  // was persisted. An idle session is authoritative for its finished history.
  // During a run, only a different, identified older turn can use this fallback.
  const inactive = lifecycle.sessionStatus === 'idle' || lifecycle.latest === false
    && isActive(lifecycle.sessionStatus) && !!turnId && !!lifecycle.activeTurnId && turnId !== lifecycle.activeTurnId;
  const completed = !active && (status === 'completed' || !['failed', 'interrupted'].includes(String(status))
    && inactive && finalReply(turn.items).length > 0);
  const final = completed ? finalReply(turn.items) : [];
  const resultIds = new Set(final.map(message => message.id));
  const latestGoal = turn.items.filter(message => message.kind === 'goalState').at(-1);
  const notices = turn.items.filter(message => {
    if (message.role !== 'system' || message.kind === 'turnDiff') return false;
    if (message.kind === 'goalState') return message === latestGoal;
    // The original command acknowledgement can still say "active" after the
    // native goal completes. Display its authoritative state once, retaining
    // explicit clear/no-goal feedback and every original stored record.
    const result = message.data?.result;
    return !(latestGoal && message.kind === 'commandResult'
      && ['goal', 'goal-create', 'goal-status'].includes(String(message.data?.command))
      && result && typeof result === 'object' && 'goal' in result && result.goal);
  });
  const process = turn.items.filter(message => !['turnDiff','turnArtifacts'].includes(message.kind) && message.role !== 'system' && !resultIds.has(message.id) && (message.role !== 'assistant' || visibleReply(message)));
  return { completed, settled: !active && (terminal || completed), active, final, notices, process };
}
