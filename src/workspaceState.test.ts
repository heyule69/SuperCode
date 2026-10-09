import { describe, expect, it } from 'vitest';
import { commandIndex, decodeDrafts, encodeDrafts, restoreWorkspace } from './workspaceState';

describe('workspace restoration', () => {
  const projects = [{ id: 'a', name: 'A', path: 'A' }, { id: 'b', name: 'B', path: 'B' }];
  const sessions = [{ id: 'old', projectId: 'a' }, { id: 'chosen', projectId: 'b' }] as Parameters<typeof restoreWorkspace>[0]['sessions'];
  it('keeps the selected conversation instead of the first conversation', () => {
    expect(restoreWorkspace({ projects, sessions }, { sessionId: 'chosen', projectId: 'a' })).toEqual({ project: projects[1], session: sessions[1] });
  });
  it('restores a project draft after the selected conversation was archived', () => {
    expect(restoreWorkspace({ projects, sessions }, { sessionId: 'archived', projectId: 'b' })).toEqual({ project: projects[1], session: undefined });
  });
  it('opens a new workspace without stealing the main window conversation', () => {
    expect(restoreWorkspace({ projects, sessions }, { sessionId: 'chosen', projectId: 'a', fresh: true })).toEqual({ project: undefined, session: undefined });
    expect(restoreWorkspace({ projects, sessions }, { sessionId: 'chosen', projectId: 'deleted', fresh: true })).toEqual({ project: undefined, session: undefined });
    expect(restoreWorkspace({ projects: [], sessions: [] }, { sessionId: '', projectId: '', fresh: true })).toEqual({ project: undefined, session: undefined });
  });
  it('starts without a project and restores projectless history independently of old project selection', () => {
    expect(restoreWorkspace({ projects, sessions }, { sessionId: '', projectId: '' })).toEqual({ project: undefined, session: undefined });
    const chat = { ...sessions[0], id: 'standalone', projectId: '' };
    expect(restoreWorkspace({ projects, sessions: [chat, ...sessions] }, { sessionId: 'standalone', projectId: 'b' })).toEqual({ project: undefined, session: chat });
  });
  it('bounds persisted drafts and ignores invalid records', () => {
    const texts = new Map(Array.from({ length: 25 }, (_, i) => [String(i), '草稿'.repeat(20000)]));
    const decoded = decodeDrafts(encodeDrafts(texts, new Map()));
    expect(decoded.size).toBe(20); expect(decoded.has('0')).toBe(false); expect(decoded.get('24')?.text.length).toBe(32000);
    expect(decodeDrafts('[null,["x",{"text":5}]]').size).toBe(0);
    expect(decodeDrafts('invalid').size).toBe(0);
  });
  it('preserves attachment-only drafts and removes empty drafts', () => {
    const attachment = { kind: 'image' as const, name: '图.png', path: 'D:/图.png' };
    expect([...decodeDrafts(encodeDrafts(new Map([['empty', '']]), new Map([['picture', [attachment]]])))]).toEqual([['picture', { text: '', attachments: [attachment] }]]);
  });
  it('restores a draft model and provider without persisting a conversation', () => {
    const selection = { connectionId: 'kimi', model: 'k3[1m]', effort: 'high' };
    const decoded = decodeDrafts(encodeDrafts(new Map(), new Map(), new Map([['project:claude', selection]])));
    expect(decoded.get('project:claude')).toEqual({ text: '', attachments: [], selection });
    expect(decodeDrafts('[["p",{"text":"hi","selection":{"connectionId":3,"model":"k3"}}]]').get('p')?.selection).toBeUndefined();
  });
  it('wraps keyboard command selection without an invalid index', () => {
    expect(commandIndex('ArrowUp', 0, 4)).toBe(3); expect(commandIndex('ArrowDown', 3, 4)).toBe(0); expect(commandIndex('ArrowDown', 1, 0)).toBe(0);
  });
});
