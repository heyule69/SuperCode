import type { RpcEvent } from './types';

export function isPlanRequest(request: RpcEvent) {
  return request.method === 'claude/tool/requestApproval' && request.params.toolName === 'ExitPlanMode';
}

export function planText(input: unknown): string {
  if (typeof input === 'string') {
    try { return planText(JSON.parse(input)); } catch { return ''; }
  }
  if (!input || typeof input !== 'object' || Array.isArray(input)) return '';
  const value = (input as Record<string, unknown>).plan;
  return typeof value === 'string' ? value.trim() : '';
}
