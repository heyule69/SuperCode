import { expect, it } from 'vitest';
import { filterModelGroups, modelGroups, sourceLabel } from './modelPicker';
import type { AgentProfile, Model, ModelSource } from './types';

const kimi: ModelSource = { providerId: 'kimi', providerName: 'Kimi Code', mark: 'K', planName: '编程计划', connectionName: 'kimi' };
const model = (id: string, isDefault = false): Model => ({ id, model: id, displayName: id, isDefault });
const profile = (id: string, modelId = 'k3'): AgentProfile => ({ id, agent: 'claude', name: id, model: modelId, current: false, hasCredential: true, modelSource: { ...kimi, connectionName: id }, models: [modelId, 'kimi-for-coding'] });

it('keeps managed official accounts separate even when their model IDs match', () => {
  const accounts: AgentProfile[] = ['personal', 'work'].map(accountId => ({ id: `account:${accountId}`, accountId, agent: 'codex', name: accountId, model: 'same-model', officialAccount: true, current: false, hasCredential: false }));
  const options = { models: [model('same-model')], profiles: accounts, source: { providerId: 'openai', providerName: 'OpenAI', mark: 'O', connectionName: 'personal', available: true }, agent: 'codex', value: 'same-model', connectionId: 'account:personal', loggedInAgents: ['codex', 'account:personal', 'account:work'] };
  const groups = modelGroups(options);
  expect(groups.map(group => group.id)).toEqual(['account:personal', 'account:work']);
  expect(groups[0].options[0].connectionId).toBe('account:personal'); expect(groups[1].options[0].connectionId).toBe('account:work');
  expect(groups[0].source.connectionName).toBe('personal'); expect(groups[1].source.connectionName).toBe('work');
  expect(modelGroups({ ...options, source: { ...options.source, available: false } }).map(group => group.id)).toEqual(['account:work']);
  expect(modelGroups({ ...options, source: { ...options.source, available: false }, loggedInAgents: ['codex'] })).toEqual([]);
});

it('does not invent official Claude models or restore a catalog marked unavailable from saved models', () => {
  const groups = modelGroups({ models: [], profiles: [profile('kimi')], source: kimi, agent: 'claude', value: 'k3', connectionId: 'kimi', order: { claude: ['kimi', '@official', '@local'] } });
  expect(groups.map(group => group.id)).toEqual(['kimi']);
  const official: AgentProfile = { id: 'official', agent: 'codex', name: 'ChatGPT', model: 'saved-model', current: true, hasCredential: false, officialAccount: true };
  const unavailable = modelGroups({ models: [], profiles: [official], activeProfile: official, source: { providerId: 'openai', providerName: 'OpenAI', mark: 'O', available: false }, agent: 'codex', value: 'saved-model', connectionId: '@official' });
  expect(unavailable.flatMap(group => group.options)).toEqual([]);
});

it('merges Kimi Claude context aliases while preserving the exact selected execution ID', () => {
  const build = (value: string) => modelGroups({ models: [model('k3[1M]', true), model('k3'), model('k3-256k')], profiles: [profile('kimi', 'k3[1M]')], connectionId: 'kimi', source: kimi, agent: 'claude', value })[0].options;
  expect(build('k3')).toHaveLength(2);
  expect(build('k3')[0]).toMatchObject({ displayName: 'Kimi K3', model: 'k3', rawIds: ['k3[1M]', 'k3'], selected: true });
  expect(build('k3')[0].context).toBeUndefined();
  expect(build('k3[1M]')[0].context).toBe('1M');
  expect(build('k3[1M]')[0].model).toBe('k3[1M]');
  expect(build('')[0].model).toBe('k3[1M]');
  expect(build('k3')[1]).toMatchObject({ model: 'k3-256k', context: '256K', selected: false });
});

it('does not rewrite unknown custom IDs, standard Kimi API IDs or other models', () => {
  const raw = [model('private-model[1m]'), model('private-model'), model('kimi-k3')];
  const groups = modelGroups({ models: raw, profiles: [profile('gateway', 'private-model')], connectionId: 'gateway', source: { providerId: 'custom', providerName: '我的网关', mark: '+' }, agent: 'claude', value: 'private-model' });
  expect(groups[0].options.map(m => m.model)).toEqual(raw.map(m => m.model));
  expect(groups[0].options[2].displayName).toBe('kimi-k3');
});

