import { afterEach, expect, it, vi } from 'vitest';
import { compatiblePermission, isPermissionMode, loadPermissionChoices, loadExecutionPermissionChoices, permissionDescription, permissionDetails, permissionOrder, planExecutionPermission } from './permissions';

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

it('retains execution permissions when planning is saved and keeps each Agent separate', () => {
  stored({ 'supercode.execution-permissions.v1': JSON.stringify({ claude: 'full', codex: 'auto', pi: 'read', injected: 'full' }) });
  expect(loadExecutionPermissionChoices({ claude: 'read', codex: 'read', opencode: 'ask' })).toEqual({ claude: 'full', codex: 'auto', opencode: 'ask' });
  expect(loadExecutionPermissionChoices({ claude: 'edit' }).claude).toBe('edit');
  stored({ 'supercode.execution-permissions.v1': '{broken' });
  expect(loadExecutionPermissionChoices({ claude: 'read' })).toEqual({});
});

it('accepts ordinary Claude plan edits without granting full access or replacing explicit policies', () => {
  stored({});
  for (const mode of ['ask', 'read', undefined]) expect(planExecutionPermission(mode, 'claude')).toBe('edit');
  for (const mode of ['strict', 'edit', 'auto', 'deny', 'full'] as const) expect(planExecutionPermission(mode, 'claude')).toBe(mode);
  expect(planExecutionPermission('read', 'codex')).toBe('ask');
  expect(planExecutionPermission('full', 'codex')).toBe('full');
});

it('stores Gemini permissions separately and only offers its native read, ask and full modes', () => {
  stored({ 'supercode.permissions.v2': JSON.stringify({ claude: 'full', gemini: 'read' }), 'supercode.execution-permissions.v1': JSON.stringify({ claude: 'full', gemini: 'ask' }) });
  expect(loadPermissionChoices('gemini', 'full')).toEqual({ claude: 'full', gemini: 'read' });
  expect(loadExecutionPermissionChoices({ gemini: 'read' })).toEqual({ claude: 'full', gemini: 'ask' });
  expect(permissionOrder('gemini')).toEqual(['read', 'ask', 'full']);
  for (const mode of ['auto', 'edit', 'deny']) expect(compatiblePermission(mode, 'gemini')).toBe('ask');
  expect(permissionDescription('ask', 'gemini')).toContain('Gemini CLI');
  expect(permissionDescription('read', 'gemini')).toContain('不修改');
  expect(permissionDetails('gemini')).not.toContain('Claude');
  stored({ 'supercode.permissions.v2': JSON.stringify({ claude: 'full' }) });
  expect(loadPermissionChoices('gemini', 'ask')).toEqual({ claude: 'full', gemini: 'ask' });
});
