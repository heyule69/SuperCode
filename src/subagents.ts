import type { Message, RpcEvent } from './types';
import { mergeEvent } from './events';

export type SubagentStatus = 'running' | 'waiting' | 'completed' | 'failed' | 'interrupted' | 'unknown';
export interface Subagent {
  key: string; anchorId: string; name: string; status: SubagentStatus;
  threadId?: string; model?: string; effort?: string; prompt: string; result: string;
  startedAt?: number; durationMs?: number;
  steps?: Message[]; stepsTruncated?: boolean;
  visual?: { shape:number; color:number };
}
const record = (value: unknown): Record<string, any> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, any> : {};
const text = (...values: unknown[]) => values.find(value => typeof value === 'string' && value.trim()) as string | undefined;
const finite = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? value : undefined;
function childReport(value: string) {
  // OpenCode frames Task results in an envelope. Match that exact native shape
  // so ordinary report HTML and code remain intact, and never alter storage.
  const task = /^<task id="[^"\r\n]+" state="(?:completed|failed|cancelled)">\r?\n<task_result>\r?\n([\s\S]*?)\r?\n<\/task_result>\r?\n<\/task>\s*$/.exec(value);
  if (task) return task[1];
  // New Claude SDK versions frame returned child reports for the parent model.
  // Keep that original record intact; the child detail shows the report itself.
  const prefix = '[Subagent hand-back] The text below is the final report of a subagent this session delegated to.';
  const marker = 'The report follows:\n';
  const start = value.startsWith(prefix) ? value.indexOf(marker, prefix.length) : -1;
  if (start < 0) return value;
  const report = value.slice(start + marker.length);
  return report.split('\n').every(line => !line || line.startsWith('  '))
    ? report.replace(/^ {2}/gm, '') : report;
}
function state(value: unknown, fallback: SubagentStatus): SubagentStatus {
  if (['completed', 'success', 'done'].includes(String(value))) return 'completed';
  if (['failed', 'errored', 'error', 'notFound'].includes(String(value))) return 'failed';
  if (['interrupted', 'shutdown', 'stopped', 'cancelled'].includes(String(value))) return 'interrupted';
  if (['waiting', 'waitingForInput'].includes(String(value))) return 'waiting';
  if (['running', 'inProgress', 'preparing', 'pendingInit'].includes(String(value))) return 'running';
  return fallback;
}
export function isSubagent(message: Message) {
  const args = record(message.data?.arguments);
  return message.kind === 'collabAgentToolCall' || message.kind === 'collabToolCall'
    || message.kind === 'claudeToolCall' && (['Task', 'Agent', 'task', 'subagent'].includes(String(message.data?.tool))
      || typeof args.subagent_type==='string' && typeof args.prompt==='string');
}