it('merges known Kimi context variants on a custom connection without changing supplier or execution IDs', () => {
  const source: ModelSource = { providerId: 'custom', providerName: '我的 Kimi 连接', mark: '+', modelFamily: 'kimi' };
  const group = modelGroups({ models: [model('k3[1M]'), model('k3'), model('private-model')], profiles: [profile('custom-kimi')], source, agent: 'claude', value: 'k3[1M]', connectionId: 'custom-kimi' })[0];
  expect(group.options).toHaveLength(2);
  expect(group.options[0]).toMatchObject({ displayName: 'Kimi K3', model: 'k3[1M]', context: '1M', rawIds: ['k3[1M]', 'k3'] });
  expect(group.source.providerName).toBe('我的 Kimi 连接');
  expect(group.options[1].model).toBe('private-model');
});

it('keeps identically named models on different connections independently selectable', () => {
  const active = { ...profile('工作账号'), current: true };
  const second = profile('个人账号');
  const groups = modelGroups({ models: [model('k3')], profiles: [active, second, { ...profile('Codex'), agent: 'codex' }], activeProfile: active, agent: 'claude', value: 'k3' });
  expect(groups).toHaveLength(2);
  expect(groups[0].options[0].selected).toBe(true);
  expect(groups[1].options[0]).toMatchObject({ selected: false, connectionId: second.id, model: 'k3' });
  expect(groups[0].options[0].key).not.toBe(groups[1].options[0].key);
  expect(sourceLabel(groups[1].source)).toBe('Kimi Code · 编程计划 · 个人账号');
});

it('searches aliases, providers and plans and filters by connection without false results', () => {
  const groups = modelGroups({ models: [model('k3[1M]'), model('kimi-for-coding-highspeed')], profiles: [profile('kimi'), profile('备用')], connectionId: 'kimi', source: kimi, agent: 'claude', value: 'k3[1M]' });
  expect(sourceLabel(kimi)).toBe('Kimi Code · 编程计划');
  expect(sourceLabel({ providerId: 'zhipu', providerName: '智谱 GLM', planName: 'GLM Coding Plan', connectionName: '智谱 GLM · GLM Coding Plan', mark: '智' })).toBe('智谱 GLM · GLM Coding Plan');
  expect(filterModelGroups(groups, 'k3[1m]', null)[0].options).toHaveLength(1);
  expect(filterModelGroups(groups, '高速版', null)[0].options[0].displayName).toContain('高速版');
  expect(filterModelGroups(groups, '编程计划', '备用')).toHaveLength(1);
  expect(filterModelGroups(groups, 'unknown', null)).toEqual([]);
});

it('uses the conversation binding even when another connection is the default for new chats', () => {
  const bound = profile('当前聊天');
  const defaultProfile = { ...profile('新聊天默认'), current: true };
  const groups = modelGroups({ models: [model('k3')], profiles: [bound, defaultProfile], activeProfile: bound, connectionId: bound.id, agent: 'claude', value: 'k3' });
  expect(groups[0].id).toBe(bound.id);
  expect(groups[0].options[0].selected).toBe(true);
  expect(groups[1].options.every(option => !option.selected)).toBe(true);
  const official = modelGroups({ models: [model('official-model')], profiles: [defaultProfile], connectionId: '@official', source: { providerId: 'anthropic', providerName: 'Anthropic', mark: 'A', available: true }, agent: 'claude', value: 'official-model' });
  expect(official[0].options[0]).toMatchObject({ connectionId: '@official', selected: true });
});

