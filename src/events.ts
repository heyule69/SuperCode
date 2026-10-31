import type { Message, RpcEvent } from './types';

const limit = 128 * 1024;
const activityKinds = new Set(['reasoning', 'executionPlan', 'runMarker']);
const toolKinds = new Set(['commandExecution', 'fileChange', 'mcpToolCall', 'dynamicToolCall', 'claudeToolCall', 'webSearch', 'imageView', 'collabAgentToolCall', 'collabToolCall', 'contextCompaction', 'enteredReviewMode', 'exitedReviewMode']);
export const isActivity = (m: Message) => !['runMarker','turnArtifacts'].includes(m.kind) && (m.role === 'tool' || m.role === 'activity');
function cut(text: string, max: number) { const end = Math.min(text.length, max); return text.slice(0, end > 0 && /[\uD800-\uDBFF]/.test(text[end - 1]) ? end - 1 : end); }
const bounded = (text: string) => cut(text, limit);
const parts = (value: unknown) => Array.isArray(value) ? value.filter(v => typeof v === 'string').join('\n\n') : '';

function metadata(value: Record<string, any>): Record<string, any> {
  let room = limit;
  function trim(v: any, depth: number): any {
    if (depth > 12 || room <= 0) return '… 详情过长，已截断';
    if (typeof v === 'string') { const max = Math.min(32 * 1024, room); const text = cut(v, max); room -= text.length; return v.length > text.length ? `${text}\n… 详情过长，已截断` : text; }
    if (Array.isArray(v)) return v.slice(0, 100).map(item => trim(item, depth + 1));
    if (v && typeof v === 'object') {
      if (['image', 'audio', 'image_url'].includes(v.type)) return { type: 'attachment', note: '附件二进制未加载' };
      return Object.fromEntries(Object.entries(v).filter(([key]) => !['signature', 'encryptedContent', 'encrypted_content'].includes(key)).slice(0, 100).map(([key, item]) => [key, trim(item, depth + 1)]));
    }
    return v;
  }
  // Keep screenshots even when a tool has very large arguments or diagnostics.
  const { media, changeSummary, ...details } = value;
  room = 16 * 1024;
  const mediaLimited = Array.isArray(media) ? trim(media.slice(0, 12), 0) : undefined;
  const mediaUsed = 16 * 1024 - room;
  room = 16 * 1024;
  const summaryLimited = Array.isArray(changeSummary) ? trim(changeSummary.slice(0, 32), 0) : undefined;
  room += limit - 16 * 1024 - mediaUsed;
  const result = trim(details, 0);
  if (mediaLimited) result.media = mediaLimited;
  if (summaryLimited) result.changeSummary = summaryLimited;
  for (const key of ['id', 'type', 'status', 'tool', 'server', 'turnId', 'phase', 'messageId', 'startedAt', 'durationMs', 'exitCode', 'success']) if (key in value) result[key] = typeof value[key] === 'string' ? cut(value[key], 4096) : value[key];
  return result;
}

function upsert(messages: Message[], message: Message): Message[] {
  const index = messages.findIndex(m => m.id === message.id);
  // A native item update does not contain the page's read-only turn context.
  // Retain it until an earlier page supplies the actual marker/user boundary.
  return index < 0 ? [...messages, message] : messages.map((m, i) => i === index ? { ...message, historyRun: message.historyRun ?? m.historyRun } : m);
}

export function isDisplayEvent(event: RpcEvent) {
  return event.id === undefined && (event.method.startsWith('item/') || ['turn/started', 'turn/completed', 'turn/plan/updated', 'turn/diff/updated', 'turn/artifacts/updated'].includes(event.method));
}

