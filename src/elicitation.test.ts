import { expect, it } from 'vitest';
import { formContent, supportedForm } from './elicitation';
it('preserves false and converts explicit numeric answers without returning extra fields', () => {
  expect(formContent({ properties: { allowed: { type: 'boolean' }, count: { type: 'integer', minimum: 1, maximum: 4 } }, required: ['allowed', 'count'] }, { allowed: false, count: '2', hidden: 'ignored' })).toEqual({ allowed: false, count: 2 });
});
it('rejects missing answers, invalid enum options, and unsupported nested forms', () => {
  expect(() => formContent({ properties: { text: { type: 'string' } }, required: ['text'] }, { text: '' })).toThrow('请填写');
  expect(() => formContent({ properties: { option: { type: 'string', enum: ['yes', 'no'] } } }, { option: 'unknown' })).toThrow('选项');
  expect(supportedForm({ properties: { complex: { type: 'object' } } })).toBe(false);
});
it('validates multi-select and integer bounds', () => {
  const schema = { properties: { tools: { type: 'array', items: { type: 'string', enum: ['a', 'b'] }, minItems: 1 }, number: { type: 'integer', maximum: 2 } } };
  expect(formContent(schema, { tools: ['a'], number: '2' })).toEqual({ tools: ['a'], number: 2 });
  expect(() => formContent(schema, { tools: [], number: '2' })).toThrow('数量');
  expect(() => formContent(schema, { tools: ['a'], number: '2.1' })).toThrow('整数');
});
