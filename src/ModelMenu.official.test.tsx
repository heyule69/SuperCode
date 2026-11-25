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
  await render();
  expect(call).toHaveBeenCalledWith('list_provider_accounts', { force: false, agent: 'codex' });
  expect(call).toHaveBeenCalledWith('list_models', { agent: 'codex', connectionId: '@official', forceRefresh: false });
  await open();
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

it('keeps the verified official account visible while its first catalog is pending', async () => {
  let finish!: (value: ModelCatalog) => void;
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts'
    ? [{ agent: 'codex', loggedIn: true }]
    : new Promise(resolve => { finish = resolve; }));
  await render(); await open();
  const chip = [...node.querySelectorAll<HTMLButtonElement>('.model-provider-filters button')].find(button => button.textContent === 'ChatGPT 官方账号');
  expect(chip).toBeDefined();
  await act(async () => chip!.click());
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('正在加载模型');
  await act(async () => finish(official));
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
});

it('preserves the official catalog when switching to it and back to an API connection', async () => {
  await render(); await open();
  const officialProps = { models: official.data, source: official.source, catalogLoaded: true, profiles: [api], connectionId: '@official', agent: 'codex', busy: false, loading: false, choose, reload, manage: () => {}, effort: '', setEffort: () => {}, hasConversation: true };
  await act(async () => root.render(<ModelMenu {...officialProps} value="official-model"/>));
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts'
    ? [{ agent: 'codex', loggedIn: true }]
    : new Promise(() => {}));
  await render();
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
  expect(node.querySelector('.model-provider-filters')?.textContent).toContain('ChatGPT 官方账号');
});

it('retains a catalog first loaded as the active official connection when moving to an API', async () => {
  await act(async () => root.render(<ModelMenu value="official-model" models={official.data} source={official.source} catalogLoaded profiles={[api]}
    connectionId="@official" agent="codex" busy={false} loading={false} choose={choose} reload={reload} manage={() => {}} effort="" setEffort={() => {}} hasConversation/>));
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : new Promise(() => {}));
  await render(); await open();
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
});

it('removes an active account on confirmed logout even when its previous catalog says available', async () => {
  const renderOfficial = async (profiles: AgentProfile[]) => act(async () => root.render(<ModelMenu value="official-model" models={official.data} source={official.source} catalogLoaded profiles={profiles}
    connectionId="@official" agent="codex" busy={false} loading={false} choose={choose} reload={reload} manage={() => {}} effort="" setEffort={() => {}} setServiceTier={() => {}} hasConversation/>));
  await renderOfficial([api]); await open();
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: false }] : official);
  await renderOfficial([{ ...api }]);
  expect(node.querySelector('.model-menu-scroll')?.textContent).not.toContain('Official Model');
  expect(node.querySelector('.model-provider-filters')?.textContent ?? '').not.toContain('ChatGPT 官方账号');
  expect(node.querySelector('[aria-label="选择速度"]')).toBeNull();
});

it('reports official catalog failures and allows an explicit refresh', async () => {
  vi.mocked(call).mockImplementation(async command => { if (command === 'list_provider_accounts') return [{ agent: 'codex', loggedIn: true }]; throw new Error('官方目录读取失败'); });
  await render(); await open();
  expect(node.querySelector('.model-menu-footer')?.textContent).toContain('官方目录读取失败');
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : official);
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="刷新模型"]')!.click());
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
  expect(call).toHaveBeenCalledWith('list_models', { agent: 'codex', connectionId: '@official', forceRefresh: true });
  const calls = vi.mocked(call).mock.calls.length;
  await open(); await open();
  expect(vi.mocked(call).mock.calls.length).toBe(calls);
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
  expect(node.querySelector('.model-chip-effort')?.textContent).toBe('Ultra');
  expect(node.querySelector('.model-menu-footer')?.textContent).not.toContain('消耗更多额度');
  await act(async () => speed.click());
  await act(async () => [...node.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(button => button.textContent === '标准')!.click());
  expect(speed.textContent).toContain('标准');
  expect(node.querySelector('.model-chip-effort')?.textContent).toBe('Ultra');
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
  expect(node.querySelector('.model-chip-effort')?.textContent).toBe('Ultra');
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择速度"]')!.click());
  await act(async () => [...node.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].find(button => button.textContent === '标准')!.click());
  expect(node.querySelector('[aria-label="选择速度"]')?.textContent).toContain('标准');
});

