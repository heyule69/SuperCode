// @vitest-environment happy-dom
import { beforeEach, expect, it } from 'vitest';
import { draftMigrationKey, draftPrefix, loadWorkspaceDrafts, readWorkspaceDraft, writeWorkspaceDraft } from './workspaceDrafts';
import { encodeDrafts } from './workspaceState';

beforeEach(() => localStorage.clear());
it('writes independent chat rows after two windows load the same initial snapshot', () => {
  const firstWindow = loadWorkspaceDrafts(), secondWindow = loadWorkspaceDrafts();
  firstWindow.set('A', { text: '第一份草稿', attachments: [] });
  secondWindow.set('B', { text: '第二份草稿', attachments: [] });
  writeWorkspaceDraft('A', firstWindow.get('A'));
  writeWorkspaceDraft('B', secondWindow.get('B'));
  writeWorkspaceDraft('A', { text: '第一份修改', attachments: [] });
  expect([...loadWorkspaceDrafts()].map(([key, draft]) => [key, draft.text]).sort()).toEqual([['A', '第一份修改'], ['B', '第二份草稿']]);
});
it('migrates legacy text, attachments and model selection without overwriting an existing live row', () => {
  const selection = { connectionId: 'glm', model: 'glm-5.3', effort: 'high' };
  const attachment = { kind: 'image' as const, name: '图.png', path: 'D:/图.png' };
  localStorage.setItem('supercode.drafts.v1', encodeDrafts(new Map([['legacy', '旧草稿'], ['live', '过时内容']]), new Map([['legacy', [attachment]]]), new Map([['legacy', selection]])));
  writeWorkspaceDraft('live', { text: '刚刚输入', attachments: [] });
  expect(loadWorkspaceDrafts().get('legacy')).toEqual({ text: '旧草稿', attachments: [attachment], selection });
  expect(loadWorkspaceDrafts().get('live')?.text).toBe('刚刚输入');
  expect(localStorage.getItem(draftMigrationKey)).toBe(localStorage.getItem('supercode.drafts.v1'));
  writeWorkspaceDraft('legacy', undefined);
  expect(readWorkspaceDraft('legacy')).toBeUndefined();
  expect(loadWorkspaceDrafts().has('legacy')).toBe(false);
});
it('keeps a newer row even if another window is still migrating an older immutable snapshot', () => {
  localStorage.setItem('supercode.drafts.v1', '[["chat",{"text":"旧","attachments":[]}]]');
  loadWorkspaceDrafts();
  writeWorkspaceDraft('chat', { text: '新', attachments: [] });
  localStorage.setItem(draftMigrationKey, localStorage.getItem('supercode.drafts.v1')!);
  expect(readWorkspaceDraft('chat')?.text).toBe('新');
});
it('bounds loaded rows and row payloads without deleting another window storage', () => {
  for (let index = 0; index < 30; index++) writeWorkspaceDraft(`chat:${index}`, { text: 'x'.repeat(40000), attachments: [] });
  expect(loadWorkspaceDrafts().size).toBe(20);
  expect(readWorkspaceDraft('chat:0')?.text.length).toBe(32000);
  expect(Array.from({ length: localStorage.length }, (_, index) => localStorage.key(index)).filter(key => key?.startsWith(draftPrefix))).toHaveLength(30);
});
