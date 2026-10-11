// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { call } from './api';
import { installFrontendDiagnostics, reportFrontendError } from './frontendDiagnostics';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
let dispose: (() => void) | undefined;
beforeEach(() => { vi.mocked(invoke).mockReset().mockResolvedValue(undefined); vi.mocked(isTauri).mockReturnValue(true); });
afterEach(() => { dispose?.(); dispose = undefined; });
it('records global JavaScript and promise errors with stacks', () => {
  dispose = installFrontendDiagnostics();
  window.dispatchEvent(new ErrorEvent('error', { error: new Error('render failed') }));
  const event = new Event('unhandledrejection'); Object.defineProperty(event, 'reason', { value: new Error('request failed') });
  window.dispatchEvent(event);
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(invoke).toHaveBeenNthCalledWith(1, 'report_frontend_error', expect.objectContaining({ source: 'javascript', message: 'render failed', stack: expect.stringContaining('render failed') }));
  expect(invoke).toHaveBeenNthCalledWith(2, 'report_frontend_error', expect.objectContaining({ source: 'promise', message: 'request failed' }));
  dispose(); dispose = undefined; window.dispatchEvent(new ErrorEvent('error', { message: 'after cleanup' }));
  expect(invoke).toHaveBeenCalledTimes(2);
});
it('records IPC failures without copying sensitive command arguments and preserves the rejection', async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error('connection lost'));
  await expect(call('send_turn', { input: 'private conversation', apiKey: 'private-key' })).rejects.toThrow('connection lost');
  expect(invoke).toHaveBeenNthCalledWith(2, 'report_frontend_error', expect.objectContaining({ source: 'ipc', command: 'send_turn', message: 'connection lost' }));
  expect(JSON.stringify(vi.mocked(invoke).mock.calls[1])).not.toContain('private');
});
it('does not recurse when logging fails or send reports in a browser preview', async () => {
  vi.mocked(invoke).mockRejectedValue(new Error('disk full'));
  await expect(call('report_frontend_error')).rejects.toThrow('disk full');
  expect(invoke).toHaveBeenCalledTimes(1);
  vi.mocked(isTauri).mockReturnValue(false); reportFrontendError('react', new Error('preview'));
  expect(invoke).toHaveBeenCalledTimes(1);
});
