// @vitest-environment happy-dom
import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useDraftPersistence, useWorkspaceDrafts } from './useWorkspaceDrafts';
import { readWorkspaceDraft, writeWorkspaceDraft } from './workspaceDrafts';

let roots: Root[] = [], nodes: HTMLElement[] = [];
const windows = new Map<string, ReturnType<typeof useWorkspaceDrafts>>();
function WindowDraft({ name, id, text }: { name: string; id: string; text: string }) {
  const store = useWorkspaceDrafts(); windows.set(name, store); store.draftKey.current = id;
  const input = useRef(text); input.current = text;
  const attachments = useRef([]);
  useDraftPersistence(store, { ready: true, key: id, text, attachments: attachments.current }, { text: input, attachments });
  return null;
}
async function mount(name: string, id: string, text: string) {
  const node = document.createElement('div'); document.body.append(node); nodes.push(node);
  const root = createRoot(node); roots.push(root);
  await act(async () => root.render(<WindowDraft name={name} id={id} text={text}/>)); return root;
}
beforeEach(() => { (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; localStorage.clear(); windows.clear(); vi.useFakeTimers(); });
afterEach(async () => { for (const root of roots) await act(async () => root.unmount()); roots = []; nodes.forEach(node => node.remove()); nodes = []; vi.useRealTimers(); vi.unstubAllGlobals(); });
it('flushes only each window active draft on close, including a draft newer than the debounce', async () => {
  const first = await mount('first', 'A', '草稿 A');
  await mount('second', 'B', '草稿 B');
  await act(async () => vi.advanceTimersByTime(260));
  await act(async () => first.render(<WindowDraft name="first" id="A" text="关闭前刚输入"/>));
  await act(async () => first.unmount()); roots = roots.filter(root => root !== first);
  expect(readWorkspaceDraft('A')?.text).toBe('关闭前刚输入');
  expect(readWorkspaceDraft('B')?.text).toBe('草稿 B');
  window.dispatchEvent(new Event('beforeunload'));
  expect(readWorkspaceDraft('A')?.text).toBe('关闭前刚输入');
});
it('reads another window latest inactive draft while preserving the current editor', async () => {
  await mount('first', 'A', '未保存的编辑 A');
  writeWorkspaceDraft('B', { text: '其他窗口输入 B', attachments: [] });
  window.dispatchEvent(new StorageEvent('storage', { key: 'supercode.draft.v2:B' }));
  expect(windows.get('first')!.drafts.current.get('B')).toBe('其他窗口输入 B');
  writeWorkspaceDraft('A', { text: '其他窗口旧值', attachments: [] });
  window.dispatchEvent(new StorageEvent('storage', { key: 'supercode.draft.v2:A' }));
  window.dispatchEvent(new Event('beforeunload'));
  expect(readWorkspaceDraft('A')?.text).toBe('未保存的编辑 A');
});
it('retains local drafts if persistence fails and the user navigates back', async () => {
  await mount('first', 'A', '');
  const original = localStorage;
  vi.stubGlobal('localStorage', { get length(){return original.length;},key:(index:number)=>original.key(index),getItem:(key:string)=>original.getItem(key),removeItem:(key:string)=>original.removeItem(key),clear:()=>original.clear(),setItem:()=>{throw new Error('full');} } satisfies Storage);
  await act(async () => windows.get('first')!.save('B', '磁盘满时的草稿', []));
  expect(windows.get('first')!.persistenceError).toContain('草稿暂未保存到磁盘');
  expect(windows.get('first')!.read('B')?.text).toBe('磁盘满时的草稿');
  vi.unstubAllGlobals();
  await act(async () => windows.get('first')!.retryPersistence());
  expect(windows.get('first')!.persistenceError).toBe('');
  expect(readWorkspaceDraft('B')?.text).toBe('磁盘满时的草稿');
});
it('retains every unsaved row outside the normal cache limit and clears the warning only after all rows save', async () => {
  await mount('first', 'A', '');
  const original = localStorage;
  vi.stubGlobal('localStorage', { get length(){return original.length;},key:(index:number)=>original.key(index),getItem:(key:string)=>original.getItem(key),removeItem:(key:string)=>original.removeItem(key),clear:()=>original.clear(),setItem:()=>{throw new Error('quota');} } satisfies Storage);
  await act(async () => { for (let index=0;index<25;index++) windows.get('first')!.save(`failed:${index}`,`未保存 ${index}`,[]); });
  expect(windows.get('first')!.drafts.current.get('failed:0')).toBe('未保存 0');
  expect(windows.get('first')!.read('failed:24')?.text).toBe('未保存 24');
  vi.unstubAllGlobals();
  await act(async () => windows.get('first')!.save('different','另一个成功保存的草稿',[]));
  expect(windows.get('first')!.persistenceError).toContain('草稿暂未保存到磁盘');
  await act(async () => windows.get('first')!.retryPersistence());
  expect(windows.get('first')!.persistenceError).toBe('');
  expect(readWorkspaceDraft('failed:0')?.text).toBe('未保存 0');
  expect(readWorkspaceDraft('failed:24')?.text).toBe('未保存 24');
});
