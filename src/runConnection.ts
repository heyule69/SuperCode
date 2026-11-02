import type { Message, RpcEvent } from './types';

export interface RunConnection { status: 'retrying' | 'restored' | 'failed' | 'stopped' | 'disconnected'; at: number; kind: 'network' | 'rateLimit' | 'server' | 'auth' | 'billing' | 'unknown'; attempt?: number; maxAttempts?: number; retryAt?: number }
const statuses = ['retrying', 'restored', 'failed', 'stopped', 'disconnected'];
const kinds = ['network', 'rateLimit', 'server', 'auth', 'billing', 'unknown'];
const positive = (n: unknown): n is number => typeof n === 'number' && Number.isFinite(n) && n > 0;
export function readRunConnection(value: unknown): RunConnection | undefined {
  if (!value || typeof value !== 'object') return;
  const raw = value as Record<string, unknown>;
  if (!statuses.includes(String(raw.status)) || !positive(raw.at)) return;
  const state: RunConnection = { status: raw.status as RunConnection['status'], at: raw.at, kind: kinds.includes(String(raw.kind)) ? raw.kind as RunConnection['kind'] : 'unknown' };
  for (const key of ['attempt', 'maxAttempts'] as const) if (positive(raw[key]) && Number.isInteger(raw[key]) && raw[key] <= 100000) state[key] = raw[key];
  if (positive(raw.retryAt) && raw.retryAt >= state.at && raw.retryAt - state.at <= 86400000) state.retryAt = raw.retryAt;
  return state;
}

export function connectionProgress(event: RpcEvent) {
  const p = event.params;
  if (event.method.endsWith('/delta') || event.method.endsWith('Delta')) return typeof p.delta === 'string' && p.delta.length > 0;
  if (['item/started', 'item/updated', 'item/completed'].includes(event.method)) return !!p.item?.id && p.item.type !== 'userMessage';
  return event.method === 'item/mcpToolCall/progress' && typeof p.message === 'string' && !!p.message;
}
function modelResponse(event: RpcEvent) {
  const p = event.params;
  return ['item/agentMessage/delta', 'item/plan/delta', 'item/reasoning/textDelta', 'item/reasoning/summaryTextDelta'].includes(event.method) && connectionProgress(event)
    || event.method === 'item/started' && ['commandExecution', 'claudeToolCall', 'mcpToolCall', 'dynamicToolCall', 'fileChange', 'webSearch'].includes(p.item?.type)
    || ['item/completed', 'item/updated'].includes(event.method) && ['agentMessage', 'plan', 'reasoning'].includes(p.item?.type)
      && ['text', 'summary', 'content'].some(key => typeof p.item[key] === 'string' && p.item[key].length > 0 || Array.isArray(p.item[key]) && p.item[key].some((value: unknown) => typeof value === 'string' ? value.length > 0 : !!value && typeof value === 'object' && typeof (value as { text?: unknown }).text === 'string' && !!(value as { text: string }).text.length));
}
export function finishConnection(connection: RunConnection | undefined, status: string, at: number): RunConnection | undefined {
  if (connection?.status !== 'retrying' && !(status === 'interrupted' && connection?.status === 'failed')) return connection;
  const { retryAt: _, ...state } = connection;
  return { ...state, status: status === 'completed' ? 'restored' : status === 'failed' ? 'failed' : 'stopped', at };
}

export function updateRunConnection(messages: Message[], event: RpcEvent, sessionId: string, now = Date.now()): Message[] {
  const p = event.params, turnId = p.turnId ?? p.turn?.id;
  if (typeof turnId !== 'string' || !turnId || event.id !== undefined) return messages;
  const marker = messages.find(m => m.sessionId === sessionId && m.kind === 'runMarker' && m.data?.turnId === turnId);
  const context = marker?.data ?? messages.find(m => m.sessionId === sessionId && m.historyRun?.data?.turnId === turnId)?.historyRun?.data;
  if (!context || ['completed', 'failed', 'interrupted'].includes(String(context.status))) return messages;
  const connection = readRunConnection(context.connection);
  let next = connection, progressAt = context.lastProgressAt;
  if (event.method === 'turn/connection/updated') {
    next = readRunConnection(p.connection);
    if (!next || connection && next.at < connection.at) return messages;
  } else if (event.method === 'error' && p.willRetry === true) next = { status:'retrying', at:now, kind:'unknown' };
  else if (event.method === 'turn/completed') next = finishConnection(connection, String(p.turn.status ?? 'failed'), now);
  else if (event.method === 'error' && p.willRetry !== true) next = finishConnection(connection, 'failed', now);
  else if (connectionProgress(event)) {
    // Pagination context stays read-only for ordinary item updates. Only an
    // observed connection transition needs to update that fallback marker.
    if (!marker && !connection) return messages;
    progressAt = now;
    if (connection && ['retrying', 'failed'].includes(connection.status) && modelResponse(event)) next = { ...connection, status:'restored', at:now, retryAt:undefined };
  }
  if (next === connection && progressAt === context.lastProgressAt) return messages;
  const data = { ...context, ...(next ? { connection:next } : {}), ...(progressAt ? { lastProgressAt:progressAt } : {}) };
  return messages.map(m => m === marker ? { ...m, data } : m.historyRun?.data?.turnId === turnId && m.sessionId === sessionId ? { ...m, historyRun:{ ...m.historyRun, data } } : m);
}