// A storage read may race native startup. Keep unacknowledged messages in their
// original position, and match acknowledgements by identity, never by text.
export function reconcileMessages(live: Message[], saved: Message[], pendingUsers: ReadonlySet<string>): Message[] {
  const liveById = new Map(live.map(message => [message.id, message]));
  const indices = new Map(saved.map((message, index) => [message.id, index]));
  const savedTurns = new Set(saved.map(message => message.data?.turnId).filter(Boolean));
  const acknowledgements = new Map(saved.flatMap((message, index) =>
    message.role === 'user' && typeof message.data?.clientMessageId === 'string'
      ? [[message.data.clientMessageId, index] as const] : []));
  const anchors = live.map(message => indices.get(message.id) ?? (message.role === 'user' && typeof message.data?.clientMessageId === 'string' ? acknowledgements.get(message.data.clientMessageId) : undefined));
  const following = new Array<number>(live.length);
  let next = saved.length;
  for (let index = live.length - 1; index >= 0; index--) {
    if (anchors[index] !== undefined) next = anchors[index]!;
    following[index] = next;
  }
  const slots = new Map<number, Message[]>();
  let position = 0, anchored = false;
  for (const [liveIndex, message] of live.entries()) {
    const clientId = message.role === 'user' ? message.data?.clientMessageId : undefined;
    const index = anchors[liveIndex];
    if (index !== undefined) { position = Math.max(position, index + 1); anchored = true; continue; }
    // The latest page can start midway through a long turn. Keep its lifecycle
    // marker while that turn is still represented, without retaining old pages.
    const keepRun = message.kind === 'runMarker' && savedTurns.has(message.data?.turnId);
    if (message.seq !== 0 && !keepRun || message.role === 'user' && !(typeof clientId === 'string' && pendingUsers.has(clientId))) continue;
    // If history was still loading at submission, place the local message after
    // that history and before the next known live item, rather than at the top.
    if (!anchored) position = following[liveIndex];
    const slot = slots.get(position) ?? [];
    slot.push(message); slots.set(position, slot);
  }
  const result: Message[] = [];
  saved.forEach((message, index) => {
    result.push(...(slots.get(index) ?? []));
    const previous = liveById.get(message.id);
    // A history request begun before completion can return afterwards. Its
    // older running state must not undo an observed completion of the same run.
    if (previous?.kind === 'runMarker' && message.kind === 'runMarker'
      && previous.data?.turnId === message.data?.turnId
      && ['completed', 'failed', 'interrupted'].includes(String(previous.data?.status))
      && ['inProgress', 'running', 'preparing'].includes(String(message.data?.status))) {
      result.push({ ...previous, seq: message.seq });
      return;
    }
    result.push(previous && previous.text.length > message.text.length
      && ['inProgress', 'preparing'].includes(String(previous.data?.status))
      && ['inProgress', 'preparing'].includes(String(message.data?.status)) ? { ...previous, seq: message.seq } : message);
  });
  result.push(...(slots.get(saved.length) ?? []));
  return result;
}

