import { expect, it } from 'vitest';
import { planText } from './planPresentation';

it('decodes streamed plan text without exposing JSON, escapes or incomplete Unicode', () => {
  const plan = '# 计划\n\n路径 C:\\test，"实现"。\n\n最后验收。';
  const encoded = JSON.stringify({ plan, allowedPrompts:[] });
  for (let i = 0; i < encoded.length; i++) {
    const draft = planText(encoded.slice(0, i), true);
    expect(plan.startsWith(draft)).toBe(true);
    expect(draft).not.toContain('"plan":');
  }
  expect(planText('{"plan":"第一阶段\\u4e', true)).toBe('第一阶段');
  expect(planText('{"plan":"第一阶段\\u4e8c', true)).toBe('第一阶段二');
  expect(planText(encoded, true)).toBe(planText(encoded));
  for (const invalid of ['{"plan":"未完成', '{"plan":true', '{"prompt":"文字', '{broken json']) {
    expect(planText(invalid)).toBe('');
  }
});
