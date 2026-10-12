import { useCallback, useState } from 'react';
import type { FilePreview } from './WorkspacePanel';

export type RightTabKind = 'workspace' | 'browser' | 'question' | 'subagents' | 'document';
export interface RightTab { id: string; kind: RightTabKind; title: string; preview?: FilePreview }
export interface RightPaneState { tabs: RightTab[]; active: string | null }
export const emptyRightPane: RightPaneState = { tabs: [], active: null };
export const workspaceTab: RightTab = { id: 'workspace', kind: 'workspace', title: '工作空间' };

export function openRightTab(state: RightPaneState, tab: RightTab): RightPaneState {
  const existing = state.tabs.findIndex(item => item.id === tab.id);
  const tabs = existing < 0 ? [...state.tabs, tab] : state.tabs.map(item => item.id === tab.id ? tab : item);
  return { tabs, active: tab.id };
}
export function closeRightTab(state: RightPaneState, id: string): RightPaneState {
  const index = state.tabs.findIndex(tab => tab.id === id);
  if (index < 0) return state;
  const tabs = state.tabs.filter(tab => tab.id !== id);
  return { tabs, active: state.active === id ? tabs[Math.max(0, index - 1)]?.id ?? null : state.active };
}
export function documentTab(preview: FilePreview, owner = ''): RightTab {
  const type = preview.plan ? 'plan' : preview.diff ? 'diff' : 'file';
  return { id: JSON.stringify([type, owner, preview.name]), kind: 'document', title: preview.name.split(/[\\/]/).pop() || '文档', preview };
}

/** Only the lightweight workspace follows navigation; content belongs to its chat. */
export function useRightPane(owner: string) {
  const [stored, setStored] = useState(() => ({ owner, state: localStorage.getItem('supercode.context') === 'true' ? openRightTab(emptyRightPane, workspaceTab) : emptyRightPane }));
  let state = stored.state;
  if (stored.owner !== owner) {
    state = stored.state.tabs.some(tab => tab.kind === 'workspace') ? openRightTab(emptyRightPane, workspaceTab) : emptyRightPane;
    setStored({ owner, state });
  }
  const open = useCallback((tab: RightTab) => setStored(old => old.owner === owner ? { owner, state: openRightTab(old.state, tab) } : old), [owner]);
  const close = useCallback((id: string) => setStored(old => old.owner === owner ? { owner, state: closeRightTab(old.state, id) } : old), [owner]);
  const select = useCallback((id: string) => setStored(old => old.owner === owner && old.state.tabs.some(tab => tab.id === id) ? { owner, state: { ...old.state, active: id } } : old), [owner]);
  const update = useCallback((tab: RightTab) => setStored(old => old.owner === owner && old.state.tabs.some(item => item.id === tab.id) ? { owner, state: { ...old.state, tabs: old.state.tabs.map(item => item.id === tab.id ? tab : item) } } : old), [owner]);
  const closeAll = useCallback(() => setStored(old => old.owner === owner ? { owner, state: emptyRightPane } : old), [owner]);
  return { ...state, open, close, select, update, closeAll };
}
