import { afterEach, expect, it, vi } from 'vitest';
import { compatiblePermission, isPermissionMode, loadPermissionChoices } from './permissions';

function stored(values: Record<string, string>) {
  vi.stubGlobal('localStorage', { getItem: (key: string) => values[key] ?? null });
}
afterEach(() => vi.unstubAllGlobals());

it('migrates a manual legacy choice for the current Agent without enabling native automatic review', () => {
  stored({ 'supercode.permission': 'ask' });
  expect(loadPermissionChoices('codex', 'auto')).toEqual({ codex: 'ask' });
  stored({ 'supercode.permission': 'read' });
  expect(loadPermissionChoices('codex', 'ask')).toEqual({ codex: 'read' });
});

it('restores separate native choices and never copies another Agent permission into a new one', () => {
  stored({ 'supercode.permissions.v2': JSON.stringify({ codex: 'auto', claude: 'edit', pi: 'strict' }), 'supercode.permission': 'full' });
  expect(loadPermissionChoices('claude', 'ask')).toEqual({ codex: 'auto', claude: 'edit', pi: 'strict' });
  expect(loadPermissionChoices('opencode', 'ask')).toEqual({ codex: 'auto', claude: 'edit', pi: 'strict', opencode: 'ask' });
});

it('rejects invalid and unsupported native modes and retains readonly settings', () => {
  for (const invalid of ['constructor', '__proto__', 'toString', 'bypass', null, 1]) expect(isPermissionMode(invalid)).toBe(false);
  expect(compatiblePermission('edit', 'codex')).toBe('ask');
  expect(compatiblePermission('auto', 'pi')).toBe('ask');
  expect(compatiblePermission('deny', 'claude')).toBe('deny');
  expect(compatiblePermission('read', 'codex')).toBe('read');
  stored({ 'supercode.permissions.v2': JSON.stringify({ pi: 'auto', codex: 'constructor', injected: 'full' }) });
  expect(loadPermissionChoices('codex', 'ask')).toEqual({ pi: 'ask', codex: 'ask' });
});

it('recovers malformed storage conservatively without restoring a different Agent full-access choice', () => {
  stored({ 'supercode.permissions.v2': '{bad', 'supercode.permission': 'full' });
  expect(loadPermissionChoices('codex', 'ask')).toEqual({ codex: 'ask' });
});
