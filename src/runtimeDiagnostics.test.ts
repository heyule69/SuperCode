import { beforeEach, expect, it, vi } from 'vitest';
import { exportRuntimeDiagnostics, openRuntimeLogFolder } from './runtimeDiagnostics';
import { call } from './api';
import { save } from '@tauri-apps/plugin-dialog';
vi.mock('./api', () => ({ call: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }));
beforeEach(() => { vi.mocked(save).mockReset(); vi.mocked(call).mockReset(); });
it('cancels without writing a file', async () => {
  vi.mocked(save).mockResolvedValue(null);
  expect(await exportRuntimeDiagnostics()).toBeNull(); expect(call).not.toHaveBeenCalled();
});
it('exports backend lifecycle records without passing raw stderr or conversation contents', async () => {
  vi.mocked(save).mockResolvedValue('D:/diagnostics.jsonl');
  vi.mocked(call).mockResolvedValue('D:/diagnostics.jsonl');
  expect(await exportRuntimeDiagnostics()).toBe('D:/diagnostics.jsonl');
  expect(call).toHaveBeenCalledExactlyOnceWith('save_runtime_diagnostics', { path: 'D:/diagnostics.jsonl' });
});
it('opens the native log directory without relying on a device-specific path', async () => {
  await openRuntimeLogFolder(); expect(call).toHaveBeenCalledExactlyOnceWith('open_runtime_log_folder');
});