it('disables Ultrafast when the account or installed CLI does not advertise it', async () => {
  vi.mocked(call).mockImplementation(async command => command === 'list_provider_accounts' ? [{ agent: 'codex', loggedIn: true }] : { tiers: ['default','fast'] });
  await act(async () => root.render(<SpeedPicker/>)); await open();
  expect(node.querySelector('.model-menu-footer')?.textContent).not.toContain('当前账号或本机 Codex 未提供超快');
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

it('keeps preloaded official options through close, reopen and unrelated profile updates', async () => {
  await render();
  const initial = vi.mocked(call).mock.calls.filter(([command]) => command === 'list_models').length;
  await open(); await open(); await render([{ ...api, name: 'Renamed API' }]); await open();
  expect(node.querySelector('.model-menu-scroll')?.textContent).toContain('Official Model');
  expect(vi.mocked(call).mock.calls.filter(([command]) => command === 'list_models')).toHaveLength(initial);
  expect(reload).not.toHaveBeenCalled();
});

it('repairs an old SDK through the existing updater and rechecks actual native speeds', async () => {
  let updated = false, complete!: () => void;
  vi.mocked(call).mockImplementation(async command => {
    if (command === 'list_provider_accounts') return [{ agent: 'codex', loggedIn: true }];
    if (command === 'get_codex_model_speeds') return updated ? { tiers: ['default','fast','ultrafast'] } : { modelUnavailable: true };
    if (command === 'update_agent') { await new Promise<void>(resolve => { complete = resolve; }); updated = true; return; }
    return official;
  });
  await act(async () => root.render(<SpeedPicker/>));
  expect(call).toHaveBeenCalledWith('get_codex_model_speeds', { connectionId: '@official', model: 'official-model' });
  await open();
  const update = node.querySelector<HTMLButtonElement>('.model-menu-status button')!;
  expect(update.textContent).toBe('更新 Codex');
  await act(async () => update.click());
  expect(call).toHaveBeenCalledWith('update_agent', { id: 'codex' });
  expect(update.disabled).toBe(true);
  await act(async () => complete());
  expect(reload).toHaveBeenCalledWith(true);
  expect(node.querySelector('.model-menu-status')).toBeNull();
  await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择速度"]')!.click());
  expect([...node.querySelectorAll<HTMLButtonElement>('.model-setting-menu button')].find(button => button.textContent === '超快')?.disabled).toBe(false);
  expect(choose).not.toHaveBeenCalled();
});

it('shows update errors without enabling unsupported speeds and keeps an explicit retry action', async () => {
  vi.mocked(call).mockImplementation(async command => {
    if (command === 'list_provider_accounts') return [{ agent: 'codex', loggedIn: true }];
    if (command === 'get_codex_model_speeds') return { modelUnavailable: true };
    if (command === 'update_agent') throw new Error('请先等待其他任务结束');
    return official;
  });
  await act(async () => root.render(<SpeedPicker/>)); await open();
  await act(async () => node.querySelector<HTMLButtonElement>('.model-menu-status button')!.click());
  expect(node.querySelector('.model-menu-error summary')?.textContent).toContain('更新失败');
  expect(node.querySelector('.model-menu-error p')?.textContent).toContain('请先等待其他任务结束');
  expect([...node.querySelectorAll('button')].some(button => button.textContent === '重试更新')).toBe(true);
  expect(reload).not.toHaveBeenCalled();
});

it('shows only relevant failures and keeps healthy account selection free of explanatory paragraphs', async () => {
  const magpie = { ...api, id: 'magpie:codex', name: 'Magpie', providerId: 'magpie', current: false, models: ['saved-model'] };
  await act(async () => root.render(<ModelMenu value="proxy-model" models={[{ id: 'proxy-model', model: 'proxy-model', displayName: 'Proxy Model', isDefault: true }]}
    catalogLoaded catalogWarning="上次使用的模型已不在 Magpie 目录" profiles={[api, magpie]} activeProfile={api} connectionId={api.id} agent="codex" busy={false} loading={false} choose={choose} reload={reload} manage={() => {}} effort="" setEffort={() => {}} hasConversation/>));
  await open();
  await act(async () => [...node.querySelectorAll<HTMLButtonElement>('.model-provider-filters button')].find(button => button.textContent === 'ChatGPT 官方账号')!.click());
  expect(node.querySelector('.model-menu-footer')?.textContent).toBe('模型设置');
  expect(node.querySelector('.model-menu-footer p')).toBeNull();
});
