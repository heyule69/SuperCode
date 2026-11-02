// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ReplyText, ReplyTranslationContext } from './ReplyText';
import { MediaContext } from './Media';
import { call } from './api';

vi.mock('./api', () => ({ desktop:true, call:vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc:(token:string) => token }));
let root: Root, node: HTMLDivElement, index = 0, session: string;
beforeEach(async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal('IntersectionObserver', undefined);
  session = `translate-test-${index++}`;
  vi.mocked(call).mockReset().mockImplementation(async (_command, args = {}) => String(args.text).replace('I will update the header.', '我会更新标题。').replace('Next I will check the table.', '接下来检查表格。'));
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
  await act(async () => { await import('./Markdown'); });
  vi.useFakeTimers();
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
async function render(text: string, options: { enabled?: boolean; streaming?: boolean; sessionId?: string } = {}) {
  await act(async () => root.render(<ReplyTranslationContext.Provider value={options.enabled ?? true}><MediaContext.Provider value={{ sessionId:options.sessionId ?? session }}><ReplyText text={text} streaming={options.streaming}/></MediaContext.Provider></ReplyTranslationContext.Provider>));
}
async function settle() { await act(async () => { await vi.advanceTimersByTimeAsync(750); }); }
async function click(label: string) {
  await act(async () => { Array.from(node.querySelectorAll('button')).find(button => button.textContent === label)!.click(); });
}

it('automatically displays Chinese, preserves code, and switches original without another request', async () => {
  const text = 'I will update the header.\n\n```sh\nnpm run build\n```';
  await render(text); await settle();
  expect(node.textContent).toContain('我会更新标题。');
  expect(node.querySelector('code')?.textContent).toBe('npm run build\n');
  expect(call).toHaveBeenCalledTimes(1);
  await click('显示原文');
  expect(node.textContent).toContain('I will update the header.');
  expect(node.textContent).not.toContain('我会更新标题。');
  await click('显示中文');
  expect(node.textContent).toContain('我会更新标题。');
  expect(call).toHaveBeenCalledTimes(1);
  await render(text); await settle();
  expect(call).toHaveBeenCalledTimes(1);
});

it('keeps original during failure, offers explicit retry and never reuses a broken result', async () => {
  vi.mocked(call).mockRejectedValueOnce(new Error('offline'));
  await render('I will update the header.'); await settle();
  expect(node.textContent).toContain('I will update the header.');
  expect(node.textContent).toContain('翻译暂不可用');
  await settle(); expect(call).toHaveBeenCalledTimes(1);
  await click('重试翻译'); await settle();
  expect(node.textContent).toContain('我会更新标题。');
  expect(call).toHaveBeenCalledTimes(2);
});

it('translates stable streaming paragraphs once, then translates the final tail', async () => {
  await render('I will update the header.\n\nNext I will check', { streaming:true }); await settle();
  expect(node.textContent).toContain('我会更新标题。');
  expect(node.textContent).toContain('Next I will check');
  expect(call).toHaveBeenCalledTimes(1);
  await click('显示原文');
  await render('I will update the header.\n\nNext I will check the table.', { streaming:true }); await settle();
  expect(node.textContent).toContain('I will update the header.');
  expect(call).toHaveBeenCalledTimes(1);
  await render('I will update the header.\n\nNext I will check the table.'); await settle();
  await click('显示中文');
  expect(node.textContent).toContain('接下来检查表格。');
  expect(call).toHaveBeenCalledTimes(2);
});

it('ignores stale in-flight results after switching chats and supports disabling automatic translation', async () => {
  let complete!: (value: string) => void;
  vi.mocked(call).mockImplementationOnce(() => new Promise<string>(resolve => { complete = resolve; }));
  await render('I will update the header.'); await settle();
  await render('Next I will check the table.', { sessionId:`${session}-other` });
  await act(async () => complete('旧聊天的译文。'));
  expect(node.textContent).not.toContain('旧聊天的译文');
  await settle();
  expect(node.textContent).toContain('接下来检查表格。');
  await render('Next I will check the table.', { sessionId:`${session}-other`, enabled:false }); await settle();
  expect(node.textContent).toContain('Next I will check the table.');
  expect(node.querySelector('.reply-translation')).toBeNull();
});

it('waits for visibility and drops queued work when a reply scrolls away', async () => {
  let observe!: IntersectionObserverCallback;
  vi.stubGlobal('IntersectionObserver', class {
    constructor(callback: IntersectionObserverCallback) { observe = callback; }
    observe() {} disconnect() {}
  });
  await render('I will update the header.'); await settle();
  expect(call).not.toHaveBeenCalled();
  await act(async () => observe([{ isIntersecting:true }] as IntersectionObserverEntry[], {} as IntersectionObserver));
  await settle(); expect(call).toHaveBeenCalledTimes(1);
});

it('does not translate Chinese, code-only content or an unfinished streaming sentence', async () => {
  for (const text of ['这是中文回复。', '```sh\nnpm run build\n```', 'I will update the header.']) {
    await render(text, { streaming:true }); await settle();
  }
  expect(call).not.toHaveBeenCalled();
});
