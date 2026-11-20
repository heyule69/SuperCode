// @vitest-environment happy-dom
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
import { ModelMenu } from './ModelMenu';
import type { AgentProfile, ModelCatalog } from './types';

vi.mock('./api', () => ({ desktop: true, call: vi.fn() }));
const api: AgentProfile = { id: 'api', agent: 'codex', name: 'My API', model: 'proxy-model', current: true, hasCredential: true };
const official: ModelCatalog = { source: { providerId: 'openai', providerName: 'OpenAI', connectionName: 'ChatGPT 官方账号', mark: 'O', available: true }, data: [{ id: 'official-model', model: 'official-model', displayName: 'Official Model', isDefault: true, supportedReasoningEfforts: [{ reasoningEffort: 'ultra' }] }] };
let root: Root, node: HTMLDivElement;
const choose = vi.fn(), reload = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(call).mockReset().mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : command === 'get_codex_model_speeds' ? { tiers: ['default','fast','ultrafast'] } : official);
  choose.mockClear(); reload.mockClear();
  node = document.createElement('div'); document.body.append(node); root = createRoot(node);
});
afterEach(async () => { await act(async () => root.unmount()); node.remove(); });
async function render(profiles = [api]) {
  await act(async () => root.render(<ModelMenu value="proxy-model" models={[{ id: 'proxy-model', model: 'proxy-model', displayName: 'Proxy Model', isDefault: true }]} catalogLoaded profiles={profiles} activeProfile={api} connectionId={api.id} agent="codex" busy={false} loading={false} choose={choose} reload={reload} manage={() => {}} effort="" setEffort={() => {}} hasConversation/>));
}
async function open() { await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click()); }

it('loads the signed-in native official catalog while an API is selected, without changing the route', async () => {
  await render(); expect(call).not.toHaveBeenCalled(); await open();
  expect(call).toHaveBeenCalledWith('list_models', { agent: 'codex', connectionId: '@official', forceRefresh: false });
  const chip = [...node.querySelectorAll<HTMLButtonElement>('.model-provider-filters button')].find(button => button.textContent === 'ChatGPT 官方账号')!;
  expect(chip).toBeDefined(); await act(async () => chip.click());
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
  expect(node.querySelector('.model-chip-name')?.textContent).toBe('Proxy Model');
  expect(choose).not.toHaveBeenCalled();
  await act(async () => node.querySelector<HTMLButtonElement>('.model-option')!.click());
  expect(choose).toHaveBeenCalledWith(expect.objectContaining({ connectionId: '@official', model: 'official-model', supportedReasoningEfforts: [{ reasoningEffort: 'ultra' }] }));
});

it('does not request an official catalog for a signed-out account', async () => {
  vi.mocked(call).mockResolvedValue([{ agent: 'codex', loggedIn: false }]);
  await render(); await open();
  expect(call).not.toHaveBeenCalledWith('list_models', expect.anything());
  expect((node.querySelector('.model-provider-filters')?.textContent ?? '')).not.toContain('ChatGPT 官方账号');
});

it('reports official catalog failures and allows an explicit refresh', async () => {
  vi.mocked(call).mockImplementation(async command => { if (command === 'list_provider_accounts') return [{ agent: 'codex', loggedIn: true }]; throw new Error('官方目录读取失败'); });
  await render(); await open();
  expect(node.querySelector('.model-menu-footer')?.textContent).toContain('官方目录读取失败');
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : official);
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="刷新模型"]')!.click());
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
  expect(call).toHaveBeenCalledWith('list_models', { agent: 'codex', connectionId: '@official', forceRefresh: true });
  await open(); await open();
  expect(vi.mocked(call).mock.calls.filter(([command]) => command === 'list_models').at(-1)?.[1]).toEqual({ agent: 'codex', connectionId: '@official', forceRefresh: false });
});

it('queries a managed official account by its connection id and never uses a signed-out managed account', async () => {
  const managed: AgentProfile = { ...api, id: 'managed-account', name: 'Work Account', current: false, officialAccount: true, accountId: 'managed-home', model: null };
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', connectionId: managed.id, loggedIn: true }] : { ...official, source: { ...official.source!, connectionName: 'Work Account' } });
  await render([api, managed]); await open();
  expect(call).toHaveBeenCalledWith('list_models', { agent: 'codex', connectionId: managed.id, forceRefresh: false });
  expect(call).not.toHaveBeenCalledWith('list_models', expect.objectContaining({ connectionId: '@official' }));
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Work Account');
});

