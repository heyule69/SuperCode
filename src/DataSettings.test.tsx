// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ call: vi.fn(), open: vi.fn(), save: vi.fn() }));
vi.mock('./api', () => ({ desktop: true, call: mocks.call }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open, save: mocks.save }));
import DataSettings from './DataSettings';
let root: Root, node: HTMLDivElement;
beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); mocks.call.mockReset(); mocks.open.mockReset(); mocks.save.mockReset();
  mocks.call.mockResolvedValue({ categories: [], pendingRestore: false });
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
const button = (text: string) => [...node.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent === text)!;
it('requires an explicit selection and confirmation before staging a restore', async () => {
  await act(async () => root.render(<DataSettings busy={false}/>));
  mocks.open.mockResolvedValue('D:/Backups/chat.supercode-backup');
  await act(async () => button('选择备份').click());
  expect(node.textContent).toContain('chat.supercode-backup');
  expect(mocks.call).not.toHaveBeenCalledWith('prepare_data_restore', expect.anything());
  await act(async () => button('准备恢复').click());
  expect(mocks.call).toHaveBeenCalledWith('prepare_data_restore', { path: 'D:/Backups/chat.supercode-backup' });
  expect(mocks.call).not.toHaveBeenCalledWith('request_app_exit');
});
it('allows a consistent backup during work but blocks destructive maintenance', async () => {
  localStorage.setItem('supercode.draft.v2:chat', 'draft'); localStorage.setItem('other-app', 'private');
  mocks.save.mockResolvedValue('D:/Backups/new.supercode-backup');
  await act(async () => root.render(<DataSettings busy={true}/>));
  expect(button('选择备份').disabled).toBe(true); expect(button('清理旧安装').disabled).toBe(true);
  await act(async () => button('保存备份').click());
  expect(mocks.call).toHaveBeenCalledWith('export_data_backup', { path: 'D:/Backups/new.supercode-backup', entries: { 'supercode.draft.v2:chat': 'draft' } });
});
