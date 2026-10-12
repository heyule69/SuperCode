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
  expect(modelContextWindow('glm-5.3', model(32768))).toBe(32768);
  expect(modelContextWindow('unknown', model(-1))).toBeUndefined();
});
it('formats the compact token lines used by the reference tooltip', () => {
  expect(contextNumber(166000)).toBe('166k');
  expect(contextNumber(475000)).toBe('475k');
  expect(contextNumber(1000000)).toBe('1m');
  expect(contextNumber(0)).toBe('0');
});
