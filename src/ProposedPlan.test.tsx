// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ProposedPlan from './ProposedPlan';

let root:Root,node:HTMLDivElement;
const plan='# 修复购物清单\n\n完整计划正文。\n\n| 用例 | 结果 |\n| --- | --- |\n| 空名称 | 拒绝 |\n\n```js\nvalidate(items);\n```\n\n计划末尾验收条件。';
beforeEach(async()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
  node=document.createElement('div');document.body.append(node);root=createRoot(node);
  await act(async()=>{await import('./Markdown');});
});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();});
it('shows the full streaming document without an execution action and keeps history read-only',async()=>{
  const respond=vi.fn(async()=>{});
  await act(async()=>root.render(<ProposedPlan text={plan} streaming respond={respond} openFile={()=>{}}/>));
  expect(node.textContent).toContain('计划末尾验收条件');
  expect(node.querySelector('table')).not.toBeNull();
  expect(node.querySelector('.plan-request-execute')).toBeNull();
  expect(node.querySelector('[aria-label="复制代码"]')).toBeNull();
  await act(async()=>root.render(<ProposedPlan text={plan} streaming={false} openFile={()=>{}}/>));
  expect(node.querySelector('.plan-request-footer')).toBeNull();
  expect(respond).not.toHaveBeenCalled();
});
it('requires a click and submits execution once without applying unsubmitted revision feedback',async()=>{
  let finish!:()=>void;const respond=vi.fn(()=>new Promise<void>(resolve=>{finish=resolve;}));
  await act(async()=>root.render(<ProposedPlan text={plan} streaming={false} respond={respond} openFile={()=>{}}/>));
  expect(respond).not.toHaveBeenCalled();
  const input=node.querySelector('textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,'不要擅自合并这个修改意见');input.dispatchEvent(new Event('input',{bubbles:true}));});
  const execute=node.querySelector<HTMLButtonElement>('.plan-request-execute')!;
  await act(async()=>{execute.click();execute.click();});
  expect(respond).toHaveBeenCalledExactlyOnceWith('execute','');
  expect(execute.disabled).toBe(true);
  await act(async()=>finish());
  expect(node.textContent).toContain('已确认执行');
  expect(execute.disabled).toBe(true);
});
it('sends revision feedback, reports a failed submission and permits retry',async()=>{
  const respond=vi.fn().mockRejectedValueOnce(new Error('连接暂不可用')).mockResolvedValue(undefined);
  await act(async()=>root.render(<ProposedPlan text={plan} streaming={false} respond={respond} openFile={()=>{}}/>));
  const input=node.querySelector('textarea')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')!.set!.call(input,' 包含边界用例 ');input.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>node.querySelector<HTMLButtonElement>('.plan-request-revise')!.click());
  expect(respond).toHaveBeenCalledWith('revise','包含边界用例');
  expect(node.querySelector('[role="alert"]')?.textContent).toContain('连接暂不可用');
  expect(node.querySelector<HTMLButtonElement>('.plan-request-revise')!.disabled).toBe(false);
  await act(async()=>node.querySelector<HTMLButtonElement>('.plan-request-revise')!.click());
  expect(node.textContent).toContain('已要求修改');
  expect(node.querySelector('[role="alert"]')).toBeNull();
});
