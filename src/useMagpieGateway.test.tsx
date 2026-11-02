// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
import { magpieGatewayText, useMagpieGateway } from './useMagpieGateway';
import { ModelMenu } from './ModelMenu';
vi.mock('./api', () => ({ desktop:true, call:vi.fn() }));
let root: Root, node: HTMLDivElement;
function Harness({ enabled, connectionId = 'magpie:claude' }: { enabled: boolean; connectionId?: string }) {
  const state = useMagpieGateway(enabled, 'claude', connectionId);
  return <span data-state={state}>{magpieGatewayText(state)}</span>;
}
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(call).mockReset().mockResolvedValue({ state:'reachable' });
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(enabled: boolean, connectionId?: string) { await act(async () => root.render(<Harness enabled={enabled} connectionId={connectionId}/>)); }

it('inspects only on opening, reports background reachability, and never launches Magpie or downloads a catalog', async () => {
  await render(false); expect(call).not.toHaveBeenCalled();
  await render(true); expect(node.textContent).toContain('后台运行');
  expect(call).toHaveBeenCalledExactlyOnceWith('get_magpie_status', { agent:'claude', connectionId:'magpie:claude' });
  await render(true); expect(call).toHaveBeenCalledTimes(1);
  await render(false); vi.mocked(call).mockResolvedValue({ state:'offline' }); await render(true);
  expect(call).toHaveBeenCalledTimes(2); expect(node.textContent).toContain('网关未连接');
});
it('ignores results from closed menus and a previously selected connection', async () => {
  const pending: ((value: unknown) => void)[] = [];
  vi.mocked(call).mockImplementation(() => new Promise(resolve => pending.push(resolve)));
  await render(true, 'old'); await render(false); await render(true, 'new');
  await act(async () => pending[1]({ state:'offline' })); expect(node.textContent).toContain('网关未连接');
  await act(async () => pending[0]({ state:'reachable' })); expect(node.textContent).toContain('网关未连接');
});
it('treats inspection errors as unknown, rather than claiming the gateway is online or offline', async () => {
  vi.mocked(call).mockRejectedValue(new Error('diagnostic unavailable'));
  await render(true); expect(node.querySelector('[data-state="unknown"]')).not.toBeNull();
  expect(node.textContent).toContain('暂时无法确认');
});
it('explains a remote gateway without claiming that the local application is online', async () => {
  vi.mocked(call).mockResolvedValue({ state:'remote' });
  await render(true); expect(node.textContent).toContain('远程 Magpie 网关');
  expect(node.textContent).toContain('无需本机 Magpie 窗口运行');
  expect(node.textContent).not.toContain('网关可连接');
});
it('keeps cached model IDs selectable with an explicit offline notice and a fresh read on reopening', async () => {
  vi.mocked(call).mockImplementation(async command => command === 'get_magpie_status' ? { state:'offline' } : []);
  const profile = { id:'magpie:claude', agent:'claude', name:'Magpie', model:'vendor/saved', models:['vendor/saved'], providerId:'magpie', current:true, hasCredential:true };
  await act(async () => root.render(<ModelMenu value={profile.model} models={[]} profiles={[profile]} source={{ providerId:'magpie', providerName:'Magpie', mark:'M', available:true }} activeProfile={profile} connectionId={profile.id} agent="claude" busy={false} loading={false} choose={async () => {}} reload={() => {}} manage={() => {}} effort="" setEffort={() => {}} hasConversation/>));
  const trigger = node.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!;
  await act(async () => trigger.click());
  expect(node.querySelector('.model-option')?.textContent).toContain('vendor/saved');
  expect(node.querySelector('.magpie-gateway-status')?.textContent).toContain('网关未连接');
  expect(node.textContent).toContain('已保存目录');
  expect(vi.mocked(call).mock.calls.every(([command]) => !['open_magpie', 'use_magpie', 'list_models', 'send_chat_message'].includes(command))).toBe(true);
  await act(async () => trigger.click()); await act(async () => trigger.click());
  expect(vi.mocked(call).mock.calls.filter(([command]) => command === 'get_magpie_status')).toHaveLength(2);
});
