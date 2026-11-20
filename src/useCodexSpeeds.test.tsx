// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
import { useCodexSpeeds } from './useCodexSpeeds';

vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
let node: HTMLDivElement, root: Root;
function Capabilities({ connection, model = 'official-model', enabled = true }: { connection: string; model?: string; enabled?: boolean }) {
  const state = useCodexSpeeds(enabled, connection, model);
  return <output>{JSON.stringify(state)}</output>;
}
beforeEach(() => { (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; vi.mocked(call).mockReset(); node = document.createElement('div'); document.body.append(node); root = createRoot(node); });
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });

it('does not query native capabilities until the model picker is opened', async () => {
  await act(async () => root.render(<Capabilities connection="@official" enabled={false}/>));
  expect(call).not.toHaveBeenCalled();
});

it('cannot apply an old account response to a newly selected official account', async () => {
  const pending: ((value: { tiers: string[] }) => void)[] = [];
  vi.mocked(call).mockImplementation(() => new Promise(resolve => { pending.push(resolve); }));
  await act(async () => root.render(<Capabilities connection="account-a"/>));
  await act(async () => root.render(<Capabilities connection="account-b"/>));
  expect(call).toHaveBeenLastCalledWith('get_codex_model_speeds', { connectionId: 'account-b', model: 'official-model' });
  await act(async () => pending[1]({ tiers: ['default','fast'] }));
  await act(async () => pending[0]({ tiers: ['default','fast','ultrafast'] }));
  expect(JSON.parse(node.textContent!).tiers).toEqual(['default','fast']);
});

it('checks a different model on the same account instead of reusing Ultrafast entitlement', async () => {
  vi.mocked(call).mockResolvedValue({ tiers: ['default','fast','ultrafast'] });
  await act(async () => root.render(<Capabilities connection="@official"/>));
  vi.mocked(call).mockResolvedValue({ tiers: ['default'] });
  await act(async () => root.render(<Capabilities connection="@official" model="other-model"/>));
  expect(call).toHaveBeenLastCalledWith('get_codex_model_speeds', { connectionId: '@official', model: 'other-model' });
  expect(JSON.parse(node.textContent!).tiers).toEqual(['default']);
});
