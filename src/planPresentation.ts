import type { RpcEvent } from './types';
export type PlanDecision = 'execute' | 'revise';

export function planFollowup(decision: PlanDecision, feedback: string) {
  return decision === 'execute' ? '请按上一条已确认的计划开始执行。'
    : `请继续修改上一条计划，保持计划模式，暂不执行。${feedback.trim() ? `\n\n修改意见：\n${feedback.trim()}` : ''}`;
}

export function isPlanRequest(request: RpcEvent) {
  return request.method === 'claude/tool/requestApproval' && request.params.toolName === 'ExitPlanMode';
}

export function planText(input: unknown, streaming = false): string {
  if (typeof input === 'string') {
    try { return planText(JSON.parse(input)); } catch {
      if (!streaming) return '';
      // Decode only the plan string, holding incomplete escapes until the next
      // delta arrives. Never expose raw tool arguments in the document.
      const start = /(?:\{|,)\s*"plan"\s*:\s*"/g.exec(input);
      if (!start) return '';
      const value = input.slice(start.index + start[0].length);
      let end = 0;
      while (end < value.length) {
        if (value[end] === '"') break;
        if (value[end] === '\\') {
          const length = value[end + 1] === 'u' ? 6 : 2;
          if (end + length > value.length) break;
          end += length;
        } else end++;
      }
      try { return JSON.parse(`"${value.slice(0, end)}"`).trimStart(); } catch { return ''; }
    }
  }
  if (!input || typeof input !== 'object' || Array.isArray(input)) return '';
  const value = (input as Record<string, unknown>).plan;
  return typeof value === 'string' ? value.trim() : '';
}
