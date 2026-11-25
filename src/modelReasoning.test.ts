import { expect, it } from 'vitest';
import { modelReasoningEfforts } from './modelReasoning';
import type { Model } from './types';

const model = (id: string, levels?: string[]): Model => ({ id, model: id, displayName: id, isDefault: true,
  ...(levels === undefined ? {} : { supportedReasoningEfforts: levels.map(reasoningEffort => ({ reasoningEffort })) }) });

it('keeps advertised Codex levels: Ultra is available on Sol but not Luna', () => {
  expect(modelReasoningEfforts('codex', model('gpt-6.2-sol', ['low', 'medium', 'high', 'xhigh', 'max', 'ultra']))).toEqual(['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
  expect(modelReasoningEfforts('codex', model('gpt-6-luna', ['low', 'medium', 'high', 'xhigh', 'max']))).not.toContain('ultra');
  expect(modelReasoningEfforts('codex', model('custom'))).toEqual([]);
});
it.each(['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5', 'claude-opus-4-7', 'claude-fable-5-2'])('uses the verified five-level Claude table for %s', id => {
  expect(modelReasoningEfforts('claude', model(id))).toEqual(['low', 'medium', 'high', 'xhigh', 'max']);
});
it.each(['claude-opus-4-6', 'claude-sonnet-4-6-20260217', 'claude-opus-4-6[1m]'])('does not invent xhigh or Ultra on %s', id => {
  expect(modelReasoningEfforts('claude', model(id))).toEqual(['low', 'medium', 'high', 'max']);
});
it.each(['claude-haiku-4-5', 'claude-sonnet-4-5', 'proxy-model', 'claude-opus-5-6'])('does not add generic Claude levels to %s', id => {
  expect(modelReasoningEfforts('claude', model(id))).toEqual([]);
});
it('respects explicit unavailable and restricted provider capabilities', () => {
  expect(modelReasoningEfforts('claude', model('claude-opus-5-5', []))).toEqual([]);
  expect(modelReasoningEfforts('claude', model('claude-sonnet-4-6', ['low', 'xhigh', 'max', 'ultra']))).toEqual(['low', 'max']);
  expect(modelReasoningEfforts('claude', model('proxy', ['low', 'high', 'ultra']))).toEqual(['low', 'high']);
});
it.each(['gemini', 'opencode'])('hides an effort control the %s adapter does not transmit', agent => {
  expect(modelReasoningEfforts(agent, model('reasoner', ['low', 'high', 'ultra']))).toEqual([]);
});
it('only exposes Pi levels supported by both the model and its RPC', () => {
  expect(modelReasoningEfforts('pi', model('model', ['off', 'high', 'xhigh', 'ultra']))).toEqual(['off', 'high', 'xhigh']);
});