// Native collaboration tool completion is not the child's completion. Use the
// child's state; spawn/send/wait records refer to the same independently running agent.
export function collectSubagents(messages: Message[]): Subagent[] {
  const agents = new Map<string, Subagent>();
  const settledTurns = new Set(messages.filter(message => message.kind==='runMarker'
    && ['completed','failed','interrupted'].includes(String(message.data?.status))).map(message => message.data?.turnId));
  for (const message of messages.filter(isSubagent)) {
    const d = record(message.data), args = record(d.arguments), states = record(d.agentsStates);
    const codex = message.kind.startsWith('collab');
    const receivers: string[] = codex ? [...new Set([
      ...(Array.isArray(d.receiverThreadIds) ? d.receiverThreadIds.filter((id: unknown) => typeof id === 'string') : []),
      ...Object.keys(states),
    ])] : [];
    const keys = codex ? receivers : [message.id];
    for (const key of keys) {
      const previous = agents.get(key), child = record(states[key]);
      const fallback = previous?.status ?? 'running';
      const asyncLaunch = !codex && (d.subagentAsync === true || message.text.startsWith('Async agent launched successfully.'));
      const nativeStatus = codex ? child.status : d.subagentStatus ?? (d.status==='failed' && message.text.trim()==='Task cancelled' ? 'interrupted'
        : (asyncLaunch || args.run_in_background) && d.status === 'completed' ? 'running' : d.status);
      const result = !codex && typeof d.subagentResult === 'string' ? d.subagentResult
        : text(child.message, child.result, !codex && !asyncLaunch && d.status !== 'preparing' ? message.text : '', previous?.result) ?? '';
      agents.set(key, {
        key, anchorId:previous?.anchorId ?? message.id,
        name:text(child.agentNickname, child.nickname, child.name, d.agentNickname, d.agentName, args.name, args.description, args.subagent_type, args.agent, previous?.name) ?? '子代理',
        status:!codex && asyncLaunch && !d.subagentStatus && !d.subagentResult && typeof d.turnId==='string' && settledTurns.has(d.turnId)
          ? 'unknown' : state(d.error || d.success === false ? 'failed' : nativeStatus, fallback),
        threadId:codex ? key : undefined,
        model:text(child.model, d.model, args.model, previous?.model),
        effort:text(child.reasoningEffort, d.reasoningEffort, args.reasoning_effort, previous?.effort),
        prompt:text(d.prompt, args.prompt, args.task, previous?.prompt) ?? '',
        result:codex ? result : childReport(result),
        startedAt:previous?.startedAt ?? finite(d.subagentStartedAt) ?? finite(d.startedAt),
        durationMs:finite(codex ? child.durationMs : asyncLaunch ? d.subagentDurationMs : d.durationMs) ?? previous?.durationMs,
        steps:Array.isArray(d.subagentSteps) ? d.subagentSteps.filter((step:unknown)=>{
          const item=record(step);return typeof item.id==='string'&&['reasoning','claudeToolCall','executionPlan'].includes(item.type);
        }).slice(-200).reduce((steps:Message[],item:Record<string,any>)=>mergeEvent(steps,{method:'item/updated',params:{item}},message.sessionId),[]) : previous?.steps,
        stepsTruncated:d.subagentStepsTruncated===true || previous?.stepsTruncated,
      });
    }
  }
  const rows = [...agents.values()], visuals = subagentVisuals(rows);
  return rows.map(agent => ({ ...agent, visual:visuals.get(agent.key) }));
}
export const subagentLabel = (agent: Subagent) => /^Agent\b/i.test(agent.name) ? agent.name : `Agent ${agent.name}`;
export const subagentStateText: Record<SubagentStatus, string> = { running:'开始工作', waiting:'等待确认', completed:'已完成', failed:'运行失败', interrupted:'已停止', unknown:'完成状态未返回' };
export function subagentEventStatus(event:RpcEvent):SubagentStatus|undefined {
  if(event.id!==undefined)return 'waiting';
  if(event.method==='turn/started'||event.method==='serverRequest/resolved')return 'running';
  if(event.method==='turn/completed')return state(event.params.turn?.status,'failed');
  return undefined;
}
export const subagentStatus = (value:unknown) => state(value,'running');

export function subagentVisual(key: string) {
  let hash = 2166136261;
  for (const character of key) hash = Math.imul(hash ^ character.charCodeAt(0), 16777619);
  return { shape:(hash >>> 0) % 6, color:(hash >>> 8) % 6 };
}
export function subagentVisuals(agents: Pick<Subagent, 'key'>[]) {
  const visuals = new Map<string, ReturnType<typeof subagentVisual>>(), used = new Set<number>();
  for (const { key } of agents) {
    if (visuals.has(key)) continue;
    const preferred = subagentVisual(key);
    let slot = preferred.color * 6 + preferred.shape;
    for (let attempt = 0; attempt < 36 && used.has(slot); attempt++) slot = (slot + 1) % 36;
    visuals.set(key, { shape:slot % 6, color:Math.floor(slot / 6) });
    used.add(slot);
  }
  return visuals;
}
