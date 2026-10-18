import type { Message } from './types';

export type ActivityCategory = 'command' | 'edit' | 'read' | 'search' | 'web' | 'browser' | 'computer' | 'plan' | 'reasoning' | 'context' | 'tool';
interface ActivitySummary { label: string; category: ActivityCategory; status: string; earlierFailure?: true }

export function activityStatus(message: Message) {
  const data = message.data;
  if (data?.error || data?.success === false || Number(data?.exitCode ?? 0) !== 0) return 'failed';
  return String(data?.status ?? 'completed');
}

const activeStatuses = new Set(['inProgress', 'running', 'preparing']);
const activityLabels: Record<ActivityCategory, [string, string]> = {
  command: ['运行了命令', '正在执行命令'],
  edit: ['编辑了文件', '正在编辑文件'],
  read: ['读取了文件', '正在读取文件'],
  search: ['搜索了代码', '正在搜索代码'],
  web: ['搜索了网页', '正在搜索网页'],
  browser: ['使用了浏览器', '正在使用浏览器'],
  computer: ['使用了电脑', '正在操作电脑'],
  plan: ['更新了计划', '正在更新计划'],
  reasoning: ['思考摘要', '正在思考'],
  context: ['整理了上下文', '正在整理上下文'],
  tool: ['调用了工具', '正在调用工具'],
};

function currentActivity(message: Message): ActivitySummary {
  const summary = summarizeActivities([message]);
  const data = message.data ?? {};
  const tool = String(data.tool ?? '');
  let label = summary.label;
  if (message.kind === 'claudeToolCall' && tool === 'Write') label = '正在写文件';
  else if (message.kind === 'claudeToolCall' && tool === 'Glob') label = '正在查找文件';
  else if (message.kind === 'imageView') label = '正在查看图片';
  else if (message.kind === 'fileChange' && Array.isArray(data.changes) && data.changes.length) {
    const kinds = data.changes.map(change => typeof change?.kind === 'string' ? change.kind : change?.kind?.type);
    if (kinds.every(kind => ['add', 'create', 'added'].includes(kind))) label = '正在写文件';
    else if (kinds.every(kind => ['delete', 'remove', 'deleted'].includes(kind))) label = '正在删除文件';
  }
  if (summary.status === 'preparing') label = tool === 'ExitPlanMode' ? '正在生成计划' : label.replace(/^正在/, '正在准备');
  return { ...summary, label };
}

