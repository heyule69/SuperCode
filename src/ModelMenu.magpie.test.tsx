// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
import { ModelMenu } from './ModelMenu';
import type { AgentProfile, ModelCatalog } from './types';

vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
const source = { providerId: 'magpie', providerName: 'Magpie', mark: 'M', available: true };
const current: AgentProfile = { id: 'api', agent: 'pi', name: 'API', model: 'current-model', current: true, hasCredential: true };
const magpie: AgentProfile = { id: 'magpie:pi', agent: 'pi', name: 'Magpie', providerId: 'magpie', model: 'old-model', models: ['old-model'], current: false, hasCredential: true };
const catalog = (id: string): ModelCatalog => ({ data: [{ id, model: id, displayName: id, isDefault: false, supportedReasoningEfforts: [{ reasoningEffort: 'high' }] }], source });
let root: Root, node: HTMLDivElement;
const choose = vi.fn(), reload = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(call).mockReset().mockImplementation(async command => command === 'list_models' ? catalog('new-model') : []);
  choose.mockClear(); reload.mockClear();
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(profiles = [current, magpie]) {
  await act(async () => root.render(<ModelMenu value="current-model" models={[{ id: 'current-model', model: 'current-model', displayName: 'current-model', isDefault: true }]} catalogLoaded profiles={profiles} activeProfile={current} connectionId={current.id} agent="pi" busy={false} loading={false} choose={choose} reload={reload} manage={() => {}} effort="" setEffort={() => {}} hasConversation/>));
}
async function toggle() { await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click()); }

it('loads a nonselected Magpie connection on opening and preserves the chat selection and model metadata', async () => {
  await render(); expect(call).not.toHaveBeenCalled();
  await toggle();
  expect(call).toHaveBeenCalledExactlyOnceWith('list_models', { agent: 'pi', connectionId: magpie.id, forceRefresh: true });
  const filter = [...node.querySelectorAll<HTMLButtonElement>('.model-provider-filters button')].find(button => button.textContent === 'Magpie')!;
  await act(async () => filter.click());
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('new-model');
  expect(node.querySelector('.model-menu-scroll')?.textContent).not.toContain('old-model');
  expect(node.querySelector('.model-chip-name')?.textContent).toBe('current-model');
  expect(choose).not.toHaveBeenCalled();
  await act(async () => node.querySelector<HTMLButtonElement>('.model-option')!.click());
  expect(choose).toHaveBeenCalledWith(expect.objectContaining({ connectionId: magpie.id, model: 'new-model', supportedReasoningEfforts: [{ reasoningEffort: 'high' }] }));
});

it('refreshes the Magpie group on reopening and on explicit refresh while another connection remains selected', async () => {
  await render(); await toggle();
  await render(); expect(call).toHaveBeenCalledTimes(1);
  await toggle();
  vi.mocked(call).mockResolvedValue(catalog('added-after-opening'));
  await toggle();
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('added-after-opening');
  expect(call).toHaveBeenCalledTimes(2);
  vi.mocked(call).mockResolvedValue(catalog('added-after-refresh'));
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="刷新模型"]')!.click());
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('added-after-refresh');
  expect(call).toHaveBeenCalledTimes(3);
  expect(reload).toHaveBeenCalledWith(true);
});

it('ignores a delayed catalog after its connection was removed', async () => {
  let finish!: (value: ModelCatalog) => void;
  vi.mocked(call).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  await render(); await toggle();
  await render([current]);
  await act(async () => finish(catalog('removed-connection-model')));
  expect(node.querySelector('.model-menu-scroll')?.textContent).not.toContain('removed-connection-model');
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('current-model');
});

it('discards an old response after the menu closes and a newer opening has loaded', async () => {
  const finish: ((value: ModelCatalog) => void)[] = [];
  vi.mocked(call).mockImplementation(() => new Promise(resolve => { finish.push(resolve); }));
  await render(); await toggle(); await toggle(); await toggle();
  await act(async () => finish[1](catalog('latest-opening-model')));
  await act(async () => finish[0](catalog('stale-opening-model')));
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('latest-opening-model');
  expect(node.querySelector('.model-menu-scroll')?.textContent).not.toContain('stale-opening-model');
});

it('shows the existing saved catalog and backend warning when Magpie is offline', async () => {
  vi.mocked(call).mockResolvedValue({ ...catalog('old-model'), warning: '无法获取 Magpie 模型，当前保留已保存的模型。' });
  await render(); await toggle();
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('old-model');
  expect(node.querySelector('.model-menu-footer')?.textContent).toContain('无法获取 Magpie 模型');
  expect(node.querySelector<HTMLButtonElement>('[aria-label="刷新模型"]')?.disabled).toBe(false);
});

it('reports a rejected catalog request instead of silently hiding the error', async () => {
  vi.mocked(call).mockRejectedValue(new Error('目录读取失败'));
  await render(); await toggle();
  expect(node.querySelector('.model-menu-footer')?.textContent).toContain('目录读取失败');
  expect(node.querySelector<HTMLButtonElement>('[aria-label="刷新模型"]')?.disabled).toBe(false);
});
