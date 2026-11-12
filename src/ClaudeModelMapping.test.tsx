// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ClaudeModelMapping, type ClaudeMapping } from './ClaudeModelMapping';
import { call } from './api';
import type { AgentProfile } from './types';
vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
const profile: AgentProfile = { id:'magpie:claude', agent:'claude', name:'Magpie', model:'v/main', models:['v/main','v/child'], current:true, hasCredential:true, providerId:'magpie' };
const initial: ClaudeMapping = { mode:'follow',sonnet:'',opus:'',haiku:'',fable:'',subagent:'',forceSubagent:false };
let root: Root, node: HTMLDivElement;
const save = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(call).mockReset().mockResolvedValue(initial); save.mockClear();
  node=document.createElement('div');document.body.append(node);root=createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(inUse=false, p=profile) { await act(async () => root.render(<ClaudeModelMapping profile={p} busy={false} inUse={inUse} back={()=>{}} save={save}/>)); }
async function mapped() { await act(async () => node.querySelector<HTMLInputElement>('input[value="mapped"]')!.click()); }
it('defaults to following the actual chat model and exposes independent roles only on request', async () => {
  await render();
  expect(call).toHaveBeenCalledExactlyOnceWith('get_claude_model_mapping',{id:profile.id});
  expect(node.querySelector<HTMLInputElement>('input[value="follow"]')!.checked).toBe(true);
  expect(node.querySelector('#claude-mapping-haiku')).toBeNull();
  await mapped();
  expect(node.querySelectorAll('.claude-mapping-row')).toHaveLength(5);
  expect(node.querySelector('#claude-mapping-subagent')?.getAttribute('placeholder')).toBe('跟随当前主模型');
  expect(node.querySelector('datalist')?.textContent).toBe('');
  expect(node.querySelectorAll('datalist option')).toHaveLength(10);
});
it('saves chosen model IDs and explicit force intent without changing the provider model', async () => {
  await render(); await mapped();
  const input=node.querySelector<HTMLInputElement>('#claude-mapping-subagent')!;
  await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'v/child'); input.dispatchEvent(new Event('input',{bubbles:true})); });
  await act(async () => node.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  await act(async () => node.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  expect(save).toHaveBeenCalledExactlyOnceWith({...initial,mode:'mapped',subagent:'v/child',forceSubagent:true});
  expect(profile.model).toBe('v/main');
});
it('keeps a busy connection read-only and does not submit programmatic form events', async () => {
  await render(true);
  expect(node.textContent).toContain('此连接正被任务使用');
  expect(node.querySelector<HTMLButtonElement>('.primary-button')!.disabled).toBe(true);
  await act(async () => node.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true})));
  expect(save).not.toHaveBeenCalled();
});
it('shows read errors instead of saving a fabricated default mapping', async () => {
  vi.mocked(call).mockRejectedValue(new Error('连接已移除'));
  await render(); expect(node.querySelector('[role="alert"]')?.textContent).toContain('连接已移除');
  expect(node.querySelector('.primary-button')).toBeNull();
});
