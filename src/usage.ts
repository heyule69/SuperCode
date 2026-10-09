export interface TokenBreakdown { inputTokens?: number; cachedInputTokens?: number; cacheWriteInputTokens?: number; outputTokens?: number; reasoningOutputTokens?: number; totalTokens?: number }
export interface TokenUsage { total: TokenBreakdown; last: TokenBreakdown; turn?: TokenBreakdown; contextTokens?: number | null; modelContextWindow?: number | null; costUsd?: number | null; cumulative?: boolean }
export interface UsageRecord { sessionId: string; turnId: string; agent: string; model: string; data: TokenUsage; at: number; title: string }
export const number = (n?: number | null) => n == null ? '—' : Intl.NumberFormat('zh-CN').format(n);
export const compactNumber = (n: number) => n >= 1_000_000 ? `${(n / 1_000_000).toFixed(2)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}K` : number(n);
export function usageTotals(records: UsageRecord[]) {
  return records.reduce((sum, record) => {
    const t = record.data.turn ?? record.data.total;
    sum.input += t.inputTokens ?? 0; sum.output += t.outputTokens ?? 0;
    sum.cached += t.cachedInputTokens ?? 0;
    sum.tokens += t.totalTokens ?? (t.inputTokens ?? 0) + (t.outputTokens ?? 0);
    if (record.data.costUsd != null) { sum.cost += record.data.costUsd; sum.priced++; }
    return sum;
  }, { input: 0, output: 0, cached: 0, tokens: 0, cost: 0, priced: 0 });
}
export function usageDay(at: number) {
  const date = new Date(at * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}
export function usageGroups(records: UsageRecord[], group: 'model' | 'chat') {
  const groups = new Map<string, { id: string; title: string; agent: string; models: Set<string>; records: UsageRecord[]; at: number }>();
  for (const record of records) {
    const id = group === 'model' ? JSON.stringify([record.agent, record.model]) : record.sessionId;
    const current = groups.get(id) ?? { id, title: group === 'model' ? record.model || '默认模型' : record.title, agent: record.agent, models: new Set(), records: [], at: 0 };
    current.records.push(record); current.models.add(record.model || '默认模型'); current.at = Math.max(current.at, record.at);
    groups.set(id, current);
  }
  return [...groups.values()].sort((a, b) => b.at - a.at).map(group => ({ ...group, totals: usageTotals(group.records) }));
}
