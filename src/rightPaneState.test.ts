import { describe, expect, it } from 'vitest';
import { closeRightTab, documentTab, emptyRightPane, openRightTab, workspaceTab } from './rightPaneState';
describe('right-side content tabs', () => {
  it('deduplicates a document and updates its contents without losing its other tabs', () => {
    const plan = documentTab({ name: '计划', text: '初稿', plan: true }, 'chat');
    let state = openRightTab(emptyRightPane, workspaceTab);
    state = openRightTab(state, plan);
    state = openRightTab(state, { ...plan, preview: { ...plan.preview!, text: '更新后的计划' } });
    expect(state.tabs).toHaveLength(2);
    expect(state.tabs[1].preview?.text).toBe('更新后的计划');
    expect(state.active).toBe(plan.id);
  });
  it('keeps source, operation diff and a similarly named document from another chat separate', () => {
    const tabs = [documentTab({ name: 'src/文件.ts', text: 'source' }, 'a'), documentTab({ name: 'src/文件.ts', text: 'diff', diff: true }, 'a'), documentTab({ name: 'src/文件.ts', text: 'source' }, 'b')];
    expect(new Set(tabs.map(tab => tab.id)).size).toBe(3);
  });
  it('returns to the adjacent tab, preserves selection when closing a background tab, and leaves no empty selection', () => {
    let state = openRightTab(emptyRightPane, workspaceTab);
    state = openRightTab(state, { id: 'browser', kind: 'browser', title: '浏览器' });
    state = openRightTab(state, { id: 'question', kind: 'question', title: '提问' });
    state = closeRightTab(state, 'workspace');
    expect(state.active).toBe('question');
    state = closeRightTab(state, 'question');
    expect(state.active).toBe('browser');
    state = closeRightTab(state, 'browser');
    expect(state).toEqual(emptyRightPane);
    expect(closeRightTab(state, 'missing')).toBe(state);
  });
});
