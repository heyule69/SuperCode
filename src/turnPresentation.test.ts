import { describe, expect, it } from 'vitest';
import { finalReply, presentTurn } from './turnPresentation';
import type { Message } from './types';

const reply = (id: string, text: string, data: Message['data'] = null): Message => ({ id, seq: 0, sessionId: 's', role: 'assistant', kind: 'agentMessage', text, data });

describe('final reply selection', () => {
  it('keeps planning open during approval, a newer run or continuing output despite an earlier completion', () => {
    const run = { ...reply('run', ''), kind:'runMarker', data:{ status:'completed', turnId:'first' } };
    const turn = { id:'u', run, items:[reply('progress', '仍在设计方案', { turnId:'first' })] };
    for (const lifecycle of [
      { sessionStatus:'waiting', activeTurnId:'first', latest:true },
      { sessionStatus:'running', activeTurnId:'next', latest:true },
      { sessionStatus:'starting', latest:true },
    ]) expect(presentTurn(turn, lifecycle)).toMatchObject({ completed:false, settled:false, active:true });
    expect(presentTurn({ ...turn, items:[...turn.items, reply('new', '追加方案', { status:'inProgress', turnId:'first' })] },
      { sessionStatus:'running', activeTurnId:'first', latest:true }).completed).toBe(false);
    for (const lifecycle of [
      { sessionStatus:'idle', latest:true },
      { sessionStatus:'running', activeTurnId:'first', latest:true },
      { sessionStatus:'waiting', activeTurnId:'next', latest:false },
    ]) expect(presentTurn(turn, lifecycle).completed).toBe(true);
  });
  it('prefers explicit Codex final phases even when cleanup commentary arrives afterwards', () => {
    const final = reply('final', '结果', { phase: 'final_answer' });
    expect(finalReply([reply('progress', '先调查', { phase: 'commentary' }), final, reply('cleanup', '清理缓存', { phase: 'commentary' })])).toEqual([final]);
    expect(finalReply([reply('progress', '只有进度', { phase: 'commentary' })])).toEqual([]);
  });
  it('keeps the complete unphased native answer across messages and tool calls without changing stored history', () => {
    const first = reply('first', '第一类报错：没有安装 Python 和 Node。');
    const tool = { ...reply('tool', '验证通过'), role:'tool', kind:'commandExecution' };
    const second = reply('second', '第二类报错：路径配置不正确。');
    const items = [first, tool, second];
    expect(finalReply(items)).toEqual([first, second]);
    expect(presentTurn({ id:'u', items }, { sessionStatus:'idle' })).toMatchObject({ final:[first, second], process:[tool] });
    expect(items.map(item => item.text)).toEqual(['第一类报错：没有安装 Python 和 Node。', '验证通过', '第二类报错：路径配置不正确。']);
  });
  it('keeps all native content blocks and attachments together while excluding explicit commentary', () => {
    const items = [reply('old', '进度', { messageId: 'm1', phase:'commentary' }), reply('text', '完成', { messageId: 'm2' }), reply('picture', '', { messageId: 'm2', media: [{ kind: 'image', path: 'D:/output/结果.png' }] })];
    expect(finalReply(items)).toEqual(items.slice(1));
    expect(finalReply([reply('empty', ''), items[2]])).toEqual([items[2]]);
  });
  it('preserves runtime warnings outside collapsed history and leaves failed or interrupted runs intact', () => {
    const notice = { ...reply('warning', '连接错误'), role: 'system' };
    const items = [reply('progress', '进度', { phase:'commentary' }), reply('final', '结果'), notice];
    const run = { ...reply('run', ''), role: 'activity', kind: 'runMarker', data: { status: 'completed' } };
    expect(presentTurn({ id: 'u', items, run })).toMatchObject({ final: [items[1]], process: [items[0]], notices: [notice], completed: true });
    for (const status of ['failed', 'interrupted', 'inProgress']) {
      expect(presentTurn({ id: 'u', items, run: { ...run, data: { status } } }).completed).toBe(false);
    }
    expect(presentTurn({ id: 'u', items }).completed).toBe(false);
  });
  it('folds loaded legacy history with missing or stale lifecycle metadata using the idle session', () => {
    const items = [reply('progress', '先检查文件', { phase:'commentary' }), reply('final', '已完成')];
    for (const run of [undefined, { ...reply('run', ''), kind: 'runMarker', data: { status: 'inProgress', turnId: 'old' } }]) {
      expect(presentTurn({ id: 'u', items, run }, { sessionStatus: 'idle', latest: true }))
        .toMatchObject({ completed: true, final: [items[1]], process: [items[0]] });
    }
    expect(presentTurn({ id: 'u', items: [reply('commentary', '还在处理', { phase: 'commentary' })] }, { sessionStatus: 'idle' }).completed).toBe(false);
  });
  it('does not fold active, failed or interrupted history based on a completed individual tool or an idle fallback', () => {
    const items = [reply('progress', '进度'), reply('partial', '部分结果')];
    for (const sessionStatus of ['starting', 'running', 'waiting', 'failed', 'interrupted', undefined]) {
      expect(presentTurn({ id: 'u', items }, { sessionStatus, latest: true }).completed).toBe(false);
    }
    for (const status of ['failed', 'interrupted']) {
      const run = { ...reply('run', ''), kind: 'runMarker', data: { status } };
      expect(presentTurn({ id: 'u', items, run }, { sessionStatus: 'idle' }).completed).toBe(false);
    }
    const old = { id: 'u', items, run: { ...reply('run', ''), kind: 'runMarker', data: { status: 'inProgress', turnId: 'old' } } };
    expect(presentTurn(old, { sessionStatus: 'running', activeTurnId: 'new', latest: false }).completed).toBe(true);
    expect(presentTurn(old, { sessionStatus: 'running', activeTurnId: 'old', latest: false }).completed).toBe(false);
    expect(presentTurn(old, { sessionStatus: 'starting', latest: false }).completed).toBe(false);
  });
  it('shows the authoritative goal state without an obsolete active acknowledgement and retains clear feedback', () => {
    const command = { ...reply('command','目标执行中'), role:'system', kind:'commandResult', data:{ command:'goal-create', result:{goal:{status:'active'}} } };
    const active = { ...reply('active','目标执行中'), role:'system', kind:'goalState', data:{goal:{status:'active'}} };
    const complete = { ...active, id:'complete', text:'目标已完成', data:{goal:{status:'complete'}} };
    const items = [command,active,complete,reply('answer','完整回答')];
    expect(presentTurn({id:'goal',items},{sessionStatus:'idle'}).notices).toEqual([complete]);
    const cleared = { ...complete,id:'clear-state',data:{goal:null} };
    const feedback = { ...command,id:'clear-command',text:'目标已清除',data:{command:'goal',result:{goal:null}} };
    expect(presentTurn({id:'goal',items:[...items,cleared,feedback]},{sessionStatus:'idle'}).notices).toEqual([cleared,feedback]);
    expect(items[0].text).toBe('目标执行中');
  });
});
