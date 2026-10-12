// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AddMenu } from './ComposerAddMenu';
import { call } from './api';
import { loadLocalSkills, type Skill } from './skills';
import { open as pickFiles } from '@tauri-apps/plugin-dialog';
import type { ExtensionCatalog, ExtensionPlugin } from './extensions';
import type { PermissionMode } from './types';

vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
vi.mock('./skills', async importOriginal => ({ ...await importOriginal<typeof import('./skills')>(), loadLocalSkills: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => ['C:/docs/参考.png', 'C:/docs/报告.pdf']) }));
let root: Root, node: HTMLDivElement;
const onSkill = vi.fn(() => true), onPlan = vi.fn(), onGoal = vi.fn(), addAttachments = vi.fn(async () => {}), project = vi.fn();
const local: Skill = { name: 'frontend-design', description: '设计界面', path: 'C:/skills/design/SKILL.md' };
const docSkill: Skill = { name: 'documents', description: '创建和编辑文档', namespace: 'Documents', path: 'C:/plugins/documents/skills/documents/SKILL.md' };
const plugin: ExtensionPlugin = { id: 'codex:docs', nativeId: 'docs', name: 'Documents', agent: 'codex', path: 'C:/plugins/documents', description: 'Create and edit documents', version: '1', installed: true, enabled: true, source: '本机', components: ['技能'], icon: null, limitation: null };
let extensions: ExtensionCatalog;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks(); extensions = { plugins: [plugin], mcp: [], warnings: [] };
  vi.mocked(call).mockImplementation(async () => extensions);
  vi.mocked(loadLocalSkills).mockResolvedValue({ skills: [local, docSkill] });
  onSkill.mockReturnValue(true);
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(options: { agent?: string; permissionMode?: PermissionMode; modeBusy?: boolean; busy?: boolean; projectId?: string; attachments?: { kind: 'skill'; name: string; path: string }[] } = {}) {
  await act(async () => root.render(<AddMenu projectId="project" agent="codex" permissionMode="ask" busy={false} modeBusy={false} attachments={[]}
    onSkill={onSkill} onPlan={onPlan} onGoal={onGoal} addAttachments={addAttachments} project={project} {...options}/>));
}
const button = (name: string) => [...node.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent?.trim() === name || b.querySelector('strong')?.textContent === name || b.querySelector(':scope > span')?.textContent === name)!;
async function click(target: HTMLElement) { await act(async () => target.click()); }
async function open() { await click(node.querySelector('[aria-label="添加附件"]')!); }
async function type(selector: string, text: string) {
  const input = node.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
  await act(async () => {
    const prototype = input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(input, text);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

it('reads metadata on demand and selects an expanded inline skill without navigating away', async () => {
  await render(); expect(call).not.toHaveBeenCalled(); expect(loadLocalSkills).not.toHaveBeenCalled();
  await open(); expect(loadLocalSkills).toHaveBeenCalledWith('project', false);
  expect(call).toHaveBeenCalledExactlyOnceWith('list_extensions', { projectId: 'project', browse: false });
  expect(node.querySelector('[aria-label="技能"]')).not.toBeNull(); expect(node.textContent).toContain('frontend-design');
  expect(node.querySelector('[aria-label="Documents 的技能"]')).toBeNull();
  await click(button('frontend-design'));
  expect(onSkill).toHaveBeenCalledExactlyOnceWith(local); expect(node.querySelector('.add-popover')).toBeNull(); expect(project).not.toHaveBeenCalled();
});
it('expands a plugin to its own skills, preserves selection indicators and leaves a rejected selection open', async () => {
  await render({ attachments: [{ kind: 'skill', name: docSkill.name, path: docSkill.path }] }); await open(); await click(button('Documents'));
  const row = button('documents'); expect(row.getAttribute('aria-checked')).toBe('true');
  onSkill.mockReturnValue(false); await click(row);
  expect(onSkill).toHaveBeenCalledWith(docSkill); expect(node.querySelector('.add-popover')).not.toBeNull();
  onSkill.mockReturnValue(true); await click(row); expect(node.querySelector('.add-popover')).toBeNull();
});
it('searches and automatically expands matching plugin skills, with no implicit message submission', async () => {
  await render(); await open(); await type('[aria-label="搜索插件与技能"]', 'documents');
  expect(node.querySelector('[aria-label="Documents 的技能"]')?.textContent).toContain('documents');
  expect(node.querySelector('[aria-label="技能"]')?.textContent).not.toContain('frontend-design');
  const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  await act(async () => node.querySelector('[aria-label="搜索插件与技能"]')!.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true); expect(onSkill).not.toHaveBeenCalled(); expect(onGoal).not.toHaveBeenCalled();
});
it('does not offer another Agent plugin or invoke a disabled plugin skill', async () => {
  extensions.plugins.push({ ...plugin, id: 'claude:pdf', name: 'Claude PDF', agent: 'claude', path: 'C:/plugins/claude' });
  extensions.plugins[0] = { ...plugin, enabled: false };
  await render(); await open(); expect(node.textContent).not.toContain('Claude PDF'); await click(button('Documents'));
  expect(button('documents').disabled).toBe(true); await click(button('documents')); expect(onSkill).not.toHaveBeenCalled();
  expect(node.querySelector('[aria-label="技能"]')?.textContent).not.toContain('documents');
});
it('selects the current Agent plugin copy when identical skills were deduplicated across engines', async () => {
  const claudePath = 'C:/plugins/claude-documents/skills/documents/SKILL.md';
  extensions.plugins.push({ ...plugin, id: 'claude:docs', agent: 'claude', path: 'C:/plugins/claude-documents' });
  const copies = { ...docSkill, paths: [docSkill.path, claudePath] };
  vi.mocked(loadLocalSkills).mockResolvedValue({ skills: [copies] });
  await render({ agent: 'claude' }); await open(); await click(button('Documents')); await click(button('documents'));
  expect(onSkill).toHaveBeenCalledExactlyOnceWith({ ...copies, path: claudePath });
});
it('enables and exits planning only on explicit clicks, and prevents mode changes during another task', async () => {
  await render(); await open(); expect(onPlan).not.toHaveBeenCalled(); await click(button('计划模式'));
  expect(onPlan).toHaveBeenLastCalledWith(true); expect(onGoal).not.toHaveBeenCalled();
  await render({ permissionMode: 'read' }); await open(); expect(button('计划模式').getAttribute('aria-checked')).toBe('true'); await click(button('计划模式'));
  expect(onPlan).toHaveBeenLastCalledWith(false);
  await render({ modeBusy: true }); await open(); expect(button('计划模式').disabled).toBe(true); expect(button('目标').disabled).toBe(true);
});
it('does not pretend unsupported Agents have native plan and persistent goal modes', async () => {
  await render({ agent: 'opencode' }); await open(); expect(button('计划模式').disabled).toBe(true); expect(button('目标').disabled).toBe(true);
  await render({ agent: 'claude' }); expect(button('计划模式').disabled).toBe(false); expect(button('目标').disabled).toBe(true);
});
it('requires an explicit nonempty goal before starting, and supports returning without running', async () => {
  await render(); await open(); await click(button('目标'));
  expect(document.activeElement).toBe(node.querySelector('textarea')); expect(button('设置并开始').disabled).toBe(true); expect(onGoal).not.toHaveBeenCalled();
  await type('textarea', '  完成登录功能\n并测试  '); expect(onGoal).not.toHaveBeenCalled();
  await click(button('返回')); expect(node.querySelector('[aria-label="添加能力"]')).not.toBeNull(); expect(onGoal).not.toHaveBeenCalled();
  await click(button('目标')); await click(button('设置并开始'));
  expect(onGoal).toHaveBeenCalledExactlyOnceWith('完成登录功能\n并测试'); expect(node.querySelector('.add-popover')).toBeNull();
});
it('ignores an old project metadata response after changing workspace, and retries read failures', async () => {
  let finish!: (value: { skills: Skill[] }) => void;
  vi.mocked(loadLocalSkills).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await render(); await open(); await render({ projectId: 'new-project' });
  expect(node.textContent).toContain('frontend-design');
  await act(async () => finish({ skills: [{ ...local, name: 'stale-project' }] })); expect(node.textContent).not.toContain('stale-project');
  vi.mocked(loadLocalSkills).mockRejectedValueOnce(new Error('无法读取技能目录'));
  await click(node.querySelector('[aria-label="刷新插件与技能"]')!); expect(node.querySelector('[role="alert"]')?.textContent).toContain('无法读取技能目录');
  await click(node.querySelector('[aria-label="刷新插件与技能"]')!); expect(node.querySelector('[role="alert"]')).toBeNull(); expect(node.textContent).toContain('frontend-design');
});
it('handles keyboard traversal, Escape and outside clicks without starting a goal', async () => {
  await render(); await open(); const first = node.querySelector<HTMLElement>('[role="menuitem"]')!; expect(document.activeElement).toBe(first);
  await act(async () => first.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true })));
  expect(document.activeElement?.textContent).toContain('目标');
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  expect(node.querySelector('.add-popover')).toBeNull(); expect(document.activeElement).toBe(node.querySelector('[aria-label="添加附件"]'));
  await open(); await act(async () => document.body.dispatchEvent(new Event('pointerdown', { bubbles: true })));
  expect(node.querySelector('.add-popover')).toBeNull(); expect(onGoal).not.toHaveBeenCalled();
});
it('preserves local file picking and image detection from the file entry', async () => {
  await render(); await open(); await click(button('文件和文件夹')); await click(button('文件'));
  expect(addAttachments).toHaveBeenCalledExactlyOnceWith([{ kind: 'image', path: 'C:/docs/参考.png', name: '参考.png' }, { kind: 'file', path: 'C:/docs/报告.pdf', name: '报告.pdf' }]);
  expect(node.querySelector('.add-popover')).toBeNull();
});
it('opens attachment choices with keyboard focus and returns without opening a picker', async () => {
  await render(); await open(); await click(button('文件和文件夹'));
  expect(node.querySelector('[aria-label="添加文件和文件夹"]')).not.toBeNull();
  expect(document.activeElement).toBe(button('文件'));
  await act(async () => button('文件').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true })));
  expect(document.activeElement).toBe(button('图片'));
  await click(button('返回')); await act(async () => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));
  expect(document.activeElement).toBe(button('文件和文件夹'));
  expect(pickFiles).not.toHaveBeenCalled(); expect(addAttachments).not.toHaveBeenCalled();
});
it.each(['图片', '文件夹'])('keeps the %s picker available from the compact entry', async name => {
  await render(); await open(); await click(button('文件和文件夹')); await click(button(name));
  expect(pickFiles).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
    directory: name === '文件夹', multiple: name !== '文件夹',
    ...(name === '图片' ? { filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }] } : {}),
  }));
  expect(addAttachments).toHaveBeenCalledOnce(); expect(node.querySelector('.add-popover')).toBeNull();
});