/** Summarize actual event metadata without inferring actions from arbitrary shell code. */
export function summarizeActivities(messages: Message[], live = false): ActivitySummary {
  if (live) {
    // An executing tool takes priority over reasoning that an engine has not
    // closed yet. Completed tools and old thought/plan rows are history.
    let latest: Message | undefined;
    for (let index = messages.length - 1; index >= 0; index--) {
      const message = messages[index];
      if (message.kind === 'runMarker' || message.role === 'user' || message.role === 'system') continue;
      latest ??= message;
      if (activeStatuses.has(activityStatus(message)) && message.role !== 'assistant'
        && !['reasoning', 'executionPlan'].includes(message.kind)) return currentActivity(message);
    }
    if (latest && activeStatuses.has(activityStatus(latest))) {
      if (latest.kind === 'reasoning' || latest.kind === 'executionPlan') return currentActivity(latest);
      if (latest.kind === 'agentMessage' || latest.kind === 'plan') return {
        label: latest.kind === 'plan' ? '正在生成计划' : '正在回复', category: 'tool' as ActivityCategory, status: activityStatus(latest),
      };
    }
    // A running turn alone does not reveal whether the model is thinking.
    return { label: '正在处理', category: 'tool' as ActivityCategory, status: 'inProgress' };
  }
  const categories = new Map<ActivityCategory, { active: boolean; paths: Set<string> }>();
  let status = 'completed';
  let latestActionState: string | undefined;
  let earlierFailure = false;
  const rank: Record<string, number> = { completed: 0, inProgress: 1, running: 1, preparing: 1, interrupted: 2, declined: 3, failed: 4 };
  for (const message of messages) {
    const data = message.data ?? {};
    const state = activityStatus(message);
    if ((rank[state] ?? 0) > (rank[status] ?? 0)) status = state;
    const add = (category: ActivityCategory, paths: unknown[] = []) => {
      // Historical failures stay on their own rows. A later real tool result
      // determines the summary; reasoning/plan text cannot manufacture success.
      if (!['reasoning', 'plan', 'context'].includes(category)) {
        latestActionState = state;
        earlierFailure ||= state === 'failed';
      }
      const value = categories.get(category) ?? { active: false, paths: new Set<string>() };
      value.active ||= activeStatuses.has(state);
      for (const path of paths) if (typeof path === 'string' && path) {
        const normalized = path.replace(/\\/g, '/');
        value.paths.add(/^[a-z]:\//i.test(normalized) ? normalized.toLowerCase() : normalized);
      }
      categories.set(category, value);
    };
    const tool = String(data.tool ?? '');
    const args = (data.arguments ?? {}) as Record<string, unknown>;
    if (message.kind === 'reasoning') add('reasoning');
    else if (message.kind === 'executionPlan' || message.kind === 'claudeToolCall' && ['EnterPlanMode', 'ExitPlanMode'].includes(tool)) add('plan');
    else if (message.kind === 'contextCompaction') add('context');
    else if (message.kind === 'fileChange') {
      const paths = Array.isArray(data.changes) ? data.changes.map(change => (change as { path?: unknown }).path) : message.text.split('\n');
      add('edit', paths);
    } else if (message.kind === 'commandExecution') {
      const actions = Array.isArray(data.commandActions) ? data.commandActions as { type?: string }[] : [];
      if (!actions.length) add('command');
      else for (const action of actions) add(action.type === 'read' || action.type === 'listFiles' ? 'read' : action.type === 'search' ? 'search' : 'command');
    } else if (message.kind === 'claudeToolCall' && ['Edit', 'Write', 'MultiEdit', 'NotebookEdit'].includes(tool)) add('edit', [args.file_path ?? args.notebook_path]);
    else if (message.kind === 'claudeToolCall' && tool === 'Bash') add('command');
    else if (message.kind === 'imageView' || (message.kind === 'claudeToolCall' && ['Read', 'Glob'].includes(tool))) add('read');
    else if (message.kind === 'claudeToolCall' && tool === 'Grep') add('search');
    else if (message.kind === 'webSearch' || (message.kind === 'claudeToolCall' && ['WebSearch', 'WebFetch'].includes(tool))) add('web');
    else {
      const identity = [data.server, data.namespace, tool].filter(Boolean).join(' ').toLowerCase();
      if (/(?:^|[\s_\/-])(browser|playwright|puppeteer|selenium)(?:$|[\s_\/-])/.test(identity)) add('browser');
      else if (/(?:^|[\s_\/-])(computer|desktop|sky)(?:$|[\s_\/-])/.test(identity)) add('computer');
      else add('tool');
    }
  }
  if (categories.size > 1) categories.delete('reasoning');
  if (latestActionState !== undefined && !activeStatuses.has(latestActionState)) status = latestActionState;
  const entries = [...categories];
  const label = entries.map(([category, value]) => {
    if (category === 'edit' && value.paths.size > 1) return value.active ? '正在编辑多个文件' : '编辑了多个文件';
    return activityLabels[category][value.active ? 1 : 0];
  }).join('、');
  return { label, category: entries[0]?.[0] ?? 'tool', status,
    ...(earlierFailure && status === 'completed' ? { earlierFailure: true as const } : {}) };
}

export function compactActivities(messages: Message[]) {
  if (messages.length <= 3) return messages;
  const visible = new Set([messages.length - 2, messages.length - 1]);
  for (let i = messages.length - 3; i >= 0; i--) {
    if (['failed', 'declined', 'interrupted'].includes(activityStatus(messages[i]))) {
      visible.add(i);
      break;
    }
  }
  if (visible.size < 3) visible.add(messages.length - 3);
  return messages.filter((_, index) => visible.has(index));
}

export function concisePath(path: string) {
  return path.replace(/\\/g, '/').split('/').filter(Boolean).slice(-2).join('/') || path;
}
