export interface ElicitationField { type?: string; title?: string; description?: string; default?: unknown; enum?: string[]; enumNames?: string[]; oneOf?: { const: string; title?: string }[]; items?: ElicitationField; minLength?: number; maxLength?: number; minimum?: number; maximum?: number; minItems?: number; maxItems?: number; format?: string }
export interface ElicitationSchema { properties?: Record<string, ElicitationField>; required?: string[] }
export function fieldOptions(field: ElicitationField) { return field.oneOf?.map(o => ({ value: o.const, label: o.title ?? o.const })) ?? field.enum?.map((value, i) => ({ value, label: field.enumNames?.[i] ?? value })) ?? []; }
export function supportedForm(schema: ElicitationSchema) { const fields = Object.values(schema.properties ?? {}); return fields.length <= 32 && fields.every(f => ['string', 'number', 'integer', 'boolean'].includes(f.type ?? '') || f.type === 'array' && fieldOptions(f.items ?? {}).length > 0); }
export function formDefaults(schema: ElicitationSchema) { return Object.fromEntries(Object.entries(schema.properties ?? {}).map(([name, f]) => [name, f.default ?? (f.type === 'boolean' ? false : f.type === 'array' ? [] : '')])); }
export function formContent(schema: ElicitationSchema, values: Record<string, unknown>) {
  if (!supportedForm(schema)) throw new Error('此表单包含暂不支持的字段');
  const content: Record<string, unknown> = {};
  for (const [name, field] of Object.entries(schema.properties ?? {})) {
    let value = values[name]; const required = schema.required?.includes(name); const label = field.title ?? name;
    if (value == null || value === '') { if (required) throw new Error(`请填写${label}`); continue; }
    if (field.type === 'number' || field.type === 'integer') { value = Number(value); if (!Number.isFinite(value) || field.type === 'integer' && !Number.isInteger(value)) throw new Error(`${label}必须为${field.type === 'integer' ? '整数' : '数字'}`); if (field.minimum != null && (value as number) < field.minimum || field.maximum != null && (value as number) > field.maximum) throw new Error(`${label}超出允许范围`); }
    else if (field.type === 'boolean') { if (typeof value !== 'boolean') throw new Error(`${label}的选项无效`); }
    else if (field.type === 'array') { const options = fieldOptions(field.items ?? {}); if (!Array.isArray(value) || value.some(v => !options.some(o => o.value === v))) throw new Error(`${label}的选项无效`); if (required && !value.length || field.minItems != null && value.length < field.minItems || field.maxItems != null && value.length > field.maxItems) throw new Error(`请检查${label}的选择数量`); }
    else { value = String(value); const options = fieldOptions(field); if (options.length && !options.some(o => o.value === value)) throw new Error(`${label}的选项无效`); if (field.minLength != null && (value as string).length < field.minLength || field.maxLength != null && (value as string).length > field.maxLength) throw new Error(`${label}的长度不符合要求`); }
    content[name] = value;
  }
  return content;
}
