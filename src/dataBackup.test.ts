// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock('./api', () => ({ desktop: true, call: mocks.call }));
import { backupEntries, replaceBackupEntries, restoreFrontendSettings } from './dataBackup';
beforeEach(() => { localStorage.clear(); mocks.call.mockReset(); });
it('restores drafts before acknowledging and preserves unrelated app keys', async () => {
  localStorage.setItem('other-app', 'keep'); localStorage.setItem('supercode.draft.v2:old', 'old');
  mocks.call.mockResolvedValueOnce({ token: 'restore', entries: { 'supercode.draft.v2:new': 'restored draft' } }).mockImplementationOnce(async () => {
    expect(localStorage.getItem('supercode.draft.v2:new')).toBe('restored draft');
  });
  await restoreFrontendSettings();
  expect(localStorage.getItem('other-app')).toBe('keep'); expect(localStorage.getItem('supercode.draft.v2:old')).toBeNull();
  expect(mocks.call).toHaveBeenLastCalledWith('acknowledge_frontend_restore', { token: 'restore' });
});
it('does not acknowledge failed storage restore and rolls back original preferences', () => {
  localStorage.setItem('supercode.preferences', 'original');
  const original = localStorage.setItem.bind(localStorage);
  const storage: Storage = { get length() { return localStorage.length; }, key: index => localStorage.key(index), getItem: key => localStorage.getItem(key), removeItem: key => localStorage.removeItem(key), clear: () => localStorage.clear(), setItem: (key, value) => { if (value === 'oversize') throw new Error('QuotaExceededError'); original(key, value); } };
  expect(() => replaceBackupEntries(storage, { 'supercode.preferences': 'oversize' })).toThrow();
  expect(backupEntries(localStorage)).toEqual({ 'supercode.preferences': 'original' });
  expect(() => replaceBackupEntries(storage, { 'other-app': 'bad' })).toThrow();
});
