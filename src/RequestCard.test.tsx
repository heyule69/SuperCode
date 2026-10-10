// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import RequestCard from './RequestCard';
import type { RpcEvent } from './types';

let root: Root, node: HTMLDivElement;
const respond = vi.fn();
const questions = [{ id: 'next', question: '接下来怎么做？', options: [{ label: '继续', description: '使用已连接的工具' }, { label: '稍后' }] }];
const request = (params = {}, id: string | number = 1): RpcEvent => ({ id, method: 'item/tool/requestUserInput', params: { questions, ...params } });
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  respond.mockReset(); respond.mockResolvedValue(undefined);
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(value = request()) { await act(async () => root.render(<RequestCard request={value} respond={respond} />)); }
const button = (text: string) => [...node.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === text)!;
async function click(selector: string) { await act(async () => node.querySelector<HTMLButtonElement>(selector)!.click()); }
async function input(value: string) {
  const field = node.querySelector<HTMLTextAreaElement>('textarea')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, value);
    field.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
async function key(key: string, target: HTMLElement = node.querySelector('section')!) { await act(async () => target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))); }

it('waits for an explicit selection, then submits the native answer shape', async () => {
  await render();
  expect(button('发送').disabled).toBe(true); expect(respond).not.toHaveBeenCalled();
  await click('.question-request-option');
  expect(node.querySelector('[aria-pressed=true]')?.textContent).toContain('继续');
  await act(async () => button('发送').click());
  expect(respond).toHaveBeenCalledExactlyOnceWith({ answers: { next: { answers: ['继续'] } } });
});
it('lets custom text replace a single choice, without treating numbers in text as shortcuts', async () => {
  await render(); await click('.question-request-option'); await input('请使用现有 MCP');
  expect(node.querySelector('[aria-pressed=true]')).toBeNull();
  await key('2', node.querySelector('textarea')!);
  await key('Enter', node.querySelector('textarea')!);
  expect(respond).toHaveBeenCalledExactlyOnceWith({ answers: { next: { answers: ['请使用现有 MCP'] } } });
});
it('supports numbered shortcuts without sending merely by selecting an option', async () => {
  await render(); await key('2'); expect(respond).not.toHaveBeenCalled(); await key('Enter');
  expect(respond).toHaveBeenCalledExactlyOnceWith({ answers: { next: { answers: ['稍后'] } } });
});
it('keeps all answers while navigating multiple questions and supports multiple selections', async () => {
  await render(request({ questions: [...questions, { id: 'features', header: '功能', question: '选哪些？', multiSelect: true, options: [{ label: 'A' }, { label: 'B' }] }] }));
  await key('1'); await act(async () => button('下一题').click());
  expect(node.textContent).toContain('2 / 2'); expect(respond).not.toHaveBeenCalled();
  await key('1'); await key('2'); await input('以及 C');
  await click('[aria-label="上一题"]'); expect(node.querySelector('[aria-pressed=true]')?.textContent).toContain('继续');
  await act(async () => button('下一题').click()); await act(async () => button('发送').click());
  expect(respond).toHaveBeenCalledExactlyOnceWith({ answers: { next: { answers: ['继续'] }, features: { answers: ['A', 'B', '以及 C'] } } });
});
it('skips Codex questions with empty native answers even after selecting a choice', async () => {
  await render(); await key('1'); await act(async () => button('跳过').click());
  expect(respond).toHaveBeenCalledExactlyOnceWith({ answers: { next: { answers: [] } } });
});
it('closes Claude questions through native denial without approving the tool or sending invalid empty answers', async () => {
  await render(request({ toolName: 'AskUserQuestion' }, 'claude:ask'));
  await click('[aria-label="跳过并关闭提问"]');
  expect(respond).toHaveBeenCalledExactlyOnceWith({ decision: 'decline' });
});
it('restricts Pi select to its advertised options and preserves cancellation', async () => {
  await render(request({ questions: [{ ...questions[0], id: '0' }], nativeRequest: { method: 'select' } }, 'pi:select'));
  expect(node.querySelector('textarea')).toBeNull();
  await act(async () => button('跳过').click());
  expect(respond).toHaveBeenCalledExactlyOnceWith({ answers: { '0': { answers: [] } } });
});
it('retains answers after a failed send and prevents duplicate submissions while pending', async () => {
  let reject!: (error: Error) => void;
  respond.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  await render(); await key('1'); await key('Enter'); await key('Enter');
  expect(respond).toHaveBeenCalledOnce(); expect(button('发送中…').disabled).toBe(true);
  await act(async () => reject(new Error('连接暂时中断')));
  expect(node.querySelector('[role=alert]')?.textContent).toBe('连接暂时中断');
  await act(async () => button('发送').click()); expect(respond).toHaveBeenCalledTimes(2);
  expect(respond.mock.calls[1][0]).toEqual({ answers: { next: { answers: ['继续'] } } });
});
it('resets selections when another native request replaces the current question', async () => {
  await render(); await key('1'); await render(request({}, 2));
  expect(button('发送').disabled).toBe(true); expect(node.querySelector('[aria-pressed=true]')).toBeNull();
});
it('keeps approval requests separate and denies explicitly instead of auto approving', async () => {
  await render({ id: 'approval', method: 'claude/tool/requestApproval', params: { toolName: 'Shell', input: { command: 'example' } } });
  expect(node.querySelector('.question-request')).toBeNull(); expect(respond).not.toHaveBeenCalled();
  await act(async () => button('拒绝').click()); expect(respond).toHaveBeenCalledExactlyOnceWith({ decision: 'decline' });
});
