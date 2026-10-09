// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PermissionMenu } from './PermissionMenu';
import { permissionDescription, permissionDetails } from './permissions';
import type { PermissionMode } from './types';

let root: Root, container: HTMLDivElement;
const change = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  change.mockClear();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
async function render(value: PermissionMode = 'ask', busy = false, agent = 'codex') {
  await act(async () => root.render(<PermissionMenu value={value} agent={agent} busy={busy} onChange={change}/>));
}
async function click(selector: string) { await act(async () => container.querySelector<HTMLButtonElement>(selector)!.click()); }
const option = (mode: string) => [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(b => b.querySelector('strong')?.textContent === mode)!;

it('shows the three Codex choices and sends the native automatic reviewer mode only when selected', async () => {
  await render(); await click('[aria-label="权限模式"]');
  expect([...container.querySelectorAll('[role="menuitemradio"] strong')].map(b => b.textContent)).toEqual(['请求批准', '帮我批准', '完全访问权限']);
  expect(option('请求批准').getAttribute('aria-checked')).toBe('true');
  expect(document.activeElement).toBe(option('请求批准'));
  expect(container.querySelector('.permission-option-full .lucide-shield-alert')).not.toBeNull();
  await act(async () => option('帮我批准').click());
  expect(change).toHaveBeenCalledExactlyOnceWith('auto');
  expect(container.querySelector('[role="menu"]')).toBeNull();
  expect(document.activeElement).toBe(container.querySelector('[aria-label="权限模式"]'));
});

it('requires the existing explicit confirmation before full access and supports cancel', async () => {
  await render(); await click('[aria-label="权限模式"]');
  await act(async () => option('完全访问权限').click());
  expect(change).not.toHaveBeenCalled();
  expect(container.querySelector('[role="dialog"]')?.getAttribute('aria-label')).toBe('启用完全访问权限');
  await click('.permission-confirm-actions .quiet-button');
  expect(change).not.toHaveBeenCalled();
  await act(async () => option('完全访问权限').click());
  await click('.permission-confirm-actions .primary-button');
  expect(change).toHaveBeenCalledExactlyOnceWith('full');
  await render('full');
  expect(container.querySelector('.permission-chip')?.textContent).toBe('完全访问');
  expect(container.querySelector('.permission-chip.full-access .lucide-shield-alert')).not.toBeNull();
});

it('cancels a pending escalation when a task starts and disables the trigger', async () => {
  await render(); await click('[aria-label="权限模式"]'); await act(async () => option('完全访问权限').click());
  await render('ask', true);
  expect(container.querySelector('.permission-popover')).toBeNull();
  expect(container.querySelector<HTMLButtonElement>('.permission-chip')!.disabled).toBe(true);
  expect(change).not.toHaveBeenCalled();
  await render(); await click('[aria-label="权限模式"]');
  expect(container.querySelector('[role="menu"]')).not.toBeNull();
});

it('allows the native Claude plan mode and keeps its current selection when opening help', async () => {
  await render('read', false, 'claude'); await click('[aria-label="权限模式"]');
  expect(option('计划模式（只读）').getAttribute('aria-checked')).toBe('true'); expect(change).not.toHaveBeenCalled();
  await click('.permission-help-link');
  expect(container.querySelector('[role="dialog"]')?.getAttribute('aria-label')).toBe('权限说明');
  await click('.permission-back');
  expect(option('计划模式（只读）').getAttribute('aria-checked')).toBe('true');
  await act(async () => option('计划模式（只读）').click());
  expect(change).toHaveBeenCalledExactlyOnceWith('read');
  expect(container.querySelector('.permission-popover')).toBeNull();
  change.mockClear();
  await render('ask', false, 'claude'); await click('[aria-label="权限模式"]');
  await act(async () => option('计划模式（只读）').click());
  expect(change).toHaveBeenCalledExactlyOnceWith('read');
});

it('supports keyboard navigation and closes without changing permissions on Escape or outside click', async () => {
  await render(); await click('[aria-label="权限模式"]');
  await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })));
  expect(document.activeElement).toBe(option('完全访问权限'));
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  expect(container.querySelector('.permission-popover')).toBeNull(); expect(change).not.toHaveBeenCalled();
  await click('[aria-label="权限模式"]');
  await act(async () => document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })));
  expect(container.querySelector('.permission-popover')).toBeNull(); expect(change).not.toHaveBeenCalled();
});

it('describes the actual Agent approval capabilities without promising a shared risk engine', () => {
  expect(permissionDescription('auto', 'codex')).toContain('自动审查');
  expect(permissionDescription('ask', 'claude')).toContain('原生手动审批');
  expect(permissionDescription('ask', 'pi')).toContain('修改和工具');
  expect(permissionDetails('opencode')).toContain('三种规则');
});

it('shows only supported choices for each Agent, including all six native Claude modes', async () => {
  for (const [agent, names] of [
    ['claude', ['计划模式（只读）', '请求批准', '自动接受编辑', '自动模式', '不询问', '完全访问权限']],
    ['opencode', ['只读', '请求批准', '完全访问权限']],
    ['pi', ['只读', '逐项批准', '读取自动批准', '完全访问权限']],
  ] as const) {
    await render('ask', false, agent); await click('[aria-label="权限模式"]');
    expect([...container.querySelectorAll('[role="menuitemradio"] strong')].map(b => b.textContent)).toEqual(names);
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  }
});

it('preserves legacy Codex read-only and manual approvals without silently enabling automatic review', async () => {
  await render('read'); await click('[aria-label="权限模式"]');
  expect(container.querySelector('.permission-legacy-note')?.textContent).toContain('只读');
  expect(option('只读')).toBeUndefined(); expect(change).not.toHaveBeenCalled();
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  await render('strict'); await click('[aria-label="权限模式"]');
  expect(option('请求批准').getAttribute('aria-checked')).toBe('true');
  expect(option('帮我批准').getAttribute('aria-checked')).toBe('false'); expect(change).not.toHaveBeenCalled();
});
