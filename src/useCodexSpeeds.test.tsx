// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
import { useCodexSpeeds } from './useCodexSpeeds';

vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
let node: HTMLDivElement, root: Root;
function Capabilities({ connection, model = 'official-model', enabled = true, sdkRevision }: { connection: string; model?: string; enabled?: boolean; sdkRevision?: string }) {
  const state = useCodexSpeeds(enabled, connection, model, sdkRevision);
  return <output>{JSON.stringify(state)}</output>;
}
beforeEach(() => { (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; vi.mocked(call).mockReset(); node = document.createElement('div'); document.body.append(node); root = createRoot(node); });
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });

it('does not query native capabilities for an inactive official route', async () => {
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

it('distinguishes a missing native model from an unfinished capability query without inventing tiers', async () => {
  vi.mocked(call).mockResolvedValue({ modelUnavailable: true });
  await act(async () => root.render(<Capabilities connection="@official"/>));
  expect(JSON.parse(node.textContent!)).toMatchObject({ modelUnavailable: true, loading: false });
  expect(JSON.parse(node.textContent!).tiers).toBeUndefined();
});

it('rechecks capabilities when the selected Codex installation changes outside this picker', async () => {
  vi.mocked(call).mockResolvedValue({ modelUnavailable: true });
  await act(async () => root.render(<Capabilities connection="@official" sdkRevision="old/codex.exe"/>));
  vi.mocked(call).mockResolvedValue({ tiers: ['default','fast','ultrafast'] });
  await act(async () => root.render(<Capabilities connection="@official" sdkRevision="new/codex.exe"/>));
  expect(call).toHaveBeenCalledTimes(2);
  expect(JSON.parse(node.textContent!).tiers).toEqual(['default','fast','ultrafast']);
  expect(JSON.parse(node.textContent!).modelUnavailable).toBeUndefined();
});
