import { conversationTurns } from './events';
import { diffFiles } from './diff';
import type { Message } from './types';

export function currentTurnChanges(messages: Message[], activeTurnId?: string | null) {
  const turn = conversationTurns(messages).at(-1);
  const turnId = String(turn?.run?.data?.turnId ?? '');
  const files = new Map<string, ReturnType<typeof diffFiles>[number]>();
  if (!turn || activeTurnId && turnId !== activeTurnId) return { id: turn?.id ?? '', files: [], diff: '' };
  for (const message of turn.items) {
    if (message.kind !== 'turnDiff' || turnId && message.data?.turnId !== turnId) continue;
    // Native Codex notifications replace the complete turn snapshot. Other
    // adapters store one cumulative snapshot per file, including an empty revert.
    const path = message.data?.path;
    if (typeof path === 'string') files.delete(path);
    else files.clear();
    for (const file of diffFiles(message.text)) files.set(file.path, file);
  }
  const changed = [...files.values()];
  return { id: turn.id, files: changed, diff: changed.map(file => file.diff).join('') };
}
