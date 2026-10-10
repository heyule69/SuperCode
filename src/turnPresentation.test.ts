import { describe, expect, it } from 'vitest';
import { finalReply, presentTurn } from './turnPresentation';
import type { Message } from './types';

const reply = (id: string, text: string, data: Message['data'] = null): Message => ({ id, seq: 0, sessionId: 's', role: 'assistant', kind: 'agentMessage', text, data });

describe('final reply selection', () => {
  it('prefers explicit Codex final phases even when cleanup commentary arrives afterwards', () => {
    const final = reply('final', '结果', { phase: 'final_answer' });
    expect(finalReply([reply('progress', '先调查', { phase: 'commentary' }), final, reply('cleanup', '清理缓存', { phase: 'commentary' })])).toEqual([final]);
    expect(finalReply([reply('progress', '只有进度', { phase: 'commentary' })])).toEqual([]);
  });
  it('uses the last native answer for Claude, OpenCode and legacy history without changing stored messages', () => {
    const items = [reply('before', '调查说明'), reply('after', '最终答复')];
    expect(finalReply(items)).toEqual([items[1]]);
    expect(items.map(item => item.text)).toEqual(['调查说明', '最终答复']);
  });
  it('keeps all Pi final content blocks and attachments together, without preceding explanations', () => {
    const items = [reply('old', '进度', { messageId: 'm1' }), reply('text', '完成', { messageId: 'm2' }), reply('picture', '', { messageId: 'm2', media: [{ kind: 'image', path: 'D:/output/结果.png' }] })];
    expect(finalReply(items)).toEqual(items.slice(1));
    expect(finalReply([reply('empty', ''), items[2]])).toEqual([items[2]]);
  });
  it('preserves runtime warnings outside collapsed history and leaves failed or interrupted runs intact', () => {
    const notice = { ...reply('warning', '连接错误'), role: 'system' };
    const items = [reply('progress', '进度'), reply('final', '结果'), notice];
    const run = { ...reply('run', ''), role: 'activity', kind: 'runMarker', data: { status: 'completed' } };
    expect(presentTurn({ id: 'u', items, run })).toMatchObject({ final: [items[1]], process: [items[0]], notices: [notice], completed: true });
    for (const status of ['failed', 'interrupted', 'inProgress']) {
      expect(presentTurn({ id: 'u', items, run: { ...run, data: { status } } }).completed).toBe(false);
    }
    expect(presentTurn({ id: 'u', items }).completed).toBe(false);
  });
});