it('follows settings order while the selected model remains on the conversation connection', () => {
  const selected = profile('glm', 'glm-5.3');
  const first = { ...profile('kimi'), current: true };
  const order = { claude: ['kimi', 'glm', '@official'], codex: ['other'] };
  const groups = modelGroups({ models: [model('glm-5.3')], profiles: [selected, first], activeProfile: selected, connectionId: 'glm', source: selected.modelSource, agent: 'claude', value: 'glm-5.3', order });
  expect(groups.map(g => g.id)).toEqual(['kimi', 'glm']);
  expect(groups[0].options.every(o => !o.selected)).toBe(true);
  expect(groups[1].options[0]).toMatchObject({ selected: true, connectionId: 'glm', model: 'glm-5.3' });
  const reversed = modelGroups({ models: [model('k3')], profiles: [first, selected], activeProfile: first, connectionId: 'kimi', agent: 'claude', value: 'k3', order: { claude: ['glm', 'kimi', '@official'] } });
  expect(reversed[0].id).toBe('glm');
  expect(reversed[1].options[0].selected).toBe(true);
});

it('shows one official Codex group while preserving an existing imported conversation binding', () => {
  const official = { ...profile('official-profile', 'configured-official-model'), agent: 'codex', officialAccount: true, models: ['configured-official-model'], modelSource: { providerId: 'openai', providerName: 'OpenAI', mark: 'O' } };
  const groups = modelGroups({ models: [model('configured-official-model')], profiles: [official], activeProfile: official, source: { ...official.modelSource, available: true }, connectionId: official.id, agent: 'codex', value: 'configured-official-model', order: { codex: ['@official', official.id] } });
  expect(groups.map(g => g.id)).toEqual(['@official']);
  expect(groups[0].source.connectionName).toBe('ChatGPT 官方账号');
  expect(groups[0].options[0]).toMatchObject({ connectionId: official.id, model: 'configured-official-model', selected: true });
});

it('unifies a legacy local official conversation and imported model aliases without losing its selection', () => {
  const official = { ...profile('imported', 'official-model'), agent: 'codex', officialAccount: true, models: ['official-model'] };
  const groups = modelGroups({ models: [model('official-model')], profiles: [official], source: { providerId: 'openai', providerName: 'OpenAI', connectionName: 'ChatGPT 官方账号', mark: 'O', available: true }, connectionId: '@local', agent: 'codex', value: 'official-model', order: { codex: ['@local', 'imported', '@official'] } });
  expect(groups.map(g => g.id)).toEqual(['@official']);
  expect(groups[0].options[0]).toMatchObject({ connectionId: '@local', selected: true });
});

it('keeps official model IDs in the single login group and distinct API connections selectable', () => {
  const first = { ...profile('imported-a', 'model-a'), agent: 'codex', officialAccount: true, models: ['model-a'] };
  const second = { ...first, id: 'imported-b', model: 'model-b', models: ['model-a', 'model-b'] };
  const api = { ...first, id: 'api', name: 'API', officialAccount: false, hasCredential: true };
  const groups = modelGroups({ models: [], profiles: [first, second, api], loggedInAgents: ['codex'], agent: 'codex', value: 'model-b', connectionId: '@official', order: { codex: ['api', '@official', 'imported-a', 'imported-b'] } });
  expect(groups.map(g => g.id)).toEqual(['api', '@official']);
  expect(groups[1].options.map(o => o.model)).toEqual(['model-a', 'model-b']);
  expect(groups[1].options[1]).toMatchObject({ connectionId: '@official', selected: true });
});

it.each(['claude', 'codex', 'opencode', 'pi'])('does not turn stale catalogs or session model IDs into an unconfigured %s connection', agent => {
  for (const connectionId of ['@local', 'removed', '@official']) {
    expect(modelGroups({ models: [model('k3[1M]')], profiles: [], source: { ...kimi, available: false }, agent, value: 'k3[1M]', connectionId, order: { [agent]: [connectionId] } })).toEqual([]);
  }
});

it('hides saved official models until login is verified, while leaving configured APIs selectable', () => {
  const official = { ...profile('official', 'saved-official'), officialAccount: true };
  const api = profile('api');
  expect(modelGroups({ models: [model('saved-official')], profiles: [official, api], activeProfile: official, agent: 'claude', value: 'saved-official', connectionId: official.id }).map(g => g.id)).toEqual(['api']);
  expect(modelGroups({ models: [], profiles: [official, api], activeProfile: official, source: { ...kimi, available: false }, loggedInAgents: ['claude'], agent: 'claude', value: 'saved-official', connectionId: official.id }).map(g => g.id)).toEqual(['api']);
});
