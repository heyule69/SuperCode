import type { ConversationTurn } from './events';

/** Stream reducers retain unchanged Message objects; carry that sharing through to turns. */
export function shareConversationTurns(previous: ConversationTurn[], next: ConversationTurn[]) {
  const byId = new Map(previous.map(turn => [turn.id, turn]));
  return next.map(turn => {
    const old = byId.get(turn.id);
    const sameRun = old?.run === turn.run || !!old?.run && !!turn.run && old.run.id === turn.run.id && old.run.text === turn.run.text && old.run.data === turn.run.data;
    return old && old.user === turn.user && sameRun && old.items.length === turn.items.length && old.items.every((message, index) => message === turn.items[index]) ? old : turn;
  });
}
