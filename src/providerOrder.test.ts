import { expect, it } from 'vitest';
import { connectionIds, mergeVisibleOrder, moveConnection, visibleProviderIds } from './providerOrder';
import type { AgentProfile } from './types';
it('keeps native OpenCode and Pi routes, orders their providers, and never invents an official account', () => {
  for (const agent of ['opencode', 'pi']) {
    expect(connectionIds(agent, [], [], {})).toEqual(['@local']);
    const own = { id: 'api', agent, name: 'API', model: 'm', current: true, hasCredential: true };
    expect(connectionIds(agent, [own], [], { [agent]: ['api', '@official', '@local'] })).toEqual(['api', '@local']);
    expect(connectionIds(agent, [own], [], { [agent]: ['@local', 'api'] })).toEqual(['@local', 'api']);
  }
});

const profiles: AgentProfile[] = [
  { id: 'kimi', agent: 'claude', name: 'Kimi', model: 'k3', current: true, hasCredential: true },
  { id: 'glm', agent: 'claude', name: '智谱', model: 'glm-5.3', current: false, hasCredential: true },
  { id: 'openai', agent: 'codex', name: 'OpenAI', model: 'configured-model', current: true, hasCredential: false },
];

it('uses saved engine-specific order including native accounts and ignores removed or cross-engine entries', () => {
  expect(connectionIds('claude', profiles, [], { claude: ['glm', '@official', 'deleted', 'glm', 'kimi', 'openai'] })).toEqual(['glm', '@official', 'kimi']);
  expect(connectionIds('codex', profiles, [], { codex: ['openai', '@official'] })).toEqual(['openai', '@official']);
  expect(connectionIds('claude', [], [], {})).toEqual(['@local', '@official']);
  expect(connectionIds('claude', [], ['claude'], {})).toEqual(['@official']);
});

it('moves complete routes in either direction without mutating the original or losing hidden entries', () => {
  const ids = ['kimi', 'glm', '@official', '@local'];
  expect(moveConnection(ids, 'glm', 'kimi')).toEqual(['glm', 'kimi', '@official', '@local']);
  expect(moveConnection(ids, 'kimi', '@official', 'after')).toEqual(['glm', '@official', 'kimi', '@local']);
  expect(moveConnection(ids, 'kimi', 'glm', 'after')).toEqual(['glm', 'kimi', '@official', '@local']);
  expect(moveConnection(ids, 'missing', 'kimi')).toBe(ids);
  expect(moveConnection(ids, 'kimi', 'kimi')).toBe(ids);
  expect(ids).toEqual(['kimi', 'glm', '@official', '@local']);
});

it('shows one current Codex login instead of CLI, imported official and native official aliases', () => {
  const official = { ...profiles[2], officialAccount: true };
  expect(connectionIds('codex', [official], [], { codex: ['@local', official.id, '@official'] })).toEqual(['@official']);
  expect(connectionIds('codex', [], [], {})).toEqual(['@official']);
  expect(connectionIds('codex', [official], [], {})).toEqual(['@official']);
});

it('preserves distinct API connections and the official login position in saved order', () => {
  const official = { ...profiles[2], id: 'imported', officialAccount: true };
  const api = { ...profiles[2], id: 'api', officialAccount: false, hasCredential: true };
  expect(connectionIds('codex', [official, api], [], { codex: ['api', 'imported', '@local', '@official'] })).toEqual(['api', '@official']);
});

it('shows configured suppliers and confirmed accounts instead of CLI or unconfigured official placeholders', () => {
  for (const agent of ['claude', 'codex', 'opencode', 'pi']) {
    expect(visibleProviderIds(agent, [], [], { [agent]: ['@local', '@official'] })).toEqual([]);
  }
  expect(visibleProviderIds('claude', profiles, [], { claude: ['@official', 'glm', '@local', 'kimi'] })).toEqual(['glm', 'kimi']);
  expect(visibleProviderIds('claude', profiles, ['claude'], { claude: ['glm', '@official', 'kimi'] })).toEqual(['glm', '@official', 'kimi']);
  const official = { ...profiles[2], officialAccount: true };
  expect(visibleProviderIds('codex', [official], [], {})).toEqual([]);
  expect(visibleProviderIds('codex', [official], ['codex'], {})).toEqual(['@official']);
});

it('promotes the chosen visible default while keeping hidden legacy routes in the persisted order', () => {
  const complete = ['@local', 'kimi', '@official', 'glm'];
  expect(mergeVisibleOrder(['glm', 'kimi'], complete)).toEqual(['glm', 'kimi', '@local', '@official']);
  expect(complete).toEqual(['@local', 'kimi', '@official', 'glm']);
});
