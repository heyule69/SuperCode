import type { Model, ModelSource } from './types';

export const effortLabels: Record<string, string> = {
  none: '关闭', off: '关闭', minimal: '最低', low: '低', medium: '中', high: '高',
  xhigh: '极高', max: '最大', ultra: 'Ultra',
};

// Verified 2026-11-24 against Claude Code's per-model effort table:
// https://code.claude.com/docs/en/model-config#adjust-effort-level
function claudeEfforts(model: Model, source?: ModelSource): string[] | undefined {
  const id = model.model.split('/').at(-1)!.toLowerCase().replace(/\[1m\]$/, '');
  if (source?.providerId === 'anthropic' && ['opus', 'sonnet', 'haiku', 'fable'].includes(id)) return ['low', 'medium', 'high', 'xhigh', 'max'];
  if (/^claude-(opus|sonnet)-4-6(?:-\d{8})?$/.test(id)) return ['low', 'medium', 'high', 'max'];
  if (/^claude-(opus-(?:4-(?:7|8)|5(?:-5)?)|sonnet-5(?:-5)?|haiku-5-5|(?:fable|mythos)-5(?:-[12])?|mythos-preview)(?:-\d{8})?$/.test(id)) return ['low', 'medium', 'high', 'xhigh', 'max'];
  if (/^claude-(opus|sonnet|haiku)-(?:3|4)(?:-|$)/.test(id)) return [];
}

export function modelReasoningEfforts(agent: string, model?: Model, source?: ModelSource): string[] {
  if (!model) return [];
  // These ACP adapters do not currently send an effort/config-option request.
  if (agent === 'opencode' || agent === 'gemini') return [];
  const declared = model.supportedReasoningEfforts?.map(level => level.reasoningEffort);
  const published = agent === 'claude' ? claudeEfforts(model, source) : undefined;
  const transport = agent === 'claude' ? ['low', 'medium', 'high', 'xhigh', 'max']
    : agent === 'pi' ? ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max']
    : agent === 'codex' ? ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'] : [];
  return [...new Set(declared ?? published ?? [])].filter(level => transport.includes(level) && (published === undefined || published.includes(level)));
}

export function modelReasoningKnown(agent: string, model: Model, source?: ModelSource): boolean {
  return ['opencode', 'gemini'].includes(agent) || model.supportedReasoningEfforts !== undefined
    || agent === 'claude' && claudeEfforts(model, source) !== undefined;
}

// The official desktop picker filters model capabilities through its enabled
// efforts (enabled-reasoning-efforts in desktop 26.1117.4957.0, checked 2026-11-25).
// Max is supported by some models but is not enabled by default.
// Preserve an existing advanced selection until the user changes or resets it.
export function visibleCodexEfforts(supported: string[], current = ''): string[] {
  const enabled = ['low', 'medium', 'high', 'xhigh', 'ultra'];
  return supported.filter(level => enabled.includes(level) || level === current);
}
