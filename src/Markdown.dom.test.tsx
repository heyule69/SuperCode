// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import Markdown from './Markdown';

let root: Root;
let container: HTMLDivElement;
const actEnvironment = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean };

beforeEach(() => {
  actEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('retains links and code nodes through stream deltas, completion and fade cleanup', async () => {
  const prefix = '[文件](src/App.tsx) · `src/Markdown.tsx`\n\n```ts\nconst message = "你好";\n```\n\n';
  const open = vi.fn();
  await act(async () => root.render(<Markdown text={prefix + '正在输出。'} streaming openFile={open}/>));
  const link = container.querySelector('.markdown-link')!;
  const inline = container.querySelector('.markdown-file-link')!;
  const pre = container.querySelector('pre')!;
  const code = pre.querySelector('code')!;
  const copy = container.querySelector('.code-block button')!;
  let text = prefix;
  for (let i = 0; i < 24; i++) {
    text += `第 ${i + 1} 段 中文流式文字。`;
    await act(async () => root.render(<Markdown text={text} streaming openFile={() => {}}/>));
    expect(container.querySelector('.markdown-link')).toBe(link);
    expect(container.querySelector('.markdown-file-link')).toBe(inline);
    expect(container.querySelector('pre')).toBe(pre);
    expect(pre.querySelector('code')).toBe(code);
    expect(container.querySelector('.code-block button')).toBe(copy);
  }
  await act(async () => root.render(<Markdown text={text} openFile={open}/>));
  await act(async () => vi.advanceTimersByTime(1200));
  expect(container.querySelector('pre')).toBe(pre);
  expect(container.querySelector('.markdown-link')).toBe(link);
  expect(container.querySelector('.stream-text-fragment')).toBeNull();
  expect(code.textContent).toBe('const message = "你好";\n');
  expect(container.textContent).toContain('第 24 段 中文流式文字。');
});

it('updates file actions without replacing a focused link', async () => {
  const first = vi.fn(), second = vi.fn();
  const text = '[文件](src/App.tsx) · `src/Markdown.tsx`';
  await act(async () => root.render(<Markdown text={text} openFile={first}/>));
  const link = container.querySelector<HTMLAnchorElement>('.markdown-link')!;
  link.focus();
  await act(async () => root.render(<Markdown text={text + ' 新文字'} openFile={second}/>));
  expect(document.activeElement).toBe(link);
  await act(async () => link.click());
  expect(first).not.toHaveBeenCalled();
  expect(second).toHaveBeenCalledWith('src/App.tsx', undefined);
  const inline = container.querySelector<HTMLAnchorElement>('.markdown-file-link')!;
  await act(async () => inline.click());
  expect(second).toHaveBeenCalledWith('src/Markdown.tsx', undefined);
});
