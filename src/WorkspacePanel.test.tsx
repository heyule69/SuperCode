// @vitest-environment happy-dom
import { act, type ComponentProps } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import WorkspacePanel from './WorkspacePanel';
import { call } from './api';

vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
let root: Root, host: HTMLDivElement, props: ComponentProps<typeof WorkspacePanel>;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear(); vi.mocked(call).mockReset();
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  props = { active: true, projectId: 'project', sessionId: 'chat', workspacePath: 'D:/workspace', workspaceName: '示例项目', changes: {isGit:true,branch:'main',files:[],diff:''}, loading:false, preview:null, expanded:false, refresh:vi.fn(), close:vi.fn(), expand:vi.fn(), closePreview:vi.fn(), openFile:vi.fn(), reviewFile:vi.fn(), showDiff:vi.fn(), reveal:vi.fn() };
  vi.mocked(call).mockImplementation(async (_, args = {}) => args.path === '.' ? { entries:[{name:'报告.md',path:'报告.md',isDirectory:false},{name:'src',path:'src',isDirectory:true}],truncated:false } : { entries:[{name:'main.ts',path:'src/main.ts',isDirectory:false}],truncated:false });
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
async function render(next = props) { props = next; await act(async () => { root.render(<WorkspacePanel {...next}/>); }); }
async function click(selector: string) { await act(async () => host.querySelector<HTMLButtonElement>(selector)!.click()); }
async function view(name: string) {
  await click('[aria-label="切换工作空间视图"]');
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(item => item.textContent === name)!.click());
}
it('switches overview, files and changes without confusing opening a source with reviewing its diff', async () => {
  props.changes.files = [{path:'src/main.ts',status:'M'}];
  props.changes.diff = 'diff --git a/src/main.ts b/src/main.ts\n@@ -1 +1 @@\n-old\n+new';
  await render();
  await click('.workspace-file-row');
  expect(props.openFile).toHaveBeenCalledWith('src/main.ts');
  await view('变更');
  await act(async () => { await vi.dynamicImportSettled(); });
  expect(host.querySelector('.change-code-line')).toBeNull();
  await click('.change-file-toggle');
  expect(host.querySelector('.change-code-line.added code')?.textContent).toBe('new');
  expect(props.expand).toHaveBeenCalledOnce();
  expect(props.reviewFile).not.toHaveBeenCalled();
  await click('.change-source-button');
  expect(props.openFile).toHaveBeenCalledTimes(2);
  await view('工作空间文件');
  expect(localStorage.getItem('supercode.workspace-view')).toBe('files');
  expect(host.querySelector('[aria-label="筛选工作空间文件"]')).not.toBeNull();
});
it('reads only expanded folders, remembers expansion across refresh, and pauses loading while hidden', async () => {
  localStorage.setItem('supercode.workspace-view','files');
  await render({ ...props, active:false });
  expect(call).not.toHaveBeenCalled();
  await render({ ...props, active:true });
  expect(call).toHaveBeenCalledWith('list_workspace_directory', {projectId:'project',sessionId:'chat',path:'.'});
  expect(vi.mocked(call).mock.calls.some(([,args])=>args?.path==='src')).toBe(false);
  await click('.workspace-tree-row[title="src"]');
  expect(host.querySelector('[title="src"] .file-kind-icon')?.getAttribute('data-icon')).toBe('folder-src-open');
  expect(host.querySelector('[title="src/main.ts"]')).not.toBeNull();
  expect(host.querySelector('[title="src/main.ts"] .file-kind-icon')?.getAttribute('data-icon')).toBe('typescript');
  await click('[aria-label="刷新工作空间"]');
  expect(host.querySelector('[title="src"]')?.getAttribute('aria-expanded')).toBe('true');
  expect(props.refresh).toHaveBeenCalledOnce();
  await click('.workspace-tree-row[title="src/main.ts"]');
  expect(props.openFile).toHaveBeenCalledWith('src/main.ts');
});
it('keeps overview compact and makes the full change list accessible', async () => {
  props.changes.files = Array.from({length:9}, (_, index) => ({path:`src/item${index}.tsx`,status:'M'}));
  await render();
  expect(host.querySelector('.workspace-location-branch')?.textContent).toBe('main');
  expect(host.querySelectorAll('.workspace-file-row')).toHaveLength(6);
  expect(host.querySelector('.workspace-file-row .file-kind-icon')?.getAttribute('data-icon')).toBe('react_ts');
  expect(host.querySelector('.workspace-overview-actions')?.textContent).toContain('查看变更9');
  await click('.workspace-more-changes');
  await act(async () => { await vi.dynamicImportSettled(); });
  expect(host.querySelectorAll('.change-file')).toHaveLength(9);
});
it('discards an old directory response when switching tasks and loads the new task independently', async () => {
  localStorage.setItem('supercode.workspace-view','files');
  let finish: (result: unknown) => void = () => {};
  vi.mocked(call).mockImplementation(async (_, args = {}) => args.sessionId === 'chat' ? new Promise(resolve => { finish = resolve; }) : { entries:[{name:'new.txt',path:'new.txt',isDirectory:false}],truncated:false });
  await render();
  await render({...props,projectId:'',sessionId:'new-chat',workspacePath:'D:/new-chat'});
  await act(async () => finish({ entries:[{name:'old.txt',path:'old.txt',isDirectory:false}],truncated:false }));
  expect(host.textContent).toContain('new.txt');
  expect(host.textContent).not.toContain('old.txt');
  expect(call).toHaveBeenCalledWith('list_workspace_directory',{projectId:'',sessionId:'new-chat',path:'.'});
});
it('shows a directory error with retry instead of presenting an empty success', async () => {
  vi.mocked(call).mockRejectedValueOnce(new Error('目录不可读'));
  // Use files so there is only one initial directory request.
  localStorage.setItem('supercode.workspace-view','files');
  await render();
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  expect(host.textContent).toContain('目录不可读');
  await click('.workspace-directory-error button');
  expect(host.textContent).toContain('报告.md');
});
it('shows normal Windows drive and network paths while keeping the original workspace path for file access', async () => {
  const drive = 'D:\\项目\\SuperCode';
  await render({ ...props, workspacePath: '\\\\?\\' + drive });
  expect(host.querySelector('.workspace-location')?.getAttribute('title')).toBe(drive);
  expect(host.querySelector('.workspace-location small')?.textContent).toBe(drive);
  await render({ ...props, workspacePath: '\\\\?\\UNC\\server\\共享\\项目' });
  expect(host.querySelector('.workspace-location small')?.textContent).toBe('\\\\server\\共享\\项目');
  await render({ ...props, workspacePath: '/home/user/project' });
  expect(host.querySelector('.workspace-location small')?.textContent).toBe('/home/user/project');
});
