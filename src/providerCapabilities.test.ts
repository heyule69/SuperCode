import { expect, it } from 'vitest';
import catalog from '../resources/providers.json';
import { providersForAgent } from './providerCapabilities';
import type { Provider } from './types';

const providers = catalog as Provider[];

it('offers documented Codex Responses vendors without treating chat-only APIs as compatible', () => {
  const codex = providersForAgent(providers, 'codex');
  expect(codex.map(p => p.id)).toEqual(expect.arrayContaining([
    'glm', 'zai', 'deepseek', 'kimi', 'minimax', 'qwen', 'volc', 'tencent', 'baidu',
    'openrouter', 'groq', 'ollama', 'lmstudio', 'openai', 'xai', 'custom',
  ]));
  expect(codex.every(p => p.presets.every(preset => preset.protocol === 'responses'))).toBe(true);
  expect(codex.map(p => p.id)).not.toEqual(expect.arrayContaining(['anthropic', 'google', 'siliconflow']));
  const qwen = codex.find(p => p.id === 'qwen')!;
  expect(qwen.presets.map(p => p.id)).toEqual(['token-responses']);
  expect(qwen.presets[0].baseUrl).toBe('https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1');
});

it('keeps Claude presets on Anthropic and preserves other engines’ protocol choices', () => {
  const claude = providersForAgent(providers, 'claude');
  expect(claude.every(p => p.presets.every(preset => preset.protocol === 'anthropic'))).toBe(true);
  expect(claude.find(p => p.id === 'glm')!.presets[0].baseUrl).toBe('https://open.bigmodel.cn/api/anthropic');
  expect(claude.find(p => p.id === 'deepseek')!.presets[0].baseUrl).toBe('https://api.deepseek.com/anthropic');
  for (const agent of ['opencode', 'pi']) {
    expect(providersForAgent(providers, agent).find(p => p.id === 'custom')!.presets.map(p => p.protocol)).toEqual(['anthropic', 'chat', 'responses']);
  }
});
