import type { Message } from './types';

export type ActivityCategory = 'command' | 'edit' | 'read' | 'search' | 'web' | 'browser' | 'computer' | 'plan' | 'reasoning' | 'context' | 'tool';

export function activityStatus(message: Message) {
  const data = message.data;
  if (data?.error || data?.success === false || Number(data?.exitCode ?? 0) !== 0) return 'failed';
  return String(data?.status ?? 'completed');
}

const activeStatuses = new Set(['inProgress', 'running', 'preparing']);
const activityLabels: Record<ActivityCategory, [string, string]> = {
  command: ['运行了命令', '正在运行命令'],
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

/** Summarize actual event metadata without inferring actions from arbitrary shell code. */
export function summarizeActivities(messages: Message[], live = false) {
  if (live) {
    // During a turn, show the newest ongoing action instead of completed history.
    // A plan stays in progress across tools; it is current only when it is latest.
    let current: Message | undefined;
    for (let index = messages.length - 1; index >= 0; index--) {
      const message = messages[index];
      if (activeStatuses.has(activityStatus(message)) && (message.kind !== 'executionPlan' || index === messages.length - 1)) { current = message; break; }
    }
    if (current) return summarizeActivities([current]);
    // Tool completion is followed by the model's next step, often before any
    // public reasoning fragment arrives. Keep the old tool result in the details.
    return { label: '正在思考', category: 'reasoning' as ActivityCategory, status: 'inProgress' };
  }
  const categories = new Map<ActivityCategory, { active: boolean; paths: Set<string> }>();
  let status = 'completed';
  const rank: Record<string, number> = { completed: 0, inProgress: 1, running: 1, preparing: 1, interrupted: 2, declined: 3, failed: 4 };
  for (const message of messages) {
    const data = message.data ?? {};
    const state = activityStatus(message);
    if ((rank[state] ?? 0) > (rank[status] ?? 0)) status = state;
    const add = (category: ActivityCategory, paths: unknown[] = []) => {
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
    else if (message.kind === 'executionPlan') add('plan');
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
  const entries = [...categories];
  const label = entries.map(([category, value]) => {
    if (category === 'edit' && value.paths.size > 1) return value.active ? '正在编辑多个文件' : '编辑了多个文件';
    return activityLabels[category][value.active ? 1 : 0];
  }).join('、');
  return { label, category: entries[0]?.[0] ?? 'tool', status };
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