it('discards an official catalog that returns after the account is removed', async () => {
  let finish!: (value: ModelCatalog) => void;
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : new Promise(resolve => { finish = resolve; }));
  await render(); await open();
  vi.mocked(call).mockResolvedValue([{ agent: 'codex', loggedIn: false }]);
  await render([{ ...api }]);
  await act(async () => finish(official));
  expect(node.querySelector('.model-menu-scroll')?.textContent).not.toContain('Official Model');
});

function SpeedPicker({ fast = true, agent = 'codex' }: { fast?: boolean; agent?: string }) {
  const [effort, setEffort] = useState('ultra'), [serviceTier, setServiceTier] = useState('');
  return <ModelMenu value="official-model" models={[{ ...official.data[0], serviceTiers: fast ? [{ id: 'fast', name: 'Fast' }, { id: 'ultrafast', name: 'Ultrafast' }] : [] }]}
    profiles={[]} source={official.source} catalogLoaded connectionId="@official" agent={agent} busy={false} loading={false} choose={choose} reload={reload} manage={() => {}}
    effort={effort} setEffort={setEffort} serviceTier={serviceTier} setServiceTier={setServiceTier} hasConversation/>;
}

it('keeps Ultra reasoning while selecting Fast, then explicitly returns to Standard', async () => {
  await act(async () => root.render(<SpeedPicker/>)); await open();
  const speed = node.querySelector<HTMLButtonElement>('[aria-label="选择速度"]')!;
  expect(speed.textContent).toContain('标准');
  await act(async () => speed.click());
  await act(async () => [...node.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(button => button.textContent === 'Fast')!.click());
  expect(speed.textContent).toContain('Fast');
  expect(node.querySelector('.model-chip-effort')?.textContent).toBe('超高');
  expect(node.querySelector('.model-menu-footer')?.textContent).toContain('消耗更多额度');
  await act(async () => speed.click());
  await act(async () => [...node.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(button => button.textContent === '标准')!.click());
  expect(speed.textContent).toContain('标准');
  expect(node.querySelector('.model-chip-effort')?.textContent).toBe('超高');
});

it('does not offer Fast without model capability metadata', async () => {
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : { tiers: ['default'] });
  await act(async () => root.render(<SpeedPicker fast={false}/>)); await open();
  expect(node.querySelector('[aria-label="选择速度"]')).toBeNull();
});

it('selects Ultrafast independently of Ultra reasoning and returns to Standard', async () => {
  await act(async () => root.render(<SpeedPicker/>)); await open();
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择速度"]')!.click());
  const ultrafast = [...node.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].find(button => button.textContent === '超快')!;
  expect(ultrafast.disabled).toBe(false);
  await act(async () => ultrafast.click());
  expect(node.querySelector('[aria-label="选择速度"]')?.textContent).toContain('超快');
  expect(node.querySelector('.model-chip-effort')?.textContent).toBe('超高');
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择速度"]')!.click());
  await act(async () => [...node.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].find(button => button.textContent === '标准')!.click());
  expect(node.querySelector('[aria-label="选择速度"]')?.textContent).toContain('标准');
});

it('disables Ultrafast when the account or installed CLI does not advertise it', async () => {
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : { tiers: ['default','fast'] });
  await act(async () => root.render(<SpeedPicker/>)); await open();
  expect(node.querySelector('.model-menu-footer')?.textContent).toContain('当前账号或本机 Codex 未提供超快');
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择速度"]')!.click());
  expect([...node.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].find(button => button.textContent === '超快')?.disabled).toBe(true);
  expect([...node.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].find(button => button.textContent === 'Fast')?.disabled).toBe(false);
});

it('shows capability failures and does not enable a paid speed before capability verification', async () => {
  vi.mocked(call).mockImplementation(async command => { if (command === 'list_provider_accounts') return [{ agent: 'codex', loggedIn: true }]; throw new Error('速度能力读取失败'); });
  await act(async () => root.render(<SpeedPicker/>)); await open();
  expect(node.querySelector('.model-menu-footer')?.textContent).toContain('速度能力读取失败');
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择速度"]')!.click());
  expect([...node.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].filter(button => button.textContent !== '标准').every(button => button.disabled)).toBe(true);
});
