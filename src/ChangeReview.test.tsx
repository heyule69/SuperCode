// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ChangeReview from './ChangeReview';
import DiffView from './DiffView';
import { reviewFiles, reviewHunks } from './changeReviewModel';
import type { Changes } from './types';

const patch = 'diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -7,2 +7,2 @@\n same\n-const old = "before";\n+const next = "after";';
const second = 'diff --git a/b.rs b/b.rs\n@@ -1 +1 @@\n-old\n+new';
let root: Root, host: HTMLDivElement;
const expand = vi.fn(), openFile = vi.fn();
const changes: Changes = { isGit:true, branch:'main', files:[{path:'src/a.ts',status:'M'},{path:'b.rs',status:'M'},{path:'new.md',status:'??'}], diff:patch+'\n'+second };
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  expand.mockClear(); openFile.mockClear();
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
async function render(data=changes) { await act(async () => root.render(<ChangeReview changes={data} loading={false} expanded={false} expand={expand} openFile={openFile}/>)); }
async function click(selector:string) { await act(async () => host.querySelector<HTMLButtonElement>(selector)!.click()); }
it('renders code only for opened files and folds real hunk context independently', async () => {
  await render();
  expect(host.querySelector('.change-code-line')).toBeNull();
  await click('[aria-label="展开 src/a.ts"]');
  expect(expand).toHaveBeenCalledOnce();
  expect(host.querySelectorAll('.change-code-line')).toHaveLength(2);
  expect(host.querySelector('.change-code-line.added .change-line-number:nth-child(2)')?.textContent).toBe('8');
  expect(host.querySelector('.change-code-line.removed .change-line-number')?.textContent).toBe('8');
  expect(host.querySelector('.hljs-keyword')?.textContent).toBe('const');
  await click('.change-context-bar button');
  expect(host.querySelectorAll('.change-code-line')).toHaveLength(3);
  await click('[aria-label="折叠全部文件"]');
  expect(host.querySelector('.change-code-line')).toBeNull();
  await click('[aria-label="展开 new.md"]');
  expect(host.textContent).toContain('新文件，尚未纳入 Git');
});
it('filters the tree, opens the selected diff, toggles wrapping and refreshes an opened patch', async () => {
  await render(); await click('[aria-label="变更文件树"]');
  const input = host.querySelector<HTMLInputElement>('[aria-label="筛选变更文件"]')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'a.ts'); input.dispatchEvent(new Event('input',{bubbles:true})); });
  expect(host.querySelectorAll('.change-tree-file')).toHaveLength(1);
  await click('[aria-label="定位差异 src/a.ts"]');
  expect(host.querySelector('.change-file.is-selected .change-code-line.added code')?.textContent).toBe('const next = "after";');
  await click('[aria-label="代码自动换行"]');
  expect(host.querySelector('.change-review')?.classList.contains('is-wrapped')).toBe(false);
  await render({...changes,diff:patch.replace('"after"','"updated"')+'\n'+second});
  expect(host.querySelector('.change-code-line.added code')?.textContent).toContain('updated');
  await click('[aria-label="查看 src/a.ts 文件"]');
  expect(openFile).toHaveBeenCalledWith('src/a.ts');
});
it('uses the same syntax and diff rows in opened turn previews', async () => {
  await act(async () => root.render(<DiffView text={patch}/>));
  expect(host.querySelector('[aria-label="代码差异"]')).not.toBeNull();
  expect(host.querySelector('.change-code-line.removed .hljs-string')?.textContent).toBe('"before"');
  expect(host.querySelector('.change-code-line.added .hljs-string')?.textContent).toBe('"after"');
});
it('combines staged and working patches and reports binary files without fabricating code lines', () => {
  const combined = reviewFiles({...changes,diff:patch+'\n'+patch});
  expect(combined[0]).toMatchObject({added:2,removed:2});
  expect(reviewHunks(patch).hunks[0]).toMatchObject({context:1,oldStart:7,nextStart:7});
  expect(reviewHunks('diff --git a/icon.png b/icon.png\nBinary files a/icon.png and b/icon.png differ')).toMatchObject({hunks:[],binary:true});
});
