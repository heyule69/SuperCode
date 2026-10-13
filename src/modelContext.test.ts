import { expect, it } from 'vitest';
import { contextNumber, modelContextWindow } from './modelContext';
const model = (contextWindow: number, effectiveContextWindowPercent?: number) => ({id:'m',model:'m',displayName:'m',isDefault:false,contextWindow,effectiveContextWindowPercent});
it('distinguishes Codex account capacity from API model specs and honors live catalog metadata', () => {
  expect(modelContextWindow('gpt-6.1-sol', undefined, true)).toBe(475000);
  expect(modelContextWindow('gpt-6.1-sol')).toBe(1050000);
  expect(modelContextWindow('gpt-6.1-sol', model(600000, 95), true)).toBe(570000);
  expect(modelContextWindow('future-private', model(32768))).toBe(32768);
});
it('resolves exact models and explicit long-context variants without guessing unknown routers', () => {
  expect(modelContextWindow('glm-5.3')).toBe(1000000);
  expect(modelContextWindow('glm-5.3-flashx')).toBe(1000000);
  expect(modelContextWindow('glm-5-turbo')).toBe(200000);
  expect(modelContextWindow('k3')).toBe(262144);
  expect(modelContextWindow('k3[1m]')).toBe(1048576);
  expect(modelContextWindow('k3-256k')).toBe(262144);
  expect(modelContextWindow('MiniMax-M2.7')).toBe(204800);
  expect(modelContextWindow('auto')).toBeUndefined();
  expect(modelContextWindow('unknown')).toBeUndefined();
  expect(modelContextWindow('private[1m]')).toBeUndefined();
  expect(modelContextWindow('glm-5-turbo[1m]')).toBe(200000);
  expect(modelContextWindow('glm-5-turbo[256k]')).toBe(200000);
  expect(modelContextWindow('glm-5.3', model(32768))).toBe(32768);
  expect(modelContextWindow('glm-5.3[1m]', model(32768))).toBe(32768);
  expect(modelContextWindow('unknown', model(-1))).toBeUndefined();
});
it('formats the compact token lines used by the reference tooltip', () => {
  expect(contextNumber(166000)).toBe('166k');
  expect(contextNumber(475000)).toBe('475k');
  expect(contextNumber(1000000)).toBe('1m');
  expect(contextNumber(0)).toBe('0');
});
it('covers preset model families, keeps coding-plan variants separate, and honors hosted limits', () => {
  for (const [id, window] of [
    ['kimi-k3', 1048576], ['kimi-k2.7-code', 262144],
    ['qwen3.8-max', 1000000], ['qwen3.8-flash', 1000000],
    ['deepseek-v4-pro', 1048576], ['deepseek-flash', 1048576],
    ['deepseek-ai/DeepSeek-V4-Flash', 1048576],
    ['gemini-3.8-flash', 1048576], ['grok-4.7', 500000],
    ['openai/gpt-oss-20b', 131072], ['openai/gpt-oss-120b', 131072],
    ['hy3', 262144], ['MiniMaxAI/MiniMax-M3', 1000000],
  ] as const) expect(modelContextWindow(id), id).toBe(window);
  expect(modelContextWindow('k3')).toBe(262144);
  expect(modelContextWindow('MiniMax-M3', model(192000))).toBe(192000);
  for (const id of ['auto', 'ark-code-latest', 'tc-code-latest', 'private-router']) {
    expect(modelContextWindow(id), id).toBeUndefined();
    expect(modelContextWindow(id, model(64000)), id).toBe(64000);
  }
});