export function mergeEvent(messages: Message[], event: RpcEvent, sessionId: string): Message[] {
  const p = event.params;
  if (event.method === 'turn/artifacts/updated') return upsert(messages, { seq:0,id:`artifacts-${p.turnId}`,sessionId,role:'activity',text:'',kind:'turnArtifacts',data:metadata({turnId:p.turnId,status:'completed',artifacts:p.artifacts,error:p.error}) });
  if (event.method === 'turn/diff/updated') return upsert(messages, { seq: 0, id: `diff-${p.turnId}`, sessionId, role: 'activity', text: bounded(String(p.diff ?? '')), kind: 'turnDiff', data: { turnId: p.turnId, status: 'completed' } });
  if (event.method === 'turn/started') {
    return upsert(messages, { seq: 0, id: `run-${p.turn.id}`, sessionId, role: 'activity', text: '', kind: 'runMarker', data: { status: 'inProgress', turnId: p.turn.id, startedAt: Date.now(), ...(p.turn.phase === 'awaitingResponse' ? { phase: 'awaitingResponse' } : {}) } });
  }
  if (event.method === 'turn/completed') {
    const turnId = p.turn?.id;
    if (typeof turnId !== 'string' || !turnId) return messages;
    const status = p.turn.status ?? 'failed';
    let marker = false, last = -1;
    const finished = messages.map((m, index) => {
      if (!m.data || m.data.turnId !== turnId) return m;
      last = index;
      if (m.kind === 'runMarker') marker = true;
      if (m.kind === 'runMarker') return { ...m, data: { ...m.data, status, durationMs: Math.max(0, Date.now() - Number(m.data.startedAt ?? Date.now())) } };
      if (['inProgress', 'running', 'preparing'].includes(String(m.data.status))) return { ...m, data: { ...m.data, status: status === 'failed' ? 'failed' : 'interrupted' } };
      return m;
    });
    // Restored/paginated history may not contain the start event. Record the
    // actual completion beside its own items, not at the end of a newer turn.
    if (!marker && last >= 0) finished.splice(last + 1, 0, { seq: 0, id: `run-${turnId}`, sessionId, role: 'activity', text: '', kind: 'runMarker', data: { turnId, status } });
    return finished;
  }
  if (event.method === 'turn/plan/updated') {
    const id = `plan-${p.turnId}`;
    const old = messages.find(m => m.id === id);
    return upsert(messages, { seq: old?.seq ?? 0, id, sessionId, role: 'activity', text: String(p.explanation ?? ''), kind: 'executionPlan', data: { status: 'inProgress', turnId: p.turnId, plan: p.plan ?? [] } });
  }
  const deltas: Record<string, { kind: string; role: string; field?: string }> = {
    'item/agentMessage/delta': { kind: 'agentMessage', role: 'assistant' },
    'item/plan/delta': { kind: 'plan', role: 'assistant' },
    'item/reasoning/summaryTextDelta': { kind: 'reasoning', role: 'activity', field: 'summary' },
    'item/reasoning/textDelta': { kind: 'reasoning', role: 'activity', field: 'content' },
    'item/commandExecution/outputDelta': { kind: 'commandExecution', role: 'tool', field: 'output' },
    'item/fileChange/outputDelta': { kind: 'fileChange', role: 'tool', field: 'output' },
    'item/claudeToolCall/inputDelta': { kind: 'claudeToolCall', role: 'tool', field: 'inputText' },
    'item/mcpToolCall/progress': { kind: 'mcpToolCall', role: 'tool', field: 'progress' },
    'item/commandExecution/terminalInteraction': { kind: 'commandExecution', role: 'tool', field: 'terminalInput' },
  };
  const config = deltas[event.method];
  if (config) {
    if (!p.itemId) return messages;
    const id = String(p.itemId);
    const old = messages.find(m => m.id === id);
    const m: Message = old ?? { seq: 0, id, sessionId, role: config.role, text: '', kind: config.kind, data: { status: 'inProgress', turnId: p.turnId } };
    const data = { ...m.data };
    let text = m.text;
    if (['summary', 'content'].includes(config.field ?? '')) {
      const key = config.field!;
      const index = Math.min(63, Math.max(0, Number(key === 'summary' ? p.summaryIndex : p.contentIndex) || 0));
      const sections = Array.isArray(data[key]) ? [...data[key] as string[]] : [];
      if (!sections.length && m.text && key === 'summary') sections[0] = m.text;
      const available = Math.max(0, limit - sections.reduce((sum, v) => sum + (v?.length ?? 0), 0));
      sections[index] = bounded((sections[index] ?? '') + cut(String(p.delta ?? ''), available));
      data[key] = sections;
      text = parts(data.summary) || parts(data.content);
    } else if (config.field === 'output' && config.kind === 'fileChange') data.operationOutput = bounded(String(data.operationOutput ?? '') + String(p.delta ?? ''));
    else if (config.field === 'output') text = bounded(m.text + String(p.delta ?? ''));
    else if (config.field) {
      const delta = config.field === 'progress' ? `${p.message ?? ''}\n` : config.field === 'terminalInput' ? p.stdin ?? '' : p.delta ?? '';
      data[config.field] = bounded(String(data[config.field] ?? '') + delta);
    } else text = bounded(text + String(p.delta ?? ''));
    return upsert(messages, { ...m, text: bounded(text), data });
  }
  if (!['item/started', 'item/updated', 'item/completed'].includes(event.method)) return messages;
  const item = p.item;
  if (!item?.id || !['agentMessage', 'plan'].includes(item.type) && !activityKinds.has(item.type) && !toolKinds.has(item.type)) return messages;
  const id = String(item.id);
  const existing = messages.find(m => m.id === id);
  const status = event.method === 'item/completed' && ['inProgress', 'running', 'preparing'].includes(String(item.status))
    ? 'completed' : item.status ?? (event.method === 'item/completed' ? 'completed' : existing?.data?.status ?? 'inProgress');
  const data = { ...existing?.data, ...item, turnId: p.turnId, status, startedAt: existing?.data?.startedAt ?? Date.now() };
  if (item.type === 'fileChange' && item.aggregatedOutput != null) data.operationOutput = item.aggregatedOutput;
  if (event.method === 'item/completed' && data.durationMs == null && existing?.data?.startedAt != null) data.durationMs = Math.max(0, Date.now() - Number(existing.data.startedAt));
  let text = item.type === 'commandExecution' ? `${item.command ?? ''}\n${item.aggregatedOutput ?? ''}`
    : item.type === 'claudeToolCall' ? item.output ?? existing?.text ?? ''
    : item.type === 'fileChange' ? (item.changes ?? []).map((c: { path: string }) => c.path).join('\n')
    : item.type === 'reasoning' ? parts(item.summary) || parts(item.content) || existing?.text || ''
    : ['mcpToolCall', 'dynamicToolCall'].includes(item.type) ? `${item.server ?? item.namespace ?? '工具'} / ${item.tool ?? '执行'}`
    : item.type === 'webSearch' ? item.query ?? '' : item.type === 'imageView' ? item.path ?? ''
    : item.text ?? item.prompt ?? item.review ?? existing?.text ?? '';
  if (item.type === 'commandExecution' && item.aggregatedOutput == null && existing?.text) text = existing.text;
  delete data.aggregatedOutput; delete data.output;
  if (event.method === 'item/completed') { delete data.summary; delete data.content; delete data.inputText; }
  return upsert(messages, { seq: existing?.seq ?? 0, id, sessionId, role: ['agentMessage', 'plan'].includes(item.type) ? 'assistant' : activityKinds.has(item.type) ? 'activity' : 'tool', text: bounded(String(text)), kind: item.type, data: metadata(data) });
}

export function initialAnswers(event: RpcEvent): Record<string, string> {
  return Object.fromEntries((event.params.questions ?? []).map((q: { id: string }) => [q.id, '']));
}

export interface ConversationTurn { id: string; user?: Message; items: Message[]; run?: Message }
export function conversationTurns(messages: Message[]): ConversationTurn[] {
  const turns: ConversationTurn[] = [];
  for (const m of messages) {
    if (m.kind === 'agentCapabilities') continue;
    if (m.role === 'user' || m.kind === 'modelSwitch' || !turns.length) turns.push({ id: m.role === 'user' && typeof m.data?.clientMessageId === 'string' ? m.data.clientMessageId : m.id, user: m.role === 'user' ? m : undefined, items: [] });
    const turn = turns[turns.length - 1];
    if (m.historyRun) {
      if (!turn.user) turn.id = m.historyRun.turnKey;
      if (!turn.run) turn.run = { seq: 0, id: m.historyRun.id, sessionId: m.sessionId, role: 'activity', text: '', kind: 'runMarker', data: m.historyRun.data };
    }
    if (m.kind === 'runMarker') turn.run = m;
    else if (m.role !== 'user') turn.items.push(m);
  }
  return turns;
}
